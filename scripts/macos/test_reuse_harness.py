"""Safety regression tests; never launch a GUI or touch the live registry."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("reuse_harness", Path(__file__).with_name("test-reuse.py"))
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)


class PreflightTests(unittest.TestCase):
    def test_only_missing_or_empty_private_registry_is_allowed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            HARNESS.require_empty_registry(root / "missing")
            HARNESS.require_empty_registry(root)
            endpoint = root / "existing-instance"
            endpoint.mkdir()
            with self.assertRaises(RuntimeError):
                HARNESS.require_empty_registry(root)
            self.assertTrue(endpoint.is_dir(), "preflight must not modify existing entries")

    def test_unsafe_or_symlinked_registry_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            private = root / "private"
            private.mkdir(mode=0o700)
            link = root / "link"
            link.symlink_to(private, target_is_directory=True)
            with self.assertRaises(RuntimeError):
                HARNESS.require_empty_registry(link)
            private.chmod(0o755)
            with self.assertRaises(RuntimeError):
                HARNESS.require_empty_registry(private)


if __name__ == "__main__":
    unittest.main()
