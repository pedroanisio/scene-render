#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Measure skew transforms against zero skew on the same stack.

Run inside the SDK after other jobs finish. This reports feature cost, not
the unchanged-scene performance gate. Each variant independently matches all
24 frame hashes at one/four threads and throughout the alternating runs.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import statistics
import tempfile
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parent.parent
# Reuse the existing regression-tested 24-frame/metrics/sanitizer validator.
SPEC = importlib.util.spec_from_file_location(
    'sr_length_benchmark', Path(__file__).with_name('length-benchmark.py'))
HELPERS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HELPERS)


def inputs(directory):
    fixture = ROOT / 'tests/data-skew.xml'
    tree = ET.parse(fixture)
    changed = {'attributes': 0, 'keys': 0}
    for node in tree.iter():
        for name in ['skewX', 'skewY']:
            if name in node.attrib:
                node.set(name, '0')
                changed['attributes'] += 1
        if node.tag == 'animate' and node.get('property') in ['skew.x', 'skew.y']:
            for key in node.findall('key'):
                key.set('value', '0')
                changed['keys'] += 1
    if not all(changed.values()):
        raise RuntimeError('expected static and animated skew in the fixture')
    normal = directory / 'normal.xml'
    tree.write(normal, encoding='utf-8', xml_declaration=True)
    return [normal, fixture], changed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('build/scene-render'))
    parser.add_argument('--runs', type=int, default=5)
    args = parser.parse_args()
    if not 1 <= args.runs <= 100:
        parser.error('--runs must be in [1,100]')
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='sr-skew-perf-') as temporary:
        directory = Path(temporary)
        scenes, count = inputs(directory)
        trace = directory / 'metrics.jsonl'
        references = []
        for scene in scenes:
            reference, _ = HELPERS.render(binary, scene, 1, trace)
            parallel, _ = HELPERS.render(binary, scene, 4, trace)
            if reference != parallel:
                raise RuntimeError(f'thread-count mismatch: {scene.name}')
            references.append(reference)
        if references[0] == references[1]:
            raise RuntimeError('skew fixture did not change the zero-skew output')
        samples = [[], []]
        for run in range(args.runs):
            for variant in ([0, 1] if run % 2 == 0 else [1, 0]):
                hashes, cpu = HELPERS.render(binary, scenes[variant], 4, trace)
                if hashes != references[variant]:
                    raise RuntimeError(f'frame mismatch in run {run}')
                samples[variant].append(cpu)
        report = {'scene': 'tests/data-skew.xml', 'zeroed_skew': count,
                  'frames': 24, 'runs': args.runs, 'threads': 4,
                  'each_variant_hashes_match': True, 'stages': {}}
        for stage in ['composite', 'stage_total']:
            values = [[sum(cpu.values()) if stage == 'stage_total'
                       else cpu[stage] for cpu in sample] for sample in samples]
            medians = [statistics.median(value) for value in values]
            report['stages'][stage] = {
                'zero_skew_cpu_s': medians[0], 'skew_cpu_s': medians[1],
                'delta_percent': (medians[1] / medians[0] - 1) * 100,
                'zero_skew_range_s': [min(values[0]), max(values[0])],
                'skew_range_s': [min(values[1]), max(values[1])]}
        print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
