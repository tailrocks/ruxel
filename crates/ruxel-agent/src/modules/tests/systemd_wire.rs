use super::*;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
#[path = "systemd_wire/behavior.rs"]
mod behavior;

#[test]
fn canonical_systemd_wire_preserves_service_control_and_cache()
-> Result<(), Box<dyn std::error::Error>> {
    const CHILD: &str = "RUXEL_SYSTEMD_WIRE_CHILD";
    if let Some(root) = std::env::var_os(CHILD) {
        return behavior::verify(Path::new(&root));
    }
    let root = std::env::temp_dir().join(format!("ruxel-systemd-wire-{}", std::process::id()));
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("actions"), "")?;
    let script = stub();
    let tool = root.join("systemctl");
    std::fs::write(&tool, script)?;
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700))?;
    let output = std::process::Command::new(std::env::current_exe()?)
        .args([
            "canonical_systemd_wire_preserves_service_control_and_cache",
            "--nocapture",
        ])
        .env(CHILD, &root)
        // The stub reads the unit types of a call with `tr`, so the child
        // inherits the PATH of the test beside the stub's own directory:
        // the stub comes first, so `systemctl` is the stub, and `tr` is
        // the one of the host, so the stub validates the types instead of
        // skipping the validation.
        .env("PATH", format!("{}:{}", root.display(), inherited_path()))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    verify_types(&tool)?;
    verify_discovery(&root)?;
    std::fs::remove_dir_all(root)?;
    Ok(())
}

/// The stub of `systemctl` the child runs against.
fn stub() -> &'static str {
    r#"#!/bin/sh
# The stub answers the units of the types it is asked for, and refuses a
# type it does not know: a discovery that asked for the services only
# would leave the timer units unknown.
#
# The types are read with `tr`, so it has to be on PATH: without it the
# loop below reads no type at all, and the validation of the types would
# silently do nothing.
command -v tr >/dev/null 2>&1 || { printf 'the stub needs tr on PATH\n' >&2; exit 9; }
types=""
for arg in "$@"; do
  case "$arg" in
  --type=*) types="${arg#--type=}" ;;
  esac
done
requested=$(printf '%s' "$types" | tr ',' ' ')
for type in $requested; do
  case "$type" in
  service|timer) ;;
  *) printf 'unknown unit type: %s\n' "$type" >&2; exit 9 ;;
  esac
done
# Every call is recorded with the types it was validated against, so that
# a test can see what the agent asked for.
if [ -n "${RUXEL_SYSTEMD_WIRE_CHILD:-}" ]; then
  printf '%s\t%s\n' "$*" "$requested" >> "$RUXEL_SYSTEMD_WIRE_CHILD/calls"
fi
wanted() {
  case ",$types," in
  ,,) return 0 ;;
  *,"$1",*) return 0 ;;
  *) return 1 ;;
  esac
}
case "$1" in
list-units)
  [ -n "$types" ] || { printf 'no unit type requested\n' >&2; exit 9; }
  if wanted service; then if [ -f "$RUXEL_SYSTEMD_WIRE_CHILD/active" ]; then printf 'demo.service loaded active running Demo\n'; else printf 'demo.service loaded inactive dead Demo\n'; fi; fi
  if wanted timer; then if [ -f "$RUXEL_SYSTEMD_WIRE_CHILD/timer-active" ]; then printf 'demo.timer loaded active running Demo timer\n'; else printf 'demo.timer loaded inactive dead Demo timer\n'; fi; fi
  ;;
list-unit-files)
  [ -n "$types" ] || { printf 'no unit type requested\n' >&2; exit 9; }
  if wanted service; then if [ -f "$RUXEL_SYSTEMD_WIRE_CHILD/enabled" ]; then printf 'demo.service enabled enabled\n'; else printf 'demo.service disabled enabled\n'; fi; fi
  if wanted timer; then if [ -f "$RUXEL_SYSTEMD_WIRE_CHILD/timer-enabled" ]; then printf 'demo.timer enabled enabled\n'; else printf 'demo.timer disabled enabled\n'; fi; fi
  ;;
