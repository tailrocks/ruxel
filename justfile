check:
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings
    cargo nextest run
    mise exec -- cargo machete

fmt:
    cargo fmt --all

agent:
    mise exec -- cargo zigbuild --target x86_64-unknown-linux-musl -p ruxel-agent --release
