"""Run: python -m unittest discover -s skills/orion-space-audit/tests -v."""

from contextlib import redirect_stdout
import importlib.util
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import socket
import tempfile
import threading
import unittest
from unittest.mock import patch
from urllib.parse import urlsplit, parse_qs
from uuid import uuid4


spec = importlib.util.spec_from_file_location("orion_audit", Path(__file__).resolve().parents[1] / "scripts/orion.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def node(identifier, size, children=None):
    return {"id": identifier, "parent_id": None if identifier == 0 else 0, "name": f"文件-{identifier}",
            "kind": "directory" if children is not None else "file", "logical_bytes": size,
            "child_count": len(children or []), "expanded": children is not None, "zero_count": 0,
            "omitted_count": 0, "omitted_bytes": 0, "children": children or []}


class ClientTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.instance, self.task_id = str(uuid4()), str(uuid4())
        self.token = 'test-"credential'
        self.calls, self.retained, self.overrides = [], [], {}
        self.health = {"name": "orion-server", "api_version": 1, "instance_id": self.instance, "capabilities": ["treemap"]}
        self.task = {"id": self.task_id, "root": str(self.root), "status": "completed", "complete": True}
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def respond(self):
                raw_body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                body = json.loads(raw_body) if raw_body else None
                owner.calls.append((self.command, self.path, body, self.headers.get("Authorization")))
                path = urlsplit(self.path).path
                if path in owner.overrides:
                    action = owner.overrides[path]
                    if action == "disconnect":
                        self.connection.shutdown(socket.SHUT_RDWR)
                        self.connection.close()
                        return
                    status, data, headers = action
                elif path == "/api/v1/health":
                    status, data, headers = 200, owner.health, {}
                elif path == "/api/v1/tasks":
                    status, data, headers = 200, owner.retained, {}
                elif path == "/api/v1/scans" or path == f"/api/v1/tasks/{owner.task_id}":
                    status, data, headers = 200, owner.task, {}
                elif path.endswith("/treemap"):
                    root = node(0, 75, [node(1, 40), node(2, 20), node(3, 10)])
                    root.update(omitted_bytes=5, omitted_count=1, zero_count=1, child_count=5)
                    status, data, headers = 200, {"revision": 7, "depth": 2, "root": root}, {}
                elif path.endswith("/entries"):
                    query = parse_qs(urlsplit(self.path).query)
                    status, data, headers = 200, {"revision": 7, "total": 40, "offset": int(query["offset"][0]), "entries": [node(1, 40)]}, {}
                else:
                    status, data, headers = 200, {**node(1, 40), "path": str(owner.root / "空 格 [1].bin")}, {}
                raw = data if isinstance(data, bytes) else json.dumps(data).encode("utf-8")
                self.send_response(status)
                for name, value in headers.items():
                    self.send_header(name, value)
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            do_GET = respond
            do_POST = respond

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.worker = threading.Thread(target=lambda: self.server.serve_forever(poll_interval=0.01), daemon=True)
        self.worker.start()
        self.addCleanup(self.stop_server)
        self.connection_file = self.root / "connection.json"
        self.document = {"url": f"http://127.0.0.1:{self.server.server_port}", "token": self.token, "instance_id": self.instance}
        self.write_connection()

    def stop_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.worker.join(timeout=2)

    def write_connection(self):
        self.connection_file.write_text(json.dumps(self.document), encoding="utf-8")

    def run_cli(self, *args):
        output = io.StringIO()
        with redirect_stdout(output):
            code = audit.main(["--connection-file", str(self.connection_file), *args])
        return code, json.loads(output.getvalue())

    def test_status_ignores_proxy_environment_and_does_not_expose_token(self):
        with patch.dict(os.environ, {"HTTP_PROXY": "http://127.0.0.1:1", "http_proxy": "http://127.0.0.1:1"}):
            code, result = self.run_cli("status")
        self.assertEqual(code, 0)
        self.assertEqual(result["server"]["instance_id"], self.instance)
        self.assertNotIn("token", result["server"])
        self.assertTrue(all(call[3] == "Bearer " + self.token for call in self.calls))
        self.assertEqual([call[0] for call in self.calls], ["GET", "GET"])

    def test_external_or_credential_bearing_urls_are_rejected_before_requests(self):
        for url in ("https://example.com", "http://127.0.0.1.evil:1234", "http://user@127.0.0.1:1234",
                    "http://127.0.0.1:1234/path", "http://127.0.0.1:1234/?token=x"):
            with self.subTest(url=url):
                self.document["url"] = url
                self.write_connection()
                code, result = self.run_cli("status")
                self.assertEqual(code, 1)
                self.assertEqual(result["error"]["code"], "invalid_connection")
        self.assertEqual(self.calls, [])

    def test_redirect_is_not_followed(self):
        self.overrides["/api/v1/health"] = (302, b"", {"Location": self.document["url"] + "/credential-leak"})
        code, result = self.run_cli("status")
        self.assertEqual(code, 1)
        self.assertEqual(result["error"]["http_status"], 302)
        self.assertEqual(len(self.calls), 1)

    def test_instance_changes_stop_before_query_or_scan(self):
        code, result = self.run_cli("--instance-id", str(uuid4()), "scan", "--root", str(self.root))
        self.assertEqual(result["error"]["code"], "instance_changed")
        self.assertEqual(self.calls, [])
        self.health["instance_id"] = str(uuid4())
        code, result = self.run_cli("status")
        self.assertEqual(code, 1)
        self.assertEqual(result["error"]["code"], "instance_changed")
        self.assertEqual(len(self.calls), 1)

    def test_existing_scan_requires_explicit_replacement(self):
        self.retained = [self.task]
        code, result = self.run_cli("scan", "--root", str(self.root))
        self.assertEqual(code, 1)
        self.assertEqual(result["error"]["code"], "existing_scan")
        self.assertFalse(any(call[0] == "POST" for call in self.calls))
        code, result = self.run_cli("scan", "--root", str(self.root), "--replace-existing")
        self.assertEqual(code, 0)
        self.assertEqual(result["data"]["id"], self.task_id)

    def test_lost_scan_response_preserves_request_identity_for_explicit_retry(self):
        self.overrides["/api/v1/scans"] = "disconnect"
        code, lost = self.run_cli("scan", "--root", str(self.root))
        self.assertEqual(code, 1)
        self.assertEqual(lost["server"]["instance_id"], self.instance)
        del self.overrides["/api/v1/scans"]
        self.retained = [self.task]
        code, retry = self.run_cli("--instance-id", self.instance, "scan", "--root", lost["root"],
                                   "--request-id", lost["request_id"], "--replace-existing")
        self.assertEqual(code, 0)
        posts = [call[2] for call in self.calls if call[0] == "POST"]
        self.assertEqual(len(posts), 2)
        self.assertEqual(posts[0], posts[1])
        self.assertEqual(retry["request_id"], lost["request_id"])

    def test_paging_passes_revision_and_preserves_stale_error_without_retry(self):
        code, result = self.run_cli("entries", "--task", self.task_id, "--offset", "20", "--revision", "7")
        self.assertEqual(code, 0)
        self.assertEqual(result["data"]["offset"], 20)
        self.assertEqual(parse_qs(urlsplit(self.calls[-1][1]).query)["revision"], ["7"])
        self.overrides[f"/api/v1/scans/{self.task_id}/entries"] = (409, {"code": "stale_revision", "message": "changed"}, {})
        previous = len(self.calls)
        code, result = self.run_cli("entries", "--task", self.task_id, "--revision", "7")
        self.assertEqual(code, 1)
        self.assertEqual(result["error"]["code"], "stale_revision")
        self.assertEqual(len(self.calls), previous + 2)

    def test_tree_top_limit_preserves_aggregate_sizes_and_zero_count(self):
        code, result = self.run_cli("tree", "--task", self.task_id, "--top", "1")
        self.assertEqual(code, 0)
        root = result["data"]["root"]
        self.assertEqual(root["children"][0]["id"], 1)
        self.assertEqual(root["omitted_count"], 3)
        self.assertEqual(root["omitted_bytes"], 35)
        self.assertEqual(root["zero_count"], 1)
        self.assertEqual(sum(child["logical_bytes"] for child in root["children"]) + root["omitted_bytes"], root["logical_bytes"])

    def test_missing_treemap_capability_has_a_clear_fallback(self):
        self.health["capabilities"] = []
        code, result = self.run_cli("tree", "--task", self.task_id)
        self.assertEqual(code, 1)
        self.assertEqual(result["error"]["code"], "treemap_unsupported")
        self.assertEqual(len(self.calls), 1)

    def test_wait_timeout_keeps_running_task_and_does_not_cancel(self):
        self.task.update(status="running", complete=False)
        with patch.object(audit.time, "monotonic", side_effect=[0, 0, 1, 2]), patch.object(audit.time, "sleep"):
            code, result = self.run_cli("wait", "--task", self.task_id, "--seconds", "1")
        self.assertEqual(code, 0)
        self.assertFalse(result["data"]["finished"])
        self.assertEqual(result["data"]["task"]["status"], "running")
        self.assertTrue(all(call[0] == "GET" for call in self.calls))

    def test_wait_preserves_failed_scan_and_coverage_instead_of_claiming_success(self):
        self.task.update(status="failed", complete=False, issue_count=1)
        code, result = self.run_cli("wait", "--task", self.task_id)
        self.assertEqual(code, 0)
        self.assertTrue(result["data"]["finished"])
        self.assertEqual(result["data"]["task"]["status"], "failed")
        self.assertFalse(result["data"]["task"]["complete"])

    def test_non_json_error_and_echoed_credential_remain_safe_json(self):
        self.overrides["/api/v1/tasks"] = (503, b"unavailable", {})
        code, result = self.run_cli("status")
        self.assertEqual(code, 1)
        self.assertEqual(result["error"]["http_status"], 503)
        self.overrides["/api/v1/tasks"] = (401, {"code": "unauthorized", "message": self.token}, {})
        code, result = self.run_cli("status")
        self.assertEqual(result["error"]["message"], "[redacted]")

    def test_detail_preserves_unicode_spaces_and_literal_path_characters(self):
        code, result = self.run_cli("detail", "--task", self.task_id, "--entry", "1")
        self.assertEqual(code, 0)
        self.assertEqual(result["data"]["path"], str(self.root / "空 格 [1].bin"))

    def test_connection_discovery_precedence_and_no_silent_fallback(self):
        desktop = self.root / "dev.orion.desktop/runtime/connection.json"
        desktop.parent.mkdir(parents=True)
        desktop.write_text("invalid-json", encoding="utf-8")
        with patch.dict(os.environ, {"LOCALAPPDATA": str(self.root), "ORION_CONNECTION_FILE": str(self.connection_file)}):
            self.assertEqual(audit.connection_path(), self.connection_file)
            self.assertEqual(audit.connection_path("explicit.json"), Path("explicit.json"))
        with patch.dict(os.environ, {"LOCALAPPDATA": str(self.root), "ORION_CONNECTION_FILE": ""}):
            self.assertEqual(audit.connection_path(), desktop)
            with self.assertRaises(audit.AuditError):
                audit.Client(audit.connection_path())


if __name__ == "__main__":
    unittest.main()
