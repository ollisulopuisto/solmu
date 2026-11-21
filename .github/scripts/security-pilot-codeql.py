#!/usr/bin/env python3
"""Print CodeQL rule/location metadata, not matched source, from temporary SARIF."""
import json
import os
from pathlib import Path
import sys

files = list(Path(sys.argv[1]).glob('*.sarif'))
if not files:
    raise SystemExit('UNKNOWN: no CodeQL SARIF produced')
count = 0
for path in files:
    data = json.loads(path.read_text())
    for run in data['runs']:
        for invocation in run.get('invocations', []):
            if invocation.get('executionSuccessful') is False:
                raise SystemExit('UNKNOWN: CodeQL invocation unsuccessful')
        for result in run.get('results', []):
            count += 1
            locations = [dict(file=loc.get('physicalLocation', {}).get('artifactLocation', {}).get('uri'), line=loc.get('physicalLocation', {}).get('region', {}).get('startLine')) for loc in result.get('locations', [])]
            print(json.dumps({'rule': result.get('ruleId'), 'level': result.get('level'), 'locations': locations}))
message = 'CodeQL report-only: '+str(count)+' findings. Inspect init/analyze warnings for extraction coverage; no whole-repo clean verdict.\n'
print(message)
if os.environ.get('GITHUB_STEP_SUMMARY'):
    with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as output:
        output.write(message)
