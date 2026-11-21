# Report-only security pilot

Owner-requested free-only pilot. No required checks, fixes, app installation,
repository secrets, cache uploads, or artifact uploads are added. Jobs refuse to
run if the repository becomes private. Only standard Ubuntu hosted runners run
PR code. Weekly schedules use UTC and are staggered across the four pilot repos.
The pilot PR and later scanner edits run once for validation; ordinary code PRs
are not scanned until the owner decides whether to widen the trigger.

The JSON job log and job summary distinguish FINDINGS, NO_FINDINGS, INCOMPLETE,
and UNKNOWN. Network, lookup, parsing, missing-tool and build errors are UNKNOWN,
never a clean result. Findings do not block merging. Logs remain subject to
GitHub's ordinary retention; there is no separate result/artifact storage.

Gitleaks scans all fetched history with complete redaction. Only locations and
rule IDs are printed. Bandit covers every tracked Python file, including tests
and packaging scripts, so assertion/subprocess findings need human triage.
Every tracked npm lock is audited without installation or lifecycle scripts;
unlocked manifests are listed as incomplete coverage.

For uv.lock, frozen uv export includes all extras and groups. Exact registry pins
are audited as a union of platforms, without environment markers, to avoid
silently omitting macOS/Windows dependencies on Linux. PyPI lookups cannot audit
local/project/git packages; those are explicitly listed as incomplete coverage.
This does not execute or install the application or its build hooks.

CodeQL runs separately for each supported language in this repository, including
workflow files. Init/analyze are SHA-pinned. Results stay in the temporary runner
filesystem with upload disabled, so this does not create code-scanning alerts or
SARIF artifacts. Inspect Analyze logs; a failed matrix job is UNKNOWN. Existing
GitHub default-setup status was not readable, and is not claimed absent.

Swift helpers are not analyzed in this Linux-only pilot; Swift CodeQL requires macOS.

Cargo audits reuse the existing CI audit job and daily cadence rather than adding a duplicate. The job now checks all three Cargo.lock files in report-only mode.
