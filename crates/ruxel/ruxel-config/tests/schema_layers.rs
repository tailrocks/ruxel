//! Pins the layer modules' local copies of Core's typealiases.

use std::path::Path;

/// Every layer module copies the Core typealiases (and the functions
/// `HttpsUrl` needs) that its fields use, because pklr drops the constraints
/// of a type reached through a dotted import such as `core.Static`. This
/// pins each copy to Core's text, so none can drift.
#[test]
fn layer_typealias_copies_match_core() {
    /// The text of a module member: a `typealias`'s declaration line, or a
    /// function's lines down to the first blank line.
    fn member(module: &str, name: &str) -> String {
        let lines: Vec<&str> = module.lines().collect();
        let start = lines
            .iter()
            .position(|line| {
                line.starts_with(&format!("typealias {name} = "))
                    || line.starts_with(&format!("const local function {name}("))
            })
            .unwrap_or_else(|| panic!("no member {name}"));
        let end = lines
            .iter()
            .skip(start)
            .position(|line| line.is_empty())
            .map_or(lines.len(), |blank| start + blank);
        lines[start..end].join("\n")
    }

    let schema = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../schema/ruxel"
    ));
    let read = |name: &str| {
        std::fs::read_to_string(schema.join(format!("{name}.pkl"))).expect("committed module")
    };
    let core = read("Core");
    for (layer, copies) in [
        (
            "Linux",
            [
                "Static",
                "Name",
                "AbsolutePath",
                "FilePath",
                "Account",
                "CommitSha",
                "Presence",
                "HttpsUrl",
                "isHttpsUrl",
                "isAuthority",
                "isPort",
                "isIpv6Address",
                "isIpv4",
            ]
            .as_slice(),
        ),
        ("Systemd", ["Static", "Name"].as_slice()),
        (
            "Debian",
            [
                "Static",
                "StaticList",
                "HttpsUrl",
                "isHttpsUrl",
                "isAuthority",
                "isPort",
                "isIpv6Address",
                "isIpv4",
            ]
            .as_slice(),
        ),
    ] {
        let module = read(layer);
        for name in copies {
            assert_eq!(
                member(&module, name),
                member(&core, name),
                "{layer}.pkl: {name} must stay a copy of Core's"
            );
        }
    }
}
