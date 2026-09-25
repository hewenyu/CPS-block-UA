import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]
FILES = ("Cargo.toml", "Cargo.lock", "plugin.json")


class ReleaseVersionTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for name in FILES:
            shutil.copyfile(ROOT / name, self.root / name)

    def run_tag(self, tag):
        return subprocess.run(
            [sys.executable, str(ROOT / "scripts/set-version.py"), tag],
            cwd=self.root, capture_output=True, text=True,
        )

    def test_stable_and_prerelease_update_all_versions_without_dependency_changes(self):
        original = tomllib.loads((self.root / "Cargo.lock").read_text())
        for tag in ("v1.2.3", "v2.0.0-rc.1"):
            with self.subTest(tag=tag):
                result = self.run_tag(tag)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(tomllib.loads((self.root / "Cargo.toml").read_text())["package"]["version"], tag[1:])
                self.assertEqual(json.loads((self.root / "plugin.json").read_text())["version"], tag[1:])
                lock = tomllib.loads((self.root / "Cargo.lock").read_text())
                local = next(p for p in lock["package"] if p["name"] == "cps-block-ua")
                self.assertEqual(local["version"], tag[1:])
                local["version"] = next(p["version"] for p in original["package"] if p["name"] == "cps-block-ua")
                self.assertEqual(lock, original)

    def test_invalid_tags_fail_before_writing(self):
        original = {name: (self.root / name).read_bytes() for name in FILES}
        for tag in ("1.2.3", "v01.2.3", "v1.2", "v1.2.3-rc.01", "v1.2.3-", "v1.2.3\ncommand", "v1.2.3/evil"):
            with self.subTest(tag=tag):
                self.assertNotEqual(self.run_tag(tag).returncode, 0)
                self.assertEqual({name: (self.root / name).read_bytes() for name in FILES}, original)


if __name__ == "__main__":
    unittest.main()
