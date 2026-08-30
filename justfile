check:
    mise exec -- cargo fmt --all --check
    mise exec -- cargo clippy --all-targets -- -D warnings
    mise exec -- cargo nextest run
    mise exec -- cargo machete
    python3 -m unittest discover -s tools/oracle -p 'test*.py'
    python3 -m unittest discover -s tools/benchmarks -p 'test*.py'
    python3 -m unittest discover -s tools/chaos -p 'test*.py'
    python3 -m unittest discover -s tools/spec-extract -p 'test*.py'
    python3 tools/oracle/verify_captures.py
    python3 tools/benchmarks/verify.py docs/benchmarks/results
    python3 tools/chaos/verify.py
    tools/chaos/test_safety.sh

agent:
    mise exec -- cargo zigbuild --target x86_64-unknown-linux-musl -p ruxel-agent --release
