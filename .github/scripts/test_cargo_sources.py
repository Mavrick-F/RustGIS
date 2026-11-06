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
        self.assertEqual(self.check_config(".cargo/config.toml", '[alias]\nxtask = "run -p xtask --"\n'), [])

    def test_replacements_and_path_overrides_are_rejected_in_both_formats(self):
        for name in [".cargo/config", "fuzz/.cargo/config.toml"]:
            for text in ['[source.crates-io]\nreplace-with = "mirror"\n', 'paths = ["../dependency"]\n']:
                with self.subTest(name=name, text=text):
                    problems = self.check_config(name, text)
                    self.assertEqual(len(problems), 1)
                    self.assertTrue(problems[0].startswith(name + ":"))

    def test_invalid_toml_is_rejected(self):
        self.assertEqual(self.check_config(".cargo/config.toml", "[broken"), [".cargo/config.toml: cannot read valid TOML"])


if __name__ == "__main__":
    unittest.main()
