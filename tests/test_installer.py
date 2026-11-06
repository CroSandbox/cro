"""Installer regression tests using isolated fixtures and no system changes."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


REPOSITORY = Path(__file__).resolve().parents[1]
BINARIES = ("microvm", "microvm-runner", "containerd-shim-krun-v2", "krun-operator")


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="cro installer ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.workspace = self.root / "krun-microvm"
        self.workspace.mkdir()
        (self.workspace / "Cargo.toml").touch()
        shutil.copy2(REPOSITORY / "krun-microvm/install.sh", self.workspace)
        (self.root / "install.sh").symlink_to("krun-microvm/install.sh")
        self.prefix = self.root / "bin"
        self.prefix.mkdir()
        self.commands = self.root / "commands"
        self.commands.mkdir()
        # Fail closed if an unintended build, dependency install, or signing runs.
        for name in ("cargo", "rustc", "brew", "sudo", "curl", "codesign"):
            self.write_command(name, "exit 99")

    def write_command(self, name, body):
        path = self.commands / name
        path.write_text("#!/usr/bin/env bash\n" + body + "\n")
        path.chmod(0o755)

    def invoke(self, *arguments, entry="install.sh", skip_submodules=True):
        flags = ["--skip-deps", "--no-sign", "--prefix", str(self.prefix)]
        if skip_submodules:
            flags.append("--skip-submodules")
        return subprocess.run(
            ["bash", str(self.root / entry), *flags, *arguments],
            cwd=self.root.parent,
            env={**os.environ, "PATH": str(self.commands) + os.pathsep + os.environ["PATH"]},
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_help_from_both_entry_points(self):
        for entry in ("install.sh", "krun-microvm/install.sh"):
            with self.subTest(entry=entry):
                result = self.invoke("--help", entry=entry)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("--prereqs-only", result.stdout)

    def test_prerequisites_do_not_require_or_install_binaries(self):
        for entry in ("install.sh", "krun-microvm/install.sh"):
            with self.subTest(entry=entry):
                result = self.invoke("--prereqs-only", entry=entry)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("No binaries were built or installed", result.stdout)
                self.assertEqual(list(self.prefix.iterdir()), [])

    def test_prefix_requires_a_nonempty_value(self):
        for arguments in (("--prefix",), ("--prefix", ""), ("--prefix", "--debug")):
            with self.subTest(arguments=arguments):
                result = self.invoke(*arguments)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("--prefix requires a destination directory", result.stderr)
                self.assertNotIn("unbound variable", result.stderr)

    def test_no_build_still_installs_existing_binaries(self):
        target = self.workspace / "target/release"
        target.mkdir(parents=True)
        for name in BINARIES:
            binary = target / name
            binary.write_text("#!/usr/bin/env bash\nexit 0\n")
            binary.chmod(0o755)
        result = self.invoke("--no-build")
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in BINARIES:
            self.assertEqual((self.prefix / name).read_bytes(), (target / name).read_bytes())
            self.assertTrue(os.access(self.prefix / name, os.X_OK))
        self.assertEqual(os.readlink(self.prefix / "cro"), "microvm")

    def test_missing_binaries_fail_before_installation(self):
        result = self.invoke("--no-build")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Binary not found", result.stderr)
        self.assertEqual(list(self.prefix.iterdir()), [])

    def test_submodule_failure_stops_prerequisite_setup(self):
        (self.root / ".git").mkdir()
        self.write_command("git", "exit 1")
        result = self.invoke("--prereqs-only", skip_submodules=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Git submodule update failed", result.stderr)
        self.assertNotIn("Submodules verified", result.stdout)

    def test_unknown_option_fails(self):
        result = self.invoke("--invalid-option")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unknown option", result.stderr)


if __name__ == "__main__":
    unittest.main()
