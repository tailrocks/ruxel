use super::{Lifecycle, lifecycle};
#[derive(Default)]
struct Synthetic {
    calls: Vec<String>,
    state: bool,
    mutate: bool,
    fail: Option<&'static str>,
}
impl Synthetic {
    fn call(&mut self, name: &str) -> anyhow::Result<()> {
        self.calls.push(name.to_owned());
        anyhow::ensure!(self.fail != Some(name), "synthetic failure");
        Ok(())
    }
}
impl Lifecycle for Synthetic {
    type Snapshot = bool;
    fn apply(&mut self) -> anyhow::Result<()> {
        self.call("apply")?;
        self.state = true;
        Ok(())
    }
    fn observe(&mut self) -> anyhow::Result<()> {
        self.call("observe")?;
        anyhow::ensure!(self.state);
        Ok(())
    }
    fn drift(&mut self) -> anyhow::Result<()> {
        self.call("drift")?;
        self.state = false;
        Ok(())
    }
    fn snapshot(&mut self) -> anyhow::Result<bool> {
        Ok(self.state)
    }
    fn preview(&mut self, mode: &str) -> anyhow::Result<()> {
        self.call(mode)?;
        if self.mutate {
            self.state = true;
        }
        Ok(())
    }
}
#[test]
fn lifecycle_orders_phases_and_fails_closed() -> anyhow::Result<()> {
    let mut run = Synthetic::default();
    lifecycle(&mut run)?;
    assert_eq!(
        run.calls,
        [
            "apply", "observe", "apply", "observe", "drift", "check", "diff", "apply", "observe"
        ]
    );
    assert!(
        lifecycle(&mut Synthetic {
            mutate: true,
            ..Default::default()
        })
        .is_err()
    );
    for step in ["apply", "observe", "drift", "check", "diff"] {
        assert!(
            lifecycle(&mut Synthetic {
                fail: Some(step),
                ..Default::default()
            })
            .is_err()
        );
    }
    Ok(())
}
