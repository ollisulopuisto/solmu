#!/usr/bin/env python3
"""Report-only security pilot. Findings never gate; scan errors are UNKNOWN."""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib


def run(command, cwd):
    return subprocess.run(  # noqa: S603 - fixed argv lists; never a shell.
        command, cwd=cwd, capture_output=True, text=True, timeout=900, check=False
    )


def classify(code, expected, data, valid=True):
    if code not in expected or not valid:
        return "UNKNOWN"
    return "FINDINGS" if data else "NO_FINDINGS"


def npm_result(result):
    data = json.loads(result.stdout)
    valid = isinstance(
        data.get("metadata", {}).get("vulnerabilities"), dict
    ) and not data.get("error")
    findings = [
        {
            "package": name,
            "severity": value.get("severity"),
            "direct": value.get("isDirect"),
            "urls": [
                v["url"]
                for v in value.get("via", [])
                if isinstance(v, dict) and "url" in v
            ],
        }
        for name, value in data.get("vulnerabilities", {}).items()
    ]
    return classify(result.returncode, (0, 1), findings, valid), findings


def rust_warnings(warnings):
    return [
        {
            "kind": kind,
            "package": (item.get("package") or {}).get("name"),
            "id": (item.get("advisory") or {}).get("id"),
        }
        for kind, entries in warnings.items()
        for item in entries
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checks", default="secrets,python,npm,go")
    args = parser.parse_args()
    root = Path.cwd()
    tracked = subprocess.check_output(  # noqa: S603 - fixed Git inventory argv.
        [shutil.which("git"), "ls-files", "-z"], text=True
    ).split("\0")
    tracked = [Path(p) for p in tracked if p]
    rows = []

    def record(check, scope, action):
        try:
            state, findings, notes = action()
            rows.append(
                {
                    "check": check,
                    "scope": scope,
                    "state": state,
                    "findings": findings,
                    "notes": notes,
                }
            )
        except (
            OSError,
            ValueError,
            KeyError,
            TypeError,
            AttributeError,
            subprocess.SubprocessError,
        ) as exc:
            rows.append(
                {
                    "check": check,
                    "scope": scope,
                    "state": "UNKNOWN",
                    "findings": [],
                    "notes": ["Scanner failure: " + type(exc).__name__],
                }
            )

    with tempfile.TemporaryDirectory(prefix="security-pilot-") as tmp:
        temp = Path(tmp)
        if "secrets" in args.checks.split(","):

            def scan_history():
                if (
                    subprocess.check_output(  # noqa: S603 - fixed Git metadata argv.
                        [shutil.which("git"), "rev-parse", "--is-shallow-repository"],
                        text=True,
                    ).strip()
                    == "true"
                ):
                    raise ValueError(
                        "Full history required. Fetch --unshallow before scanning."
                    )
                output = temp / "secrets.json"
                result = run(
                    [
                        "gitleaks",
                        "git",
                        ".",
                        "--redact=100",
                        "--no-banner",
                        "--exit-code=10",
                        "--log-opts=--all --full-history --diff-merges=first-parent",
                        "--report-format=json",
                        "--report-path=" + str(output),
                    ],
                    root,
                )
                data = json.loads(output.read_text()) if output.exists() else None
                findings = [
                    {
                        "rule": x["RuleID"],
                        "file": x["File"],
                        "line": x["StartLine"],
                        "commit": x["Commit"],
                    }
                    for x in data or []
                ]
                return (
                    classify(
                        result.returncode, (0, 10), findings, isinstance(data, list)
                    ),
                    findings,
                    [
                        "Full fetched history, including merge patches. "
                        "Matched values/source are never printed."
                    ],
                )

            record("gitleaks", ".", scan_history)

        if "npm" in args.checks.split(","):
            for lock in [p for p in tracked if p.name == "package-lock.json"]:

                def npm(lock=lock):
                    result = run(
                        [
                            "npm",
                            "audit",
                            "--package-lock-only",
                            "--ignore-scripts",
                            "--json",
                        ],
                        root / lock.parent,
                    )
                    state, findings = npm_result(result)
                    return (
                        state,
                        findings,
                        ["Lockfile only; no install, scripts, or fixes."],
                    )

                record("npm audit", str(lock), npm)
            unlocked = [
                str(p)
                for p in tracked
                if p.name == "package.json"
                and not (p.parent / "package-lock.json").exists()
            ]
            if unlocked:
                rows.append(
                    {
                        "check": "npm inventory",
                        "scope": ".",
                        "state": "INCOMPLETE",
                        "findings": [],
                        "notes": ["No npm lockfile: " + ", ".join(unlocked)],
                    }
                )

        if "python" in args.checks.split(","):
            python_files = [str(p) for p in tracked if p.suffix == ".py"]
            if python_files:

                def bandit():
                    output = temp / "bandit.json"
                    result = run(
                        ["bandit", "-q", "-f", "json", "-o", str(output), *python_files],
                        root,
                    )
                    data = json.loads(output.read_text())
                    findings = [
                        {
                            "rule": x["test_id"],
                            "file": x["filename"],
                            "line": x["line_number"],
                            "severity": x["issue_severity"],
                            "confidence": x["issue_confidence"],
                        }
                        for x in data["results"]
                    ]
                    errors = data.get("errors", [])
                    return (
                        classify(result.returncode, (0, 1), findings, not errors),
                        findings,
                        [
                            "Tracked Python files, including tests and build helpers.",
                            "Parse errors: " + str(len(errors)),
                        ],
                    )

                record("bandit", ".", bandit)
            for lock in [p for p in tracked if p.name == "uv.lock"]:

                def audit(lock=lock):
                    exported = temp / "export.txt"
                    result = run(
                        [
                            "uv",
                            "export",
                            "--frozen",
                            "--all-extras",
                            "--all-groups",
                            "--no-emit-local",
                            "--no-hashes",
                            "--no-header",
                            "--no-annotate",
                            "-o",
                            str(exported),
                        ],
                        root / lock.parent,
                    )
                    if result.returncode:
                        raise ValueError("uv export failed: " + result.stderr[:300])
                    # Audit all locked platforms, not only this Linux environment.
                    requirements = sorted(
                        {
                            line.split(";", 1)[0].strip()
                            for line in exported.read_text().splitlines()
                            if re.match(r"^[\w.-]+==[^\s;]+", line)
                        }
                    )
                    output = temp / "requirements.txt"
                    output.write_text("\n".join(requirements) + "\n")
                    if not requirements:
                        raise ValueError("No registry requirements exported")
                    result = run(
                        [
                            "pip-audit",
                            "--strict",
                            "--disable-pip",
                            "--no-deps",
                            "--progress-spinner=off",
                            "--desc=off",
                            "-r",
                            str(output),
                            "-f",
                            "json",
                        ],
                        root,
                    )
                    data = json.loads(result.stdout)
                    findings = [
                        {
                            "package": d["name"],
                            "version": d["version"],
                            "id": v["id"],
                            "fixes": v.get("fix_versions", []),
                        }
                        for d in data["dependencies"]
                        for v in d.get("vulns", [])
                    ]
                    skipped = [
                        d.get("name")
                        for d in data["dependencies"]
                        if d.get("skip_reason")
                    ]
                    packages = tomllib.loads((root / lock).read_text())["package"]
                    nonregistry = [
                        x["name"]
                        for x in packages
                        if "registry" not in x.get("source", {})
                    ]
                    state = classify(result.returncode, (0, 1), findings, not skipped)
                    notes = [
                        "Frozen export, all extras/groups; all locked platforms. "
                        "No packages installed.",
                        "Registry pins: " + str(len(requirements)),
                        "Not covered by PyPI advisory lookup (local/project/git): "
                        + ", ".join(nonregistry),
                    ]
                    if nonregistry and state == "NO_FINDINGS":
                        state = "INCOMPLETE"
                    return state, findings, notes

                record("pip-audit", str(lock), audit)
            manifests = [
                p
                for p in tracked
                if p.name == "pyproject.toml" and not (p.parent / "uv.lock").exists()
            ]
            if manifests:
                rows.append(
                    {
                        "check": "Python inventory",
                        "scope": ".",
                        "state": "INCOMPLETE",
                        "findings": [],
                        "notes": [
                            "No supported resolved lock: "
                            + ", ".join(map(str, manifests))
                        ],
                    }
                )

        if "rust" in args.checks.split(","):
            for lock in [p for p in tracked if p.name == "Cargo.lock"]:

                def rust(lock=lock):
                    result = run(
                        ["cargo-audit", "audit", "--file", str(lock), "--json"], root
                    )
                    data = json.loads(result.stdout)
                    findings = [
                        {
                            "id": x["advisory"]["id"],
                            "package": x["package"]["name"],
                            "version": x["package"]["version"],
                            "url": x["advisory"].get("url"),
                        }
                        for x in data["vulnerabilities"]["list"]
                    ]
                    warnings = data.get("warnings", {})
                    warning_findings = rust_warnings(warnings)
                    findings.extend(warning_findings)
                    return (
                        classify(result.returncode, (0, 1), findings),
                        findings,
                        [
                            "Includes RustSec informational warnings "
                            "and vulnerability findings."
                        ],
                    )

                record("cargo audit", str(lock), rust)

        if "go" in args.checks.split(","):
            for mod in [p for p in tracked if p.name == "go.mod"]:

                def go(mod=mod):
                    result = run(
                        ["govulncheck", "-json", "-tags=webkit2_41", "./..."],
                        root / mod.parent,
                    )
                    decoder = json.JSONDecoder()
                    text = result.stdout.lstrip()
                    messages = []
                    while text:
                        value, end = decoder.raw_decode(text)
                        messages.append(value)
                        text = text[end:].lstrip()
                    findings = [x["finding"] for x in messages if "finding" in x]
                    valid = any("config" in x for x in messages) and not any(
                        "error" in x for x in messages
                    )
                    return (
                        classify(result.returncode, (0, 3), findings, valid),
                        findings,
                        ["Package loading or advisory lookup failed; no clean verdict."]
                        if result.returncode not in (0, 3)
                        else ["Reachability analysis, current runner platform."],
                    )

                record("govulncheck", str(mod), go)

    print(json.dumps(rows, indent=2))
    summary = "\n".join(
        "- "
        + x["check"]
        + " ("
        + x["scope"]
        + "): "
        + x["state"]
        + "; "
        + str(len(x["findings"]))
        + " findings"
        for x in rows
    )
    print(summary, file=sys.stderr)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as output:
            output.write(
                "## Report-only security pilot\n\n"
                + summary
                + "\n\nNO_FINDINGS is not a whole-repo clean verdict. "
                "UNKNOWN/INCOMPLETE means missing coverage. "
                "See the job log for rule/location metadata.\n"
            )
    return 2 if any(x["state"] == "UNKNOWN" for x in rows) else 0


if __name__ == "__main__":
    sys.exit(main())
