"""Only exact-commit, intact, successfully validated binaries may be reused."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('ci_artifact', Path(__file__).resolve().parents[1] / 'ci_artifact.py')
artifact = importlib.util.module_from_spec(spec)
spec.loader.exec_module(artifact)

class ArtifactTests(unittest.TestCase):
    def test_latest_matching_main_validation_must_succeed(self):
        run = dict(id=3, head_sha='abc', head_branch='main', event='push',
                   path='.github/workflows/ci.yml', status='completed', conclusion='success')
        self.assertEqual(artifact.validated_run([run], 'abc'), 3)
        for changes in [dict(head_sha='other'), dict(head_branch='feature'), dict(event='pull_request'),
                        dict(path='other.yml'), dict(status='in_progress'), dict(conclusion='failure')]:
            self.assertIsNone(artifact.validated_run([dict(run, **changes)], 'abc'))
        self.assertIsNone(artifact.validated_run([run, dict(run, id=4, conclusion='failure')], 'abc'))

    def test_checksum_and_commit_required_for_both_tools(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'COMMIT').write_text('abc\n')
            for name in ('scene-render', 'sr-fuzz'):
                (root / name).write_bytes(name.encode())
            checksums = ''.join(f'{hashlib.sha256(name.encode()).hexdigest()}  {name}\n'
                                for name in ('scene-render', 'sr-fuzz'))
            (root / 'SHA256SUMS').write_text(checksums)
            artifact.verify_tools(root, 'abc')
            with self.assertRaises(ValueError):
                artifact.verify_tools(root, 'other')
            (root / 'sr-fuzz').write_bytes(b'corrupt')
            with self.assertRaises(ValueError):
                artifact.verify_tools(root, 'abc')
            (root / 'SHA256SUMS').write_text(checksums.splitlines()[0] + '\n')
            with self.assertRaises(ValueError):
                artifact.verify_tools(root, 'abc')
            (root / 'SHA256SUMS').write_text(checksums + 'abc  ../outside\n')
            with self.assertRaises(ValueError):
                artifact.verify_tools(root, 'abc')

if __name__ == '__main__':
    unittest.main()
