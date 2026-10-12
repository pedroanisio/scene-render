"""Cached timestamps may only be restored for byte-identical build inputs."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('ci_source_times', Path(__file__).resolve().parents[1] / 'ci_source_times.py')
ci_source_times = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci_source_times)


class SourceTimesTest(unittest.TestCase):
    def test_unchanged_inputs_are_fresh_but_changed_and_new_inputs_invalidate_cargo(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            unchanged, changed, added = [root / name for name in ('same.rs', 'changed.rs', 'new.rs')]
            unchanged.write_text('fn same() {}')
            changed.write_text('fn before() {}')
            old = 1_600_000_000_000_000_000
            for path in (unchanged, changed):
                os.utime(path, ns=(old, old))
            snapshot = ci_source_times.snapshot(root, ['same.rs', 'changed.rs'])
            changed.write_text('fn after() {}')
            added.write_text('fn new() {}')
            fresh = old + 10_000_000_000
            for path in (unchanged, changed, added):
                os.utime(path, ns=(fresh, fresh))
            restored = ci_source_times.restore(root, snapshot)
            self.assertEqual(restored, 1)
            self.assertEqual(unchanged.stat().st_mtime_ns, old)
            self.assertEqual(changed.stat().st_mtime_ns, fresh)
            self.assertEqual(added.stat().st_mtime_ns, fresh)
            self.assertEqual(ci_source_times.restore(root, json.loads(json.dumps(snapshot))), 1)
            unchanged.chmod(0o755)
            os.utime(unchanged, ns=(fresh, fresh))
            self.assertEqual(ci_source_times.restore(root, snapshot), 0)
            self.assertEqual(unchanged.stat().st_mtime_ns, fresh)

    def test_deleted_files_symlinks_and_paths_outside_workspace_are_not_touched(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / 'target.rs'
            target.write_text('fn target() {}')
            (root / 'link.rs').symlink_to(target)
            snapshot = ci_source_times.snapshot(root, ['target.rs', 'link.rs'])
            self.assertNotIn('link.rs', snapshot)
            target.unlink()
            self.assertEqual(ci_source_times.restore(root, snapshot), 0)
            for name in ('../escape', '/tmp/escape'):
                with self.assertRaises(ValueError):
                    ci_source_times.restore(root, {name: {'sha256': '', 'mtime_ns': 0}})

if __name__ == '__main__':
    unittest.main()
