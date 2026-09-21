"""Opt-in integration test. Set ORION_TEST_SERVER to a built orion-server executable."""

import hashlib
import http.client
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from urllib.parse import urlsplit
from uuid import uuid4


@unittest.skipUnless(os.environ.get("ORION_TEST_SERVER"), "Set ORION_TEST_SERVER to test a real isolated backend")
class LiveServerTests(unittest.TestCase):
    def test_scan_queries_and_retry_with_literal_paths(self):
        executable = Path(os.environ["ORION_TEST_SERVER"]).resolve(strict=True)
        helper = Path(__file__).resolve().parents[1] / "scripts/orion.py"
        with tempfile.TemporaryDirectory(prefix="orion-skill-test-") as temporary:
            root = Path(temporary).resolve()
            runtime, data = root / "runtime", root / "审计 样本 [1]"
            runtime.mkdir()
            data.mkdir()
            (data / "项目" / "target").mkdir(parents=True)
            (data / "下载").mkdir()
            (data / "项目" / "Cargo.toml").write_text('[package]\nname="fixture"\nversion="0.1.0"\n', encoding="utf-8")
            for relative, size in [("项目/target/build.bin", 4 * 1024 * 1024), ("下载/安装包.iso", 2 * 1024 * 1024)]:
                with (data / relative).open("wb") as stream:
                    stream.truncate(size)
            for index in range(25):
                (data / f"资料 [{index:02}].txt").write_text("仅用于测试\n" * (index + 1), encoding="utf-8")

            def fingerprints():
                return {str(path.relative_to(data)): hashlib.sha256(path.read_bytes()).hexdigest()
                        for path in data.rglob("*") if path.is_file()}

            original = fingerprints()
            expected_size = sum(path.stat().st_size for path in data.rglob("*") if path.is_file())
            connection_file = runtime / "connection.json"
            environment = {**os.environ, "ORION_PORT": "0", "ORION_RUNTIME_DIR": str(runtime),
                           "ORION_CONNECTION_FILE": str(connection_file), "ORION_SCAN_WORKERS": "2"}
            with (root / "server.log").open("wb") as log:
                process = subprocess.Popen([str(executable)], cwd=root, env=environment, stdout=log, stderr=log,
                                           creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                connection = None
                try:
                    deadline = time.monotonic() + 10
                    while not connection_file.exists() and time.monotonic() < deadline:
                        if process.poll() is not None:
                            self.fail("Test server exited before writing its connection file")
                        time.sleep(0.05)
                    connection = json.loads(connection_file.read_text(encoding="utf-8"))

                    def run(*arguments, expected_code=0):
                        completed = subprocess.run([sys.executable, "-B", "-X", "utf8", str(helper),
                                                    "--connection-file", str(connection_file), *arguments],
                                                   cwd=root, capture_output=True, timeout=15)
                        stdout = completed.stdout.decode("utf-8")
                        self.assertNotIn(connection["token"], stdout + completed.stderr.decode("utf-8"))
                        self.assertEqual(completed.returncode, expected_code, stdout)
                        return json.loads(stdout)

                    self.assertEqual(run("status")["data"]["tasks"], [])
                    request_id = str(uuid4())
                    scan = run("scan", "--root", str(data), "--request-id", request_id)
                    task_id = scan["data"]["id"]
                    completed = run("wait", "--task", task_id, "--seconds", "5")["data"]
                    self.assertTrue(completed["finished"])
                    self.assertEqual(completed["task"]["status"], "completed")
                    self.assertTrue(completed["task"]["complete"])
                    self.assertEqual(completed["task"]["logical_bytes"], expected_size)
                    self.assertEqual(completed["task"]["files"], len(original))
                    self.assertEqual(run("task", "--task", task_id)["data"]["id"], task_id)

                    blocked = run("scan", "--root", str(data), expected_code=1)
                    self.assertEqual(blocked["error"]["code"], "existing_scan")
                    retried = run("--instance-id", connection["instance_id"], "scan", "--root", str(data),
                                  "--request-id", request_id, "--replace-existing")
                    self.assertEqual(retried["data"]["id"], task_id)

                    first = run("entries", "--task", task_id, "--limit", "20")["data"]
                    second = run("entries", "--task", task_id, "--offset", "20", "--limit", "20",
                                 "--revision", str(first["revision"]))["data"]
                    self.assertEqual(len(first["entries"]), 20)
                    self.assertEqual(len(first["entries"]) + len(second["entries"]), first["total"])
                    file_entry = next(entry for entry in first["entries"] if entry["kind"] == "file")
                    detail = run("detail", "--task", task_id, "--entry", str(file_entry["id"]))["data"]
                    self.assertTrue(Path(detail["path"]).is_file())
                    self.assertIn("[", detail["name"])

                    overview = run("tree", "--task", task_id, "--depth", "3", "--top", "1")["data"]["root"]
                    self.assertEqual(overview["logical_bytes"], expected_size)
                    self.assertEqual(len(overview["children"]), 1)
                    self.assertEqual(overview["children"][0]["logical_bytes"] + overview["omitted_bytes"], expected_size)
                    self.assertEqual(fingerprints(), original)
                finally:
                    if connection and process.poll() is None:
                        client = http.client.HTTPConnection("127.0.0.1", urlsplit(connection["url"]).port, timeout=5)
                        try:
                            client.request("POST", "/api/v1/server/shutdown", json.dumps({"instance_id": connection["instance_id"], "cancel_active": True}),
                                           {"Authorization": "Bearer " + connection["token"], "Content-Type": "application/json"})
                            client.getresponse().read()
                            process.wait(timeout=5)
                        except (OSError, http.client.HTTPException, subprocess.TimeoutExpired):
                            pass
                        finally:
                            client.close()
                    if process.poll() is None:
                        process.terminate()  # Only the process created by this test.
                        process.wait(timeout=5)
                    self.assertEqual(root.parent, Path(tempfile.gettempdir()).resolve())


if __name__ == "__main__":
    unittest.main()
