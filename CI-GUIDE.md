# Fast-beeper CI and urgent builds

## Which checks run

- Every PR/main push reports CI selection and the separate policy workflow.
  Documentation-only changes don't start native compilation. Avoid workflow-wide
  path ignores so status reporting remains predictable.
- Beeper Python packaging/launcher or harness-only changes run the current-source
  adapter tests plus Python tests on Linux, without recompiling four native apps.
- Runtime source/assets/native-packaging changes run those portable checks first,
  then the macOS test leg (tests, demo build, window smoke test) as the merge
  gate; quality, Ubuntu tests and a PR's Nix package wait for it and report
  later, so the Mac result lands first and the rest may finish after a merge.
  A failed macOS leg does not hide Linux feedback. Windows test legs, Debian12,
  Windows ARM and the three-system Nix fan-out still run only on version tags,
  merge queues and manual scope=full, matching the recorded preference that
  full platform matrices belong to tagged releases or manual runs; release
  coverage is unchanged. Nix inputs (flake/flatpak) still select native+Nix.
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
