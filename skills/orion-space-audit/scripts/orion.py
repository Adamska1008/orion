#!/usr/bin/env python3
"""Small, dependency-free client for Orion space audits (Python 3.10+)."""

import argparse
import http.client
import json
import os
from pathlib import Path
import sys
import time
from urllib.parse import urlencode, urlsplit
from uuid import UUID, uuid4


class AuditError(Exception):
    def __init__(self, code, message, **details):
        super().__init__(message)
        self.error = {"code": code, "message": message, **details}


def connection_path(explicit=None):
    if explicit is not None:
        return Path(explicit)
    configured = os.environ.get("ORION_CONNECTION_FILE")
    if configured:
        return Path(configured)
    candidates = []
    if os.environ.get("LOCALAPPDATA"):
        candidates.append(Path(os.environ["LOCALAPPDATA"]) / "dev.orion.desktop/runtime/connection.json")
    candidates.append(Path.cwd() / ".orion/connection-43120.json")
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise AuditError("connection_missing", "No Orion connection file found. Start Orion or specify --connection-file.")


class Client:
    def __init__(self, path, timeout=5):
        try:
            with Path(path).open("r", encoding="utf-8-sig") as stream:
                document = json.loads(stream.read(65537))
        except OSError:
            raise AuditError("connection_missing", "Cannot read the selected Orion connection file.") from None
        except (ValueError, UnicodeError):
            raise AuditError("invalid_connection", "The selected connection file is not valid JSON.") from None
        try:
            url, token, instance = (document[key] for key in ("url", "token", "instance_id"))
            if not all(isinstance(value, str) and value for value in (url, token, instance)):
                raise ValueError()
            parsed = urlsplit(url)
            if (parsed.scheme != "http" or parsed.hostname != "127.0.0.1"
                    or parsed.port is None or not 1 <= parsed.port <= 65535
                    or parsed.username is not None or parsed.password is not None
                    or parsed.path not in ("", "/") or parsed.query or parsed.fragment
                    or any(ord(char) <= 32 for char in url)
                    or any(ord(char) < 33 or ord(char) > 126 for char in token)):
                raise ValueError()
            UUID(instance)
        except (KeyError, TypeError, ValueError, AttributeError):
            raise AuditError("invalid_connection", "Expected a loopback HTTP URL with a port, token and instance ID.") from None
        self.port, self.token, self.instance_id = parsed.port, token, instance
        self.url = f"http://127.0.0.1:{self.port}"
        self.timeout = timeout

    @property
    def server(self):
        return {"url": self.url, "instance_id": self.instance_id}

    def request(self, method, path, *, body=None, query=None, timeout=None):
        # http.client uses neither environment proxies nor automatic redirects.
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout or self.timeout)
        target = "/api/v1" + path
        if query:
            target += "?" + urlencode({key: value for key, value in query.items() if value is not None})
        headers = {"Authorization": "Bearer " + self.token, "Accept": "application/json"}
        payload = None
        if body is not None:
            headers["Content-Type"] = "application/json"
            payload = json.dumps(body, ensure_ascii=False).encode("utf-8")
        try:
            connection.request(method, target, body=payload, headers=headers)
            response = connection.getresponse()
            raw = response.read(8 * 1024 * 1024 + 1)
            if len(raw) > 8 * 1024 * 1024:
                raise AuditError("invalid_response", "Orion response exceeded the size limit.")
            try:
                data = json.loads(raw)
            except (ValueError, UnicodeError):
                data = None
            if not 200 <= response.status < 300:
                error = data if isinstance(data, dict) else {}
                code = error.get("code") if isinstance(error.get("code"), str) else "http_error"
                message = error.get("message") if isinstance(error.get("message"), str) else "Orion rejected the request."
                raise AuditError(code, message[:500], http_status=response.status, task_id=error.get("task_id"))
            if not isinstance(data, (dict, list)):
                raise AuditError("invalid_response", "Orion did not return a JSON object or array.")
            return data
        except (OSError, http.client.HTTPException):
            raise AuditError("unreachable", "The request could not be completed. Check Orion and the selected connection file.") from None
        finally:
            connection.close()

    def health(self, expected_instance=None):
        if expected_instance is not None and expected_instance != self.instance_id:
            raise AuditError("instance_changed", "The connection file points to a different server instance. Run status again.")
        health = self.request("GET", "/health")
        if not isinstance(health, dict) or health.get("name") != "orion-server" or health.get("api_version") != 1:
            raise AuditError("incompatible_server", "Expected orion-server with API version 1.")
        if health.get("instance_id") != self.instance_id:
            raise AuditError("instance_changed", "Server instance does not match the connection file.")
        return health

    def tasks(self):
        tasks = self.request("GET", "/tasks")
        if not isinstance(tasks, list) or any(not isinstance(task, dict) for task in tasks):
            raise AuditError("invalid_response", "Orion returned an invalid task list.")
        return tasks

    def task(self, task_id, timeout=None):
        task = self.request("GET", f"/tasks/{task_id}", timeout=timeout)
        if not isinstance(task, dict) or task.get("id") != task_id or task.get("status") not in (
                "running", "cancelling", "cancelled", "completed", "failed"):
            raise AuditError("invalid_response", "Unknown task identity or state; inspect status before continuing.")
        return task


