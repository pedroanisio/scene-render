#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Regression tests for failures the render verification tools must not hide."""
import importlib.util
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


if __name__ == '__main__':
    unittest.main()
