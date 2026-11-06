"""Opt-in real build VM tests: set CRO_BIN and CRO_BUILD_TEST_ROOTFS.

The rootfs must be a trusted native-architecture Linux rootfs with /bin/sh and
/bin/busybox. The CLI and its sibling microvm-runner must use native libkrun.
No source rootfs files are modified; builds clone into a temporary cache.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import unittest


@unittest.skipUnless(os.environ.get("CRO_BIN") and os.environ.get("CRO_BUILD_TEST_ROOTFS"),
                     "requires CRO_BIN, CRO_BUILD_TEST_ROOTFS, and working native virtualization")
class NativeBuildVmTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="cro-native-build-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.context = self.base / "context"
        self.context.mkdir()
        self.cache = self.base / "cache"
        self.cache.mkdir()
        self.source = Path(os.environ["CRO_BUILD_TEST_ROOTFS"]).resolve()
        self.cli = Path(os.environ["CRO_BIN"]).resolve()
        self.config = self.base / "config.json"
        self.config.write_text(json.dumps({"entrypoint": [], "cmd": [], "env": ["PATH=/bin:/usr/bin"],
                                           "working_dir": "/", "user": None}))
        self.registry = self.cache / "local_images.json"
        self.registry.write_text(json.dumps({"images": {"test-base:latest": {
            "tag": "test-base:latest", "digest": "sha256:" + "a" * 64,
            "rootfs_path": str(self.source), "config_path": str(self.config), "created_at": 0,
        }}}))

    def build(self, dockerfile, tag):
        (self.context / "Dockerfile").write_text(dockerfile)
        command = [str(self.cli), "build", "--data-dir", str(self.cache), "-t", tag, str(self.context)]
        with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                              start_new_session=True,
                              env={**os.environ, "CRO_HOST_ONLY_TEST_VALUE": "host-secret-sentinel"}) as process:
            try:
                stdout, stderr = process.communicate(timeout=45)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate()
                self.fail("native build exceeded 45 seconds")
            return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)

    def test_shell_and_exec_runs_are_isolated_and_persist_results(self):
        host_file = self.base / "host-sentinel"
        host_file.write_text("HOST_DATA")
        command = ["/bin/sh", "-c", 'test "$(/bin/busybox cat /app/result)" = first && printf second >> /app/result']
        result = self.build(
            'FROM test-base:latest\nWORKDIR /app\nENV EXPLICIT_VALUE=guest\n'
            f"RUN test ! -e '{host_file}' && test -z \"$CRO_HOST_ONLY_TEST_VALUE\" && "
            'test "$EXPLICIT_VALUE" = guest && test "$PWD" = /app && printf first > /app/result\n'
            + "RUN " + json.dumps(command) + "\n", "isolated:latest")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        record = json.loads(self.registry.read_text())["images"]["isolated:latest"]
        self.assertEqual((Path(record["rootfs_path"]) / "app/result").read_text(), "firstsecond")
        self.assertEqual(host_file.read_text(), "HOST_DATA")
        self.assertEqual(list((self.cache / "staging").iterdir()), [])
        self.assertEqual(list((self.cache / "build-runs").iterdir()), [])

    def test_distinct_run_outputs_keep_tags_independent(self):
        first = self.build("FROM test-base:latest\nRUN printf first > /result\n", "first:latest")
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        second = self.build("FROM test-base:latest\nRUN printf second > /result\n", "second:latest")
        self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
        images = json.loads(self.registry.read_text())["images"]
        self.assertNotEqual(images["first:latest"]["digest"], images["second:latest"]["digest"])
        self.assertEqual((Path(images["first:latest"]["rootfs_path"]) / "result").read_text(), "first")
        self.assertEqual((Path(images["second:latest"]["rootfs_path"]) / "result").read_text(), "second")

    def test_guest_exit_failure_cannot_publish_an_image(self):
        result = self.build("FROM test-base:latest\nRUN exit 7\n", "must-not-exist:latest")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("isolated build VM", result.stderr)
        self.assertNotIn("must-not-exist:latest", json.loads(self.registry.read_text())["images"])
        self.assertEqual(list((self.cache / "staging").iterdir()), [])
        self.assertEqual(list((self.cache / "build-runs").iterdir()), [])


if __name__ == "__main__":
    unittest.main()
