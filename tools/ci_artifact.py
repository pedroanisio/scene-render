"""Fetch an exact-commit validated main binary for performance comparisons."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def validated_run(runs, sha):
    matching = [run for run in runs if run.get('head_sha') == sha and run.get('head_branch') == 'main'
                and run.get('event') == 'push' and run.get('path') == '.github/workflows/ci.yml']
    latest = max(matching, key=lambda run: run['id'], default=None)
    if latest and latest.get('status') == 'completed' and latest.get('conclusion') == 'success':
        return latest['id']
    return None


def verify_tools(root, sha):
    root = Path(root)
    if (root / 'COMMIT').read_text().strip() != sha:
        raise ValueError('artifact commit differs')
    entries = [line.split() for line in (root / 'SHA256SUMS').read_text().splitlines()]
    if len(entries) != 2 or any(len(entry) != 2 for entry in entries):
        raise ValueError('invalid tool checksum inventory')
    if {entry[1] for entry in entries} != {'scene-render', 'sr-fuzz'}:
        raise ValueError('invalid tool checksum paths')
    for expected, name in entries:
        with (root / name).open('rb') as source:
            actual = hashlib.file_digest(source, 'sha256').hexdigest()
        if actual != expected:
            raise ValueError('artifact checksum differs: ' + name)


def api(endpoint):
    return json.loads(subprocess.check_output(['gh', 'api', endpoint], text=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('sha')
    parser.add_argument('output')
    parser.add_argument('--repository', default=os.environ.get('GITHUB_REPOSITORY'))
    args = parser.parse_args()
    if not args.repository:
        parser.error('--repository or GITHUB_REPOSITORY required')
    endpoint = f'repos/{args.repository}/actions/workflows/ci.yml/runs?head_sha={args.sha}&branch=main&event=push&per_page=10'
    try:
        run_id = validated_run(api(endpoint)['workflow_runs'], args.sha)
        if run_id is None:
            print('No successful exact-commit main validation; rebuild baseline.')
            return 1
        artifacts = api(f'repos/{args.repository}/actions/runs/{run_id}/artifacts?per_page=100')['artifacts']
        if not any(item['name'] == 'validated-linux-tools' and not item.get('expired', True) for item in artifacts):
            print('Validated tools unavailable; rebuild baseline.')
            return 1
        with tempfile.TemporaryDirectory() as directory:
            subprocess.run(['gh', 'run', 'download', str(run_id), '--repo', args.repository,
                            '--name', 'validated-linux-tools', '--dir', directory], check=True)
            verify_tools(directory, args.sha)
            output = Path(args.output)
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(Path(directory) / 'scene-render', output)
            output.chmod(0o755)
        print(f'Reused validated baseline from CI {run_id}: {args.sha}')
        return 0
    except subprocess.CalledProcessError:
        print('Artifact service unavailable; rebuild baseline.')
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
