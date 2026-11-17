# Agent Instructions

Solmu is a small native WhatsApp client (Rust). Before starting work, read
`CONTRIBUTING.md` (product boundaries and conventions), `ROADMAP.md`
(direction) and `WORK-LOG.md` / `CHECKPOINT.md` (what other agents have done
and are doing). Several agents work on this repo in parallel; record your own
work in `WORK-LOG.md` so nobody duplicates or conflicts with it.

## Repo facts (verified 2026-11-17)

Checks (the same ones CI runs, from CONTRIBUTING.md):

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked --all-features --no-deps
```

- Linux GUI build dependencies:
  `sudo python3 .github/scripts/install-ubuntu-build-deps.py libxkbcommon-dev libwayland-dev libgl1-mesa-dev libasound2-dev libpipewire-0.3-dev clang libclang-dev cmake perl`.
  `nix develop` provides the complete environment.
- Headless UI captures use synthetic demo content only:
  `cargo run --features demo -- --demo` (`--demo-shot` for a headless capture).
  A PR that changes how the app looks needs before/after screenshots from demo
  content in its description; real conversation content never goes in a PR.
- Leave the translation template and catalogs (`solmu.pot`, PO files) out of PRs
  that add or change UI strings; the maintainer regenerates them on main.

CI quirks (`.github/workflows/ci.yml`; details in `CI-GUIDE.md`):

- `.github/scripts/ci-scope.py` selects checks by changed paths;
  documentation-only changes skip native compilation.
- On PRs the macOS test leg is the merge gate: it starts right after the
  portable checks, and quality, Ubuntu tests and the Nix package report later,
  so a PR can look gated-green while those still run. Full platform matrices
  (Windows, Debian 12, all three Nix systems) run on version tags, merge
  queues and manual scope=full.
- CI wraps cargo with `mbx` (mr-boxington) on runners; plain `cargo` works
  locally.

Test failures observed on unrelated diffs:

- `demo::tests::linked_messages_style_uses_one_clock_per_run` (`src/demo.rs`) -
  asserts the message-bubble clock layout (footer widths: only the last bubble
  of a same-sender run keeps its clock). On 2026-11-17 it failed on the Ubuntu
  leg of the unrelated PR #363 (`footer(0)` was 31.5, expected 0.0) while macOS
  passed; the cause is not established, so do not assume it is flaky. If it
  fails on your PR, check whether main fails the same way and report that
  evidence instead of patching around it.

Merge gate: a PR merges only when Devin review shows no open findings on the
current head and the required CI checks are green (owner's standing rule). If
Devin has not reviewed the current head, wait for it instead of merging on CI
alone.

Loop guard: if the same check fails twice with the same error, stop. Report
the command, the error and what you tried instead of retrying a third time or
patching blind.
