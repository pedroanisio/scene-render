#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Regression tests for failures the render verification tools must not hide."""
import importlib.util
import json
import pathlib
import signal
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parent.parent


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'tools' / (name + '.py'))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


class VerificationFailures(unittest.TestCase):
    def test_benchmark_rejects_stale_metrics(self):
        tool = module('length-benchmark')
        hashes = ''.join(f'{i} {i:016x}\n' for i in range(24))
        valid = {'summary': True, 'frames': 24, 'status': 0,
                 'cpu': {'composite': 1.0}}
        with tempfile.TemporaryDirectory() as directory:
            trace = pathlib.Path(directory) / 'trace.jsonl'
            trace.write_text(json.dumps(valid) + '\n')
            result = subprocess.CompletedProcess(['renderer'], 0, hashes, '')
            with patch.object(tool.subprocess, 'run', return_value=result):
                with self.assertRaises(RuntimeError):
                    tool.render('renderer', 'scene', 4, trace)

    def test_length_benchmark_requires_complete_successful_frames(self):
        tool = module('length-benchmark')
        hashes = ''.join(f'{i} {i:016x}\n' for i in range(24))
        valid = {'summary': True, 'frames': 24, 'status': 0, 'cpu': {'composite': 1.0}}
        cases = [(hashes, valid, ''), ('', valid, ''), (hashes.splitlines()[0], valid, ''),
                 (hashes + hashes.splitlines()[0], valid, ''),
                 (hashes.replace('23 ', '24 '), valid, ''),
                 (hashes, dict(valid, frames=1), ''),
                 (hashes, dict(valid, status=5), ''),
                 (hashes, valid, 'runtime error: overflow')]
        with tempfile.TemporaryDirectory() as directory:
            trace = pathlib.Path(directory) / 'trace.jsonl'
            for index, (stdout, summary, stderr) in enumerate(cases):
                with self.subTest(case=index):
                    result = subprocess.CompletedProcess(['renderer'], 0, stdout, stderr)

                    def complete(*args, **kwargs):
                        trace.write_text(json.dumps(summary) + '\n')
                        return result

                    with patch.object(tool.subprocess, 'run', side_effect=complete):
                        if index == 0:
                            tool.render('renderer', 'scene', 4, trace)
                        else:
                            with self.assertRaises(RuntimeError):
                                tool.render('renderer', 'scene', 4, trace)

    def test_benchmark_requires_fresh_valid_metrics(self):
        tool = module('length-benchmark')
        hashes = ''.join(f'{i} {i:016x}\n' for i in range(24))
        stale = {'summary': True, 'frames': 24, 'status': 0,
                 'cpu': {'composite': 99.0}}
        fresh = dict(stale, cpu={'composite': 2.0})
        with tempfile.TemporaryDirectory() as directory:
            trace = pathlib.Path(directory) / 'trace.jsonl'
            for content in ('', '{broken', '[]', json.dumps(fresh)):
                with self.subTest(content=content):
                    trace.write_text(json.dumps(stale) + '\n')

                    def complete(*args, **kwargs):
                        self.assertFalse(trace.exists())
                        trace.write_text(content)
                        return subprocess.CompletedProcess(['renderer'], 0, hashes, '')

                    with patch.object(tool.subprocess, 'run', side_effect=complete):
                        if content == json.dumps(fresh):
                            _, cpu = tool.render('renderer', 'scene', 4, trace)
                            self.assertEqual(cpu['composite'], 2.0)
                        else:
                            with self.assertRaises(RuntimeError):
                                tool.render('renderer', 'scene', 4, trace)

    def test_frame_order_preserves_root_sequence(self):
        tool = module('frame-order-check')
        for prelude in ('', '<styles/>', '<metadata/><styles/>',
                        '<metadata/><parameters/><styles/><colorManagement/>'):
            for existing in ('', '<output path="old.mp4"/><output path="other.mp4"/>'):
                with self.subTest(prelude=prelude, existing=existing):
                    with tempfile.TemporaryDirectory() as directory:
                        work = pathlib.Path(directory)
                        scene = work / 'input.xml'
                        scene.write_text('<scene version="1.1"><project width="16" '
                                         'height="16" fps="1" duration="1"/>' +
                                         prelude + existing + '<assets/><composition/></scene>')
                        expected = ['project'] + [node.tag for node in
                            ET.fromstring('<root>' + prelude + '</root>')]
                        expected += ['output', 'assets', 'composition']

                        def inspect_document(args):
                            root = ET.parse(args[args.index('--scene') + 1]).getroot()
                            self.assertEqual([node.tag for node in root], expected)
                            self.assertEqual(root.find('output').get('codec'), 'ffv1')
                            raise RuntimeError('document order checked')

                        with patch.object(tool, 'run', side_effect=inspect_document):
                            with self.assertRaisesRegex(RuntimeError, 'document order checked'):
                                tool.check('renderer', 'hooks', scene, work, 1)

    def test_recovering_sanitizer_is_failure(self):
        for name in ('equivalence-oracle', 'frame-order-check'):
            tool = module(name)
            for diagnostic in ('runtime error: overflow', 'ERROR: AddressSanitizer',
                               '[FAIL] unexpected output'):
                with self.subTest(tool=name, diagnostic=diagnostic):
                    result = subprocess.CompletedProcess(['renderer'], 0, '', diagnostic)
                    with patch.object(tool.subprocess, 'run', return_value=result):
                        with self.assertRaises(RuntimeError):
                            tool.run(['renderer'])

    def test_interruption_requires_sigkill_without_sanitizer_error(self):
        tool = module('frame-order-check')
        for code, error in ((1, ''), (0, ''), (-signal.SIGSEGV, ''),
                            (-signal.SIGKILL, 'runtime error: overflow')):
            with self.subTest(code=code, error=error):
                result = subprocess.CompletedProcess(['renderer'], code, '', error)
                with patch.object(tool.subprocess, 'run', return_value=result):
                    with self.assertRaises(RuntimeError):
                        tool.run(['renderer'], expect='abort')
        result = subprocess.CompletedProcess(['renderer'], -signal.SIGKILL, '', '')
        with patch.object(tool.subprocess, 'run', return_value=result):
            self.assertIs(tool.run(['renderer'], expect='abort'), result)


