# XFS synthetic acceptance

`cargo test --locked --manifest-path tools/acceptance-xfs/Cargo.toml` runs the
agent's production XFS module with synthetic geometry and ioctl callbacks,
and checks descriptor handoff. On Linux it also checks an actual geometry
ioctl failure against procfs. These tests do not access a block device or prove
mounted-XFS behavior. Perl with its core modules is required.
