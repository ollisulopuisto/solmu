#!/usr/bin/env python3
"""Report-only security pilot. Findings never gate; scan errors are UNKNOWN."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib


def run(command, cwd):
    return subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=900)


def classify(code, expected, data, valid=True):
    if code not in expected or not valid:
        return 'UNKNOWN'
    return 'FINDINGS' if data else 'NO_FINDINGS'


def npm_result(result):
    data = json.loads(result.stdout)
    valid = isinstance(data.get('metadata', {}).get('vulnerabilities'), dict) and not data.get('error')
    findings = [dict(package=name, severity=value.get('severity'),
                     direct=value.get('isDirect'),
                     urls=[v['url'] for v in value.get('via', []) if isinstance(v, dict) and 'url' in v])
                for name, value in data.get('vulnerabilities', {}).items()]
    return classify(result.returncode, (0, 1), findings, valid), findings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checks', default='secrets,python,npm,go')
    args = parser.parse_args()
    root = Path.cwd()
    tracked = subprocess.check_output(['git', 'ls-files', '-z'], text=True).split('\0')
    tracked = [Path(p) for p in tracked if p]
    rows = []

    def record(check, scope, action):
        try:
            state, findings, notes = action()
            rows.append(dict(check=check, scope=scope, state=state, findings=findings, notes=notes))
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
            rows.append(dict(check=check, scope=scope, state='UNKNOWN', findings=[], notes=[type(exc).__name__ + ': ' + str(exc)[:500]]))

    with tempfile.TemporaryDirectory(prefix='security-pilot-') as tmp:
        temp = Path(tmp)
        if 'secrets' in args.checks.split(','):
            def secrets():
                if subprocess.check_output(['git', 'rev-parse', '--is-shallow-repository'], text=True).strip() == 'true':
                    raise ValueError('Full history required. Fetch --unshallow before scanning.')
                output = temp / 'secrets.json'
                result = run(['gitleaks', 'git', '.', '--redact=100', '--no-banner', '--exit-code=10', '--log-opts=--all', '--report-format=json', '--report-path='+str(output)], root)
                data = json.loads(output.read_text()) if output.exists() else None
                findings = [dict(rule=x['RuleID'], file=x['File'], line=x['StartLine'], commit=x['Commit']) for x in data or []]
                return classify(result.returncode, (0, 10), findings, isinstance(data, list)), findings, ['Full fetched history; secret values and matched source are never printed.']
            record('gitleaks', '.', secrets)

        if 'npm' in args.checks.split(','):
            for lock in [p for p in tracked if p.name == 'package-lock.json']:
                def npm(lock=lock):
                    result = run(['npm', 'audit', '--package-lock-only', '--ignore-scripts', '--json'], root / lock.parent)
                    state, findings = npm_result(result)
                    return state, findings, ['Lockfile only; no install, scripts, or fixes.']
                record('npm audit', str(lock), npm)
            unlocked = [str(p) for p in tracked if p.name == 'package.json' and not (p.parent / 'package-lock.json').exists()]
            if unlocked:
                rows.append(dict(check='npm inventory', scope='.', state='INCOMPLETE', findings=[], notes=['No npm lockfile: '+', '.join(unlocked)]))

        if 'python' in args.checks.split(','):
            python_files = [str(p) for p in tracked if p.suffix == '.py']
            if python_files:
                def bandit():
                    output = temp / 'bandit.json'
                    result = run(['bandit', '-q', '-f', 'json', '-o', str(output), *python_files], root)
                    data = json.loads(output.read_text())
                    findings = [dict(rule=x['test_id'], file=x['filename'], line=x['line_number'], severity=x['issue_severity'], confidence=x['issue_confidence']) for x in data['results']]
                    errors = data.get('errors', [])
                    return classify(result.returncode, (0, 1), findings, not errors), findings, ['Tracked Python files, including tests and build helpers.', 'Parse errors: '+str(len(errors))]
                record('bandit', '.', bandit)
            for lock in [p for p in tracked if p.name == 'uv.lock']:
                def audit(lock=lock):
                    exported = temp / 'export.txt'
                    result = run(['uv', 'export', '--frozen', '--all-extras', '--all-groups', '--no-emit-local', '--no-hashes', '--no-header', '--no-annotate', '-o', str(exported)], root / lock.parent)
                    if result.returncode:
                        raise ValueError('uv export failed: '+result.stderr[:300])
                    # Audit the union of platforms, not just the Linux runner's environment.
                    requirements = sorted(set(line.split(';', 1)[0].strip() for line in exported.read_text().splitlines() if re.match(r'^[\w.-]+==[^\s;]+', line)))
                    output = temp / 'requirements.txt'
                    output.write_text('\n'.join(requirements)+'\n')
                    if not requirements:
                        raise ValueError('No registry requirements exported')
                    result = run(['pip-audit', '--strict', '--disable-pip', '--no-deps', '--progress-spinner=off', '--desc=off', '-r', str(output), '-f', 'json'], root)
                    data = json.loads(result.stdout)
                    findings = [dict(package=d['name'], version=d['version'], id=v['id'], fixes=v.get('fix_versions', [])) for d in data['dependencies'] for v in d.get('vulns', [])]
                    skipped = [d.get('name') for d in data['dependencies'] if d.get('skip_reason')]
                    packages = tomllib.loads((root / lock).read_text())['package']
                    nonregistry = [x['name'] for x in packages if 'registry' not in x.get('source', {})]
                    state = classify(result.returncode, (0, 1), findings, not skipped)
                    notes = ['Frozen export, all extras/groups; markers removed to audit every locked platform. No packages installed.', 'Registry pins: '+str(len(requirements)), 'Not covered by PyPI advisory lookup (local/project/git): '+', '.join(nonregistry)]
                    if nonregistry and state == 'NO_FINDINGS':
                        state = 'INCOMPLETE'
                    return state, findings, notes
                record('pip-audit', str(lock), audit)
            manifests = [p for p in tracked if p.name == 'pyproject.toml' and not (p.parent / 'uv.lock').exists()]
            if manifests:
                rows.append(dict(check='Python inventory', scope='.', state='INCOMPLETE', findings=[], notes=['No supported resolved lock: '+', '.join(map(str, manifests))]))

        if 'rust' in args.checks.split(','):
            for lock in [p for p in tracked if p.name == 'Cargo.lock']:
                def rust(lock=lock):
                    result = run(['cargo-audit', 'audit', '--file', str(lock), '--json'], root)
                    data = json.loads(result.stdout)
                    findings = [dict(id=x['advisory']['id'], package=x['package']['name'], version=x['package']['version'], url=x['advisory'].get('url')) for x in data['vulnerabilities']['list']]
                    warnings = data.get('warnings', {})
                    warning_findings = [dict(kind=kind, package=x.get('package', {}).get('name'), id=x.get('advisory', {}).get('id')) for kind, entries in warnings.items() for x in entries]
                    findings.extend(warning_findings)
                    return classify(result.returncode, (0, 1), findings), findings, ['Includes RustSec informational warnings, not only vulnerabilities.']
                record('cargo audit', str(lock), rust)

        if 'go' in args.checks.split(','):
            for mod in [p for p in tracked if p.name == 'go.mod']:
                def go(mod=mod):
                    result = run(['govulncheck', '-json', './...'], root / mod.parent)
                    decoder = json.JSONDecoder()
                    text = result.stdout.lstrip()
                    messages = []
                    while text:
                        value, end = decoder.raw_decode(text)
                        messages.append(value)
                        text = text[end:].lstrip()
                    findings = [x['finding'] for x in messages if 'finding' in x]
                    valid = any('config' in x for x in messages) and not any('error' in x for x in messages)
                    return classify(result.returncode, (0, 3), findings, valid), findings, [result.stderr[:1000]] if result.returncode not in (0, 3) else ['Reachability analysis, current runner platform.']
                record('govulncheck', str(mod), go)

    print(json.dumps(rows, indent=2))
    summary = '\n'.join('- '+x['check']+' ('+x['scope']+'): '+x['state']+'; '+str(len(x['findings']))+' findings' for x in rows)
    print(summary, file=sys.stderr)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as output:
            output.write('## Report-only security pilot\n\n'+summary+'\n\nNO_FINDINGS is not a whole-repo clean verdict. UNKNOWN/INCOMPLETE means coverage is missing. Details are in the job log, without secret values.\n')
    return 2 if any(x['state'] == 'UNKNOWN' for x in rows) else 0


if __name__ == '__main__':
    sys.exit(main())
