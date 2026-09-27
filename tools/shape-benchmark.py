#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Measure B1-4 feature cost on 24-frame 640x360 scenes (run inside the SDK).

Three pairs, each alternating runs of both variants at four threads:

* path-renderer: 1200 integer-aligned rects drawn by the 1.0 analytic
  renderer versus the same rects with an explicit default B1-4 attribute
  (paintOrder="fill-stroke"), which selects the path renderer. Both
  variants must produce identical frame hashes.
* strokes: the same rects with a 2 px dashed, round-joined stroke on the
  path renderer versus the legacy centred stroke (different output).
* background: a solid project background versus an oklab, dithered linear
  gradient background (different output).

Each variant's frames match at one and four threads and in every timed run.
This reports feature cost, not the unchanged-scene performance gate.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import statistics
import tempfile


ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    'sr_length_benchmark', Path(__file__).with_name('length-benchmark.py'))
HELPERS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HELPERS)


def rects(extra, background='#101820', paints=''):
    parts = ['<scene version="1.1"><project width="640" height="360" fps="12" '
             f'duration="2" background="{background}"/>{paints}<composition>']
    for y in range(30):
        for x in range(40):
            parts.append(f'<shape id="r{y}_{x}" shape="rect" x="{x * 16}" '
                         f'y="{y * 12}" width="12" height="8" fill="#48C9BC" '
                         f'{extra}><animate property="opacity"><key time="0" '
                         'value="1"/><key time="2" value="0.5"/></animate>'
                         '</shape>')
    parts.append('</composition></scene>')
    return ''.join(parts)


GRADIENT = ('<paints><linearGradient id="bg" x1="0" y1="0" x2="1" y2="1" '
            'interpolationSpace="oklab"><stop offset="0" color="#101830"/>'
            '<stop offset="1" color="#402050"/></linearGradient></paints>')

PAIRS = {
    'path-renderer': (rects(''), rects('paintOrder="fill-stroke"'), True),
    'strokes': (rects('stroke="#FFFFFF" strokeWidth="2"'),
                rects('stroke="#FFFFFF" strokeWidth="2" strokeJoin="round" '
                      'dash="4 2"'), False),
    'background': (rects(''), rects('', 'url(#bg)', GRADIENT), False),
}


def measure(binary, directory, name, runs):
    legacy, feature, same = PAIRS[name]
    scenes = []
    for label, text in (('legacy', legacy), ('feature', feature)):
        path = directory / f'{name}-{label}.xml'
        path.write_text(text)
        scenes.append(path)
    trace = directory / 'metrics.jsonl'
    references = []
    for scene in scenes:
        one, _ = HELPERS.render(binary, scene, 1, trace)
        four, _ = HELPERS.render(binary, scene, 4, trace)
        if one != four:
            raise RuntimeError(f'thread-count mismatch: {scene.name}')
        references.append(one)
    if same != (references[0] == references[1]):
        raise RuntimeError(f'{name}: unexpected output equality {not same}')
    samples = [[], []]
    for run in range(runs):
        for variant in ([0, 1] if run % 2 == 0 else [1, 0]):
            hashes, cpu = HELPERS.render(binary, scenes[variant], 4, trace)
            if hashes != references[variant]:
                raise RuntimeError(f'{name}: frame mismatch in run {run}')
            samples[variant].append(cpu)
    result = {'identical_output': same, 'stages': {}}
    for stage in ['clear', 'composite', 'stage_total']:
        values = [[sum(cpu.values()) if stage == 'stage_total' else cpu.get(stage, 0.0)
                   for cpu in sample] for sample in samples]
        medians = [statistics.median(value) for value in values]
        result['stages'][stage] = {
            'legacy_cpu_s': medians[0], 'feature_cpu_s': medians[1],
            'delta_percent': (medians[1] / medians[0] - 1) * 100 if medians[0] else None,
            'legacy_range_s': [min(values[0]), max(values[0])],
            'feature_range_s': [min(values[1]), max(values[1])]}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('build/scene-render'))
    parser.add_argument('--runs', type=int, default=5)
    args = parser.parse_args()
    if not 1 <= args.runs <= 100:
        parser.error('--runs must be in [1,100]')
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='sr-shape-perf-') as temporary:
        directory = Path(temporary)
        report = {'frames': 24, 'runs': args.runs, 'threads': 4, 'pairs': {}}
        for name in PAIRS:
            report['pairs'][name] = measure(binary, directory, name, args.runs)
        print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
