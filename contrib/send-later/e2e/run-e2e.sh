#!/bin/bash
# End-to-end Send Later scheduler tests: builds the REAL daemon binary from
# this repo's sources (in a throwaway harness crate, so no app-wide build is
# needed) and drives it against a stub Beeper Server API. No network, no
# credentials, no real chats: the stub stands in for 127.0.0.1:23373.
#
# Usage: contrib/send-later/e2e/run-e2e.sh
# Requires: rustup with the 1.98.0 toolchain (the repo pin), python3, curl.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
HARNESS=${SL_E2E_HARNESS:-/tmp/scheduler-harness}
export SL_E2E_WORK=${SL_E2E_WORK:-/tmp/sltest}
export SL_E2E_STUB_PORT=${SL_E2E_STUB_PORT:-23901}
export SL_E2E_DAEMON_PORT=${SL_E2E_DAEMON_PORT:-23902}

echo "== building harness crate from repo sources in $HARNESS =="
mkdir -p "$HARNESS/src/bin"
cat > "$HARNESS/Cargo.toml" <<'TOML'
[package]
name = "harness"
version = "0.0.0"
edition = "2024"

[lib]
name = "harness"
path = "src/lib.rs"

[[bin]]
name = "solmu-scheduler"
path = "src/bin/solmu-scheduler.rs"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.12", default-features = false, features = ["blocking", "rustls-tls"] }
jiff = "0.2"

[profile.release]
debug = false
TOML
cat > "$HARNESS/src/lib.rs" <<'RS'
//! Harness compiling the real Solmu scheduler modules standalone.
pub mod scheduler {
    include!("scheduler.inc");
}
pub mod recurrence {
    include!("recurrence.inc");
}
pub mod scheduler_api {
    include!("scheduler_api.inc");
}
RS
sed 's|^//!|//|' "$REPO/src/scheduler.rs" > "$HARNESS/src/scheduler.inc"
sed 's|^//!|//|' "$REPO/src/recurrence.rs" > "$HARNESS/src/recurrence.inc"
sed 's|^//!|//|' "$REPO/src/scheduler_api.rs" > "$HARNESS/src/scheduler_api.inc"
sed 's/solmu::/harness::/g' "$REPO/src/bin/solmu-scheduler.rs" > "$HARNESS/src/bin/solmu-scheduler.rs"
(cd "$HARNESS" && cargo +1.98.0 build --release --bin solmu-scheduler)
export SL_E2E_BIN="$HARNESS/target/release/solmu-scheduler"

echo "== running cases =="
"$HERE/cases.sh"

python3 "$HERE/closed_client.py" --binary "$SL_E2E_BIN"