class ValidatorStyles(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tool = module('validate-scene')
        cls.schema = cls.tool.Schema(cls.tool.DEFAULT_SCHEMA)

    def report(self, tokens, color='var(--ink)'):
        scene = ET.Element('scene', version='1.1')
        ET.SubElement(scene, 'project', width='64', height='32', fps='12',
                      duration='2', background=color)
        styles = ET.SubElement(scene, 'styles')
        for name, value in tokens:
            ET.SubElement(styles, 'token', name=name, value=value)
        ET.SubElement(scene, 'composition')
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'scene.xml'
            ET.ElementTree(scene).write(path)
            report = self.tool.Report(str(path))
            doc = self.tool.parse_document(str(path), report)
            self.schema.assign_types(doc)
            self.tool.Checker(doc, self.schema, report, str(path), False).check_values()
            return report.items

    def test_aliases_and_unused_literals(self):
        tokens = [('ink', 'var(--base)'), ('base', '#102030'),
                  ('unused', '42'), ('empty', ''), ('Ink', '#FFFFFF')]
        self.assertEqual(self.report(tokens), [])

    def test_alias_failures_even_when_unused(self):
        for tokens in ([('a', 'var(--b)'), ('b', 'var(--a)')],
                       [('a', 'var(--missing)')], [('a', 'var(--bad name)')]):
            with self.subTest(tokens=tokens):
                items = self.report(tokens, '#102030')
                self.assertTrue(items)
                self.assertTrue(all(i['rule'] == 'TOKEN-REF' for i in items))

    def test_alias_resolves_to_color_at_use(self):
        items = self.report([('ink', 'var(--number)'), ('number', '42')])
        self.assertTrue(any(i['rule'] == 'TOKEN-REF' and
                            i['where'] == '<project> @background' for i in items))

    def test_spaced_reference_is_only_an_unused_literal(self):
        tokens = [('ink', ' var(--base)'), ('base', '#102030')]
        self.assertEqual(self.report(tokens, '#102030'), [])
        items = self.report(tokens)
        self.assertTrue(any(i['rule'] == 'TOKEN-REF' and
                            i['where'] == '<project> @background' for i in items))

    def test_alias_depth_boundary_in_both_orders(self):
        for hops in (64, 65):
            tokens = [('ink', 'var(--n0)')]
            tokens += [(f'n{i}', f'var(--n{i+1})') for i in range(hops - 1)]
            tokens.append((f'n{hops-1}', '#102030'))
            for ordered in (tokens, list(reversed(tokens))):
                with self.subTest(hops=hops, reverse=ordered is not tokens):
                    self.assertEqual(bool(self.report(ordered)), hops > 64)

    def test_existing_styles_fixture(self):
        path = str(ROOT / 'tests/data-styles.xml')
        report = self.tool.Report(path)
        doc = self.tool.parse_document(path, report)
        self.tool.Checker(doc, self.schema, report, path, False).run(None)
        self.assertEqual(report.items, [])


if __name__ == '__main__':
    unittest.main()
