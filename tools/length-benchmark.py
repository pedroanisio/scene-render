#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Measure scoped/mixed-unit geometry against independently authored pixels.

Run inside the SDK after other builds and render jobs have finished. This
feature-cost measurement does not replace scripts/perf-check.py's unchanged-
scene budget. Generated inputs and render artifacts stay in a temporary folder.
"""
import argparse
import json
import math
from pathlib import Path
import re
import statistics
import subprocess
import tempfile


def scene(relative, copies):
    parts = ['<scene version="1.1"><project width="320" height="192" '
             'fps="12" duration="2"/><composition>']
    for copy in range(copies):
        width, height = ('50vw', '50vh') if relative else ('160', '96')
        parts.append(f'<group id="g{copy}" width="{width}" height="{height}">')
        for y in range(32):
            for x in range(40):
                pos_x = f'{x * 5}%' if relative else str(x * 8)
                pos_y = f'{y * 6.25:g}%' if relative else str(y * 6)
                width, height = ('5%', '6.25%') if relative else ('8', '6')
                end = f'{(x + 1) * 2.5:g}vw' if relative else str((x + 1) * 8)
                mask_w, mask_h = ('75%', '100%') if relative else ('6', '6')
                parts.append(f'<shape id="s{copy}_{y}_{x}" shape="rect" '
                             f'x="{pos_x}" y="{pos_y}" width="{width}" '
                             f'height="{height}" fill="#48C9BC" opacity="0.5">'
                             f'<mask type="rect" width="{mask_w}" height="{mask_h}"/>'
                             '<animate property="position.x">'
                             f'<key time="0" value="{pos_x}"/>'
                             f'<key time="2" value="{end}"/>'
                             '</animate></shape>')
        parts.append('</group>')
    parts.append('</composition></scene>')
    return ''.join(parts)


def render(binary, path, threads, trace):
    trace.unlink(missing_ok=True)
    result = subprocess.run([str(binary), '--scene', str(path), '--hash',
                             '--threads', str(threads), '--metrics-trace', str(trace)],
                            capture_output=True, text=True, check=True)
    if any(marker in result.stderr + result.stdout for marker in
           ('runtime error:', 'ERROR: AddressSanitizer', '[FAIL]')):
        raise RuntimeError('renderer reported a sanitizer or verification failure')
    try:
        summary = json.loads(trace.read_text().splitlines()[-1])
    except (OSError, IndexError, json.JSONDecodeError) as error:
        raise RuntimeError('renderer did not write fresh valid metrics') from error
    if not isinstance(summary, dict):
        raise RuntimeError('expected a metrics summary object')
    if (summary.get('summary') is not True or summary.get('frames') != 24 or
            summary.get('status') != 0):
        raise RuntimeError('expected a successful 24-frame metrics summary')
    hashes = {}
    for line in result.stdout.splitlines():
        match = re.fullmatch(r'(\d+) ([0-9a-f]{16})', line)
        if not match or int(match[1]) in hashes:
            raise RuntimeError('malformed or duplicate frame hash')
        hashes[int(match[1])] = match[2]
    if set(hashes) != set(range(24)):
        raise RuntimeError('expected exactly frame hashes 0 through 23')
    cpu = summary.get('cpu', {})
    if ('composite' not in cpu or cpu['composite'] <= 0 or
            any(not math.isfinite(value) or value < 0 for value in cpu.values())):
        raise RuntimeError('missing or invalid stage CPU measurements')
    return hashes, cpu


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('build/scene-render'))
    parser.add_argument('--copies', type=int, choices=range(1, 33), default=4)
    parser.add_argument('--runs', type=int, default=5)
    args = parser.parse_args()
    if not 1 <= args.runs <= 100:
        parser.error('--runs must be in [1, 100]')
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='sr-length-perf-') as temporary:
        root = Path(temporary)
        inputs = [root / 'pixels.xml', root / 'relative.xml']
        for i, path in enumerate(inputs):
            path.write_text(scene(bool(i), args.copies))
        trace = root / 'metrics.jsonl'
        reference = None
        for threads in [1, 4]:
            for path in inputs:
                hashes, _ = render(binary, path, threads, trace)
                if reference is None:
                    reference = hashes
                if hashes != reference:
                    raise RuntimeError(f'frame mismatch: {path.name}, threads={threads}')
        samples = [[], []]
        for run in range(args.runs):
            for i in ([0, 1] if run % 2 == 0 else [1, 0]):
                hashes, cpu = render(binary, inputs[i], 4, trace)
                if hashes != reference:
                    raise RuntimeError(f'frame mismatch in run {run}, {inputs[i].name}')
                samples[i].append(cpu)
        report = {'nodes': args.copies * 1281 + 1, 'masks': args.copies * 1280,
                  'frames': 24, 'runs': args.runs, 'threads': 4,
                  'all_frame_hashes_match': True, 'stages': {}}
        for stage in ['composite', 'stage_total']:
            medians = []
            ranges = []
            for sample in samples:
                values = [sum(cpu.values()) if stage == 'stage_total' else cpu[stage]
                          for cpu in sample]
                medians.append(statistics.median(values))
                ranges.append([min(values), max(values)])
            report['stages'][stage] = {'pixels_cpu_s': medians[0],
                'relative_cpu_s': medians[1], 'delta_percent':
                (medians[1] / medians[0] - 1) * 100,
                'pixels_range_s': ranges[0], 'relative_range_s': ranges[1]}
        print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
