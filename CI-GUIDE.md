# Fast-beeper CI and urgent builds

## Which checks run

- Every PR/main push reports CI selection and the separate policy workflow.
  Documentation-only changes don't start native compilation. Avoid workflow-wide
  path ignores so status reporting remains predictable.
- Beeper Python packaging/launcher or harness-only changes run the current-source
  adapter tests plus Python tests on Linux, without recompiling four native apps.
- Runtime source/assets/native-packaging changes run those portable checks first,
  then quality, macOS tests/demo/window smoke, Ubuntu tests and selected Nix
  work run in parallel. A macOS success alone is not acceptance: wait for all
  selected checks. Windows test legs, Debian12, Windows ARM and the three-system
  Nix fan-out still run only on version tags, merge queues and manual scope=full;
  release coverage is unchanged. Nix inputs (flake/flatpak) still select native+Nix.
- Cargo/toolchain/build inputs and CI routing changes select every check family.
  Nix inputs select native and Nix checks. Unclassified implementation paths
  conservatively select portable/native/Nix checks. Deleted files count too.
  Exception: a diff whose only build-relevant changes rewrite bare
  `version = "..."` lines in Cargo.toml/Cargo.lock (release metadata, no
  dependency/checksum/source edits) keeps portable packaging validation and
  native quality/tests but skips the advisory audit and the Nix fan-out, and
  the Flatpak workflow skips its container build for the same diffs. Any
  dependency-line, checksum, source or name change selects full checks, and
  docs-only changes already select no builds.
- Dependency advisories run for dependency changes, full manual CI and a small
  daily audit-only run. New advisories must be detected even without commits.
- Merge queues, version tags and **CI -> Run workflow -> scope=full** select every
  family. **scope=fast** is portable-only feedback, not native or release acceptance.
  Tagging a release therefore runs the full matrix on the exact release commit.
- Obsolete checks cancel within their workflow/ref. Flatpak and packaging checks
  now have cancellation too; packaging publication is not cancelled.

`ci-scope.py` is the path contract. Its tests run before every CI selection.
Keep that mapping updated when adding new build inputs. Selection failures stop
CI rather than silently running no checks. Existing job names remain unchanged;
no branch protections or settings are edited by this change.

## ASAP hotfix compile/package path

1. Open Actions -> **Trial binaries** -> **Run workflow**.
2. Choose the reviewed branch/ref and exactly the platform needed. Default is
   `aarch64-apple-darwin` (Apple Silicon). `all` deliberately starts four builds.
3. The selected native runner runs launcher/package tests, locked real release
   compilation and portable ZIP/checksum preparation. It does not wait for CI,
   Nix, other platforms or a merge. Superseded runs of the same ref/target cancel.
4. This is an unsigned, unnotarized trial, not a signed production release or
   full acceptance. It does not make a DMG, install a backend or launch the app.
   Existing Beeper Desktop/Server and the documented Python launcher are needed.
5. Uploads stay OFF under the existing `FAST_BEEPER_UPLOAD_ARTIFACTS` opt-in gate.
   With uploads off the job is compile/package evidence only, with no retained
   downloadable binary. Do not enable uploads as a workaround for this limit.
   Local `MAC-DMG-TRIAL.md` remains the no-upload Mac trial route.

Normal tag-release signing, verification, provenance and publication are
unchanged. Before a production tag, use full CI on that exact reviewed commit
and resolve failures; the urgent route does not waive those checks or authorize
merges/tags/releases. Human-led merges only. Existing Windows x64 failures remain
open and are not reclassified as passing by this workflow change.

## Cost choices and evidence

Removed automatic four-target release-mode trial rebuilds on both PR and merge.
Ordinary native tests retain their platform matrix where relevant. Mac preview
and Flatpak uploads now use the existing upload gate too. No new caches are
introduced: existing compiler/Nix caches remain, trial/release compilation still
uses no restored compiler output, and Flatpak cache remains disabled.

On main `0c6d22d5`, live job timing observed on October29,2026 was about17.3minutes
for the Apple Silicon trial,21.6 for Ubuntu tests,22.8 for Debian compilation,
7.4 for Windows ARM checks and3.2 for advisories. These are individual observed
runner durations, not billed-minute estimates or promised savings. Several jobs
were still running at the observation. Queuing, warm caches, cancellations and
runner type make extrapolation unreliable. Measure completed post-merge runs
before claiming actual savings.

Reproduce workflow validation locally:

```sh
python3 .github/scripts/test-ci-scope.py
python3 .github/scripts/test-ci-workflows.py
./contrib/beeper-tools/check-source.sh
actionlint .github/workflows/ci.yml .github/workflows/trial-binaries.yml \
  .github/workflows/flatpak.yml .github/workflows/packaging.yml
```

Revert this focused change to restore the previous automatic trial routing.

## Approved parallel/cache/screenshot optimization

Owner approval on26November2026 covers options1,2and5 together: parallel native
work, same-repository PR cache saves, and skipping only reviewed unaffected
screenshot scenes. These form one requested CI optimization; no wait reductions
or Rust test filtering. Parallel work can consume more runner minutes on a PR
that later fails macOS, in exchange for earlier all-platform feedback.

The cache action input and implementation were both checked:
https://raw.githubusercontent.com/jdx/mr-boxington-action/v1/action.yml
https://raw.githubusercontent.com/jdx/mr-boxington-action/v1/dist/index.js
The implementation compares head/base repository full names, requires the
standard PR merge ref, and refuses fork saves. The GitHub cache scope is the
PR merge ref; existing generations remain separate by job. This enables later
PR-run reuse, not a promise that edited app code will never recompile. Actual
second-run effects still require measurement.

The canonical gallery lives only in `.github/scripts/scene-map.json`; both
workflow jobs load it through `ci-scope.py --gallery`. Completeness and fail-open
guards are tested before selection. Startup and About/settings captures always
run. See `docs/ci-scene-selection.md` for the reviewed reachability limits and
the historical replay showing no new savings for the sampled recent PR mix.