def compact_tree(node, top):
    keys = ("id", "parent_id", "name", "kind", "logical_bytes", "child_count", "expanded",
            "zero_count", "omitted_count", "omitted_bytes")
    result = {key: node[key] for key in keys}
    children = node["children"]
    result["children"] = [compact_tree(child, top) for child in children[:top]]
    result["omitted_count"] += len(children[top:])
    result["omitted_bytes"] += sum(child["logical_bytes"] for child in children[top:])
    return result


def execute(client, args, health):
    if args.command == "status":
        return {"health": health, "tasks": client.tasks()}
    if args.command == "scan":
        tasks = client.tasks()
        if tasks and not args.replace_existing:
            raise AuditError("existing_scan", "Reuse the retained scan, or explicitly pass --replace-existing to request another.", tasks=tasks)
        return client.request("POST", "/scans", body={"root": args.root, "request_id": args.request_id})
    if args.command == "task":
        return client.task(args.task)
    if args.command == "wait":
        deadline = time.monotonic() + args.seconds
        task = None
        while (remaining := deadline - time.monotonic()) > 0:
            task = client.task(args.task, timeout=min(client.timeout, remaining))
            if task["status"] not in ("running", "cancelling"):
                return {"finished": True, "task": task}
            time.sleep(min(1, max(0, deadline - time.monotonic())))
        return {"finished": False, "task": task}
    if args.command == "detail":
        return client.request("GET", f"/scans/{args.task}/entries/{args.entry}")
    if args.command == "entries":
        return client.request("GET", f"/scans/{args.task}/entries", query={
            "parent": args.parent, "offset": args.offset, "limit": args.limit, "revision": args.revision})
    if "treemap" not in health.get("capabilities", []):
        raise AuditError("treemap_unsupported", "This server does not advertise treemap. Use entries and detail.")
    data = client.request("GET", f"/scans/{args.task}/treemap", query={"parent": args.parent, "depth": args.depth})
    try:
        return {"revision": data["revision"], "depth": data["depth"], "top_per_directory": args.top,
                "root": compact_tree(data["root"], args.top)}
    except (KeyError, TypeError, ValueError):
        raise AuditError("invalid_response", "Orion returned an invalid treemap.") from None


def bounded_number(low, high, kind=int):
    def parse(value):
        try:
            number = kind(value)
            if not low <= number <= high:
                raise ValueError()
            return number
        except ValueError:
            raise argparse.ArgumentTypeError(f"expected a number between {low} and {high}") from None
    return parse


def uuid_argument(value):
    try:
        return str(UUID(value))
    except ValueError:
        raise argparse.ArgumentTypeError("expected a UUID") from None


def absolute_directory(value):
    if not Path(value).is_absolute():
        raise argparse.ArgumentTypeError("root must be an absolute directory path")
    return value  # Preserve the exact root string for idempotent retries.


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--connection-file")
    result.add_argument("--instance-id", type=uuid_argument)
    result.add_argument("--timeout", type=bounded_number(0.1, 30, float), default=5)
    commands = result.add_subparsers(dest="command", required=True)
    commands.add_parser("status", help="Read service health and retained tasks")
    scan = commands.add_parser("scan", help="Request a new read-only scan")
    scan.add_argument("--root", type=absolute_directory, required=True)
    scan.add_argument("--request-id", type=uuid_argument)
    scan.add_argument("--replace-existing", action="store_true")
    for name in ("task", "wait", "entries", "detail", "tree"):
        command = commands.add_parser(name)
        command.add_argument("--task", type=uuid_argument, required=True)
        if name == "wait":
            command.add_argument("--seconds", type=bounded_number(1, 60, float), default=20)
        if name == "detail":
            command.add_argument("--entry", type=bounded_number(0, 2**64 - 1), required=True)
        if name in ("entries", "tree"):
            command.add_argument("--parent", type=bounded_number(0, 2**64 - 1), default=0)
        if name == "entries":
            command.add_argument("--offset", type=bounded_number(0, 2**64 - 1), default=0)
            command.add_argument("--limit", type=bounded_number(1, 500), default=20)
            command.add_argument("--revision", type=bounded_number(0, 2**64 - 1))
        if name == "tree":
            command.add_argument("--depth", type=bounded_number(1, 4), default=2)
            command.add_argument("--top", type=bounded_number(1, 64), default=8)
    return result


def main(argv=None):
    args = parser().parse_args(argv)
    if args.command == "scan" and args.request_id is None:
        args.request_id = str(uuid4())
    client = None
    verified = False
    try:
        client = Client(connection_path(args.connection_file), args.timeout)
        health = client.health(args.instance_id)
        verified = True
        output = {"ok": True, "server": client.server, "data": execute(client, args, health)}
    except AuditError as error:
        output = {"ok": False, "error": error.error}
        if verified:
            output["server"] = client.server
    if args.command == "scan":
        output.update(request_id=args.request_id, root=args.root)
    # Redact before JSON escaping, including unexpected echoes in response data.
    def redact(value):
        if isinstance(value, str):
            return value.replace(client.token, "[redacted]")
        if isinstance(value, list):
            return [redact(item) for item in value]
        if isinstance(value, dict):
            return {redact(key): redact(item) for key, item in value.items()}
        return value

    print(json.dumps(redact(output) if client else output, ensure_ascii=False))
    return 0 if output["ok"] else 1


if __name__ == "__main__":
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    sys.exit(main())