*) printf '%s\n' "$*" >> "$RUXEL_SYSTEMD_WIRE_CHILD/actions"
case "$1" in
start|restart) : > "$RUXEL_SYSTEMD_WIRE_CHILD/active";;
stop) if [ "$2" = "demo.timer" ]; then /bin/rm -f "$RUXEL_SYSTEMD_WIRE_CHILD/timer-active"; else /bin/rm -f "$RUXEL_SYSTEMD_WIRE_CHILD/active"; fi;;
reload) :;;
enable) if [ "$2" = "demo.timer" ]; then : > "$RUXEL_SYSTEMD_WIRE_CHILD/timer-enabled"; else : > "$RUXEL_SYSTEMD_WIRE_CHILD/enabled"; fi;;
disable) if [ "$2" = "demo.timer" ]; then /bin/rm -f "$RUXEL_SYSTEMD_WIRE_CHILD/timer-enabled"; else /bin/rm -f "$RUXEL_SYSTEMD_WIRE_CHILD/enabled"; fi;;
daemon-reload) [ "$#" = 1 ] || exit 9;;
*) exit 8;;
esac;;
esac
"#
}

/// The PATH of the test, which the child inherits beside the stub's own
/// directory.
fn inherited_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// A call the stub recorded: the arguments it was given, and the unit
/// types it read out of them.
///
/// The types are read with `tr`, so they are empty when the validation
/// did not run: the stub reads no type at all without it, and a
/// discovery that asked for the services only would leave the timer
/// types out.
#[derive(Debug)]
struct Call {
    arguments: String,
    types: Vec<String>,
}

/// Every call the stub recorded, in the order it was called.
fn calls(root: &Path) -> Result<Vec<Call>, Box<dyn std::error::Error>> {
    let recorded = std::fs::read_to_string(root.join("calls"))?;
    Ok(recorded
        .lines()
        .map(|line| {
            let (arguments, types) = line.split_once('\t').unwrap_or((line, ""));
            Call {
                arguments: arguments.to_owned(),
                types: types.split_whitespace().map(str::to_owned).collect(),
            }
        })
        .collect())
}

/// The agent discovers the timer units: it asks `systemctl` for the
/// services and the timers together, and the stub reads both types out
/// of the call. A discovery that asked for the services only, or a call
/// whose types the stub could not read, fails this test.
fn verify_discovery(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let calls = calls(root)?;
    for (command, types) in [
        ("list-units ", vec!["service", "timer"]),
        ("list-unit-files ", vec!["service", "timer"]),
    ] {
        let discovered: Vec<&Call> = calls
            .iter()
            .filter(|call| call.arguments.starts_with(command))
            .collect();
        assert!(
            !discovered.is_empty(),
            "the agent never asked for {command:?}: {calls:?}"
        );
        for call in discovered {
            for kind in &types {
                assert!(
                    call.types.iter().any(|read| read == kind),
                    "{:?} was asked without the {kind:?} type: {:?}",
                    call.arguments,
                    call.types
                );
            }
        }
    }
    Ok(())
}

/// The stub answers the units of the types it is asked for, and refuses a
/// type it does not know: a discovery that asked for the services only
/// would leave the timer units unknown, and the timer test would not see
/// it.
fn verify_types(tool: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let listed = |args: &[&str]| -> Result<String, Box<dyn std::error::Error>> {
        let output = std::process::Command::new(tool).args(args).output()?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let services = listed(&[
        "list-units",
        "--all",
        "--type=service",
        "--no-legend",
        "--plain",
    ])?;
    assert!(services.contains("demo.service"), "{services}");
    assert!(!services.contains("demo.timer"), "{services}");
    let timers = listed(&[
        "list-units",
        "--all",
        "--type=timer",
        "--no-legend",
        "--plain",
    ])?;
    assert!(timers.contains("demo.timer"), "{timers}");
    assert!(!timers.contains("demo.service"), "{timers}");
    let files = listed(&[
        "list-unit-files",
        "--type=service,timer",
        "--no-legend",
        "--plain",
    ])?;
    assert!(files.contains("demo.service"), "{files}");
    assert!(files.contains("demo.timer"), "{files}");
    // A type the stub does not know is refused, not answered.
    let unknown = std::process::Command::new(tool)
        .args([
            "list-units",
            "--all",
            "--type=socket",
            "--no-legend",
            "--plain",
        ])
        .output()?;
    assert!(
        !unknown.status.success(),
        "the stub answered a type it does not know"
    );
    Ok(())
}
