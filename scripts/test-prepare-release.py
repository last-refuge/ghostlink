"""A public preview must have exactly one build for every supported platform."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("prepare", Path(__file__).with_name("prepare-release.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ReleaseTests(unittest.TestCase):
    def test_complete_set_and_missing_or_duplicate_platforms(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / "staging"
            source.mkdir()
            filenames = [
                "ghostlink-0.1.0-windows-x86_64-setup.exe",
                "ghostlink-0.1.0-windows-x86_64.zip",
                "ghostlink-0.1.0-macos-x86_64.zip",
                "ghostlink-0.1.0-macos-aarch64.zip",
                "ghostlink-0.1.0-linux-x86_64.tar.gz",
                "ghostlink-0.1.0-linux-aarch64.tar.gz",
                "ghostlink-git-0.1.0.r1-1-x86_64.pkg.tar.zst",
            ]
            for filename in filenames:
                (source / filename).write_bytes(filename.encode())
            installer = Path(__file__).resolve().parent.parent / "install.sh"
            output = root / "release"
            module.prepare(source, output, installer)
            manifest = (output / "SHA256SUMS").read_text().splitlines()
            self.assertEqual(len(manifest), 8)
            for line in manifest:
                digest, name = line.split("  ")
                self.assertEqual(hashlib.sha256((output / name).read_bytes()).hexdigest(), digest)
            missing = source / filenames[-1]
            missing.unlink()
            with self.assertRaises(ValueError):
                module.prepare(source, root / "invalid", installer)
            self.assertFalse((root / "invalid").exists())
            missing.write_bytes(b"restored")
            duplicate = source / "duplicate"
            duplicate.mkdir()
            (duplicate / filenames[0]).write_bytes(b"duplicate")
            with self.assertRaises(ValueError):
                module.prepare(source, root / "ambiguous", installer)
            self.assertFalse((root / "ambiguous").exists())


if __name__ == "__main__":
    unittest.main()
