---
name: sentrux-verification
description: Verify a sentrux PR by running cargo fmt/clippy/test/doc and a quick CLI scan on a fixture repo.
---

# Sentrux Verification

Use this skill when asked to verify a `sentrux` PR or run the golden-path checks.

## Devin Secrets Needed
- None.

## Pre-conditions
- The repo is a Rust workspace (`sentrux-core` + `sentrux-bin`) under `/home/ubuntu/repos/sentrux`.
- Stable Rust >= workspace `rust-version` (currently 1.91.0) is installed.
- Language grammar plugins are pre-installed under `~/.sentrux/plugins/`.
- Network should be avoided during verification; set `SENTRUX_SKIP_GRAMMAR_DOWNLOAD=1` before running the CLI.

## Golden-path commands
```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
cargo build --bin sentrux
```

## CLI scan fixture
Create a tiny Python project to exercise the Tarjan SCC / snapshot pipeline:
```bash
rm -rf /tmp/sentrux_fixture
mkdir -p /tmp/sentrux_fixture/pkg
cd /tmp/sentrux_fixture
git init -q
cat > pyproject.toml <<'EOF'
[project]
name = "fixture"
version = "0.1.0"
EOF
cat > pkg/__init__.py <<'EOF'
"""pkg package"""
EOF
cat > pkg/a.py <<'EOF'
import pkg.b
EOF
cat > pkg/b.py <<'EOF'
import pkg.a
EOF
git add . && git -c user.name="Tester" -c user.email="test@example.com" commit -q -m "init"
```

Run the scan without hitting the network:
```bash
SENTRUX_SKIP_GRAMMAR_DOWNLOAD=1 /home/ubuntu/repos/sentrux/target/debug/sentrux gate --save /tmp/sentrux_fixture
SENTRUX_SKIP_GRAMMAR_DOWNLOAD=1 /home/ubuntu/repos/sentrux/target/debug/sentrux gate /tmp/sentrux_fixture
```

A working run prints a `Quality:` score, shows `Cycles: N → N` and `✓ No degradation detected`.

To explicitly confirm the cycle is detected, add a rule file and run `sentrux check`:
```bash
mkdir -p /tmp/sentrux_fixture/.sentrux
cat > /tmp/sentrux_fixture/.sentrux/rules.toml <<'EOF'
[constraints]
max_cycles = 0
EOF
SENTRUX_SKIP_GRAMMAR_DOWNLOAD=1 /home/ubuntu/repos/sentrux/target/debug/sentrux check /tmp/sentrux_fixture
```

## Common notes
- `cargo doc` may emit a build-script notice (`Generated embedded.rs with 52 plugins`); that is a build-script message, not a rustdoc warning. Use `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` to enforce no rustdoc warnings.
- The `sentrux` binary attempts to download grammar tarballs on first run; `SENTRUX_SKIP_GRAMMAR_DOWNLOAD=1` disables that so CI/verify runs remain offline.
- Tests that fail without grammar plugins will report grammar loading errors in `sentrux-core` tests; ensure plugins are pre-installed or use the CI environment.
