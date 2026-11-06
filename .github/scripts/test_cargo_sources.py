"""Regression checks for Cargo configuration that can bypass registry source checks."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("cargo_sources", Path(__file__).with_name("check-cargo-sources.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CargoSources(unittest.TestCase):
    def check_config(self, name, text):
        """Track a config in an isolated repository, with no dependency downloads or builds."""
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            subprocess.run(["git", "init", "-q", folder], check=True)
            path = root / name
            path.parent.mkdir(parents=True)
            path.write_text(text)
            subprocess.run(["git", "add", "--", name], cwd=root, check=True)
            return MODULE.check(root)

    def test_build_options_are_allowed(self):
        for name in [".cargo/config.toml", ".Cargo/Config.toml"]:
            with self.subTest(name=name):
                self.assertEqual(self.check_config(name, '[alias]\nxtask = "run -p xtask --"\n'), [])

    def test_overrides_and_includes_are_rejected_in_both_formats(self):
        for name in [".cargo/config", "fuzz/.cargo/config.toml", ".Cargo/Config.toml", "fuzz/.CARGO/CONFIG"]:
            for text in [
                '[source.crates-io]\nreplace-with = "mirror"\n',
                'paths = ["../dependency"]\n',
                '[patch.crates-io]\nexample = { path = "../dependency" }\n',
                'include = ["other.toml"]\n',
                'include = [{ path = "other.toml", optional = true }]\n',
            ]:
                with self.subTest(name=name, text=text):
                    problems = self.check_config(name, text)
                    self.assertEqual(len(problems), 1)
                    self.assertTrue(problems[0].startswith(name + ":"))

    def test_invalid_toml_is_rejected(self):
        self.assertEqual(self.check_config(".cargo/config.toml", "[broken"), [".cargo/config.toml: cannot read valid TOML"])


if __name__ == "__main__":
    unittest.main()
