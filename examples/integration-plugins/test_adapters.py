"""Offline protocol regression: only owned loopback servers and temp SQLite."""
from __future__ import annotations
from contextlib import contextmanager
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request

from github_issues import GithubIssues
from webhook_chat import WebhookChat, signature
from protocol import read_frame

@contextmanager
def server(handler):
    instance = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=instance.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{instance.server_port}"
    finally:
        instance.shutdown()
        instance.server_close()
        thread.join(2)

def iso(ms):
    return datetime.fromtimestamp(ms / 1000, timezone.utc).isoformat().replace("+00:00", "Z")

class GitHubContract(unittest.TestCase):
    def test_permission_denial_is_not_hidden_as_an_empty_successful_poll(self):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args): pass
            def do_GET(self):
                raw = b'{"message":"Resource not accessible by integration"}'
                self.send_response(403)
                self.send_header("X-RateLimit-Remaining", "4000")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)
        with server(Handler) as base:
            adapter = GithubIssues()
            adapter.initialize({"api_base": base, "allow_loopback_http": True, "token_env": "AGENA_TEST_UNUSED_CREDENTIAL"})
            request = {"binding_id": "fixture", "config": {"repository": "owner/repo"}, "cursor": None, "limit": 32, "now_ms": 1000, "not_before_ms": 1}
            with self.assertRaisesRegex(ValueError, "403"):
                adapter.poll(request)

    def test_pagination_excludes_pull_requests_and_conditional_fetch_keeps_checkpoint(self):
        now = 1780000000000
        requests = []
        def issue(number, created):
            return {"number": number, "created_at": iso(created), "title": f"Issue {number}", "body": "body", "user": {"login": "author", "type": "User"}, "labels": ["triage"]}
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args): pass
            def do_GET(self):
                requests.append((self.path, dict(self.headers)))
                if self.headers.get("If-None-Match") == '"fixture-head"':
                    self.send_response(304); self.end_headers(); return
                page = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query).get("page", ["1"])[0]
                self.send_response(200)
                self.send_header("ETag", '"fixture-head"')
                if page == "1":
                    self.send_header("Link", f'<http://127.0.0.1:{self.server.server_port}/repos/owner/repo/issues?page=2>; rel="next"')
                    pr = issue(9, now + 2000); pr["pull_request"] = {}
                    body = [issue(10, now + 3000), pr]
                else:
                    body = [issue(8, now + 1000), issue(7, now - 5000)]
                raw = json.dumps(body).encode(); self.send_header("Content-Length", str(len(raw))); self.end_headers(); self.wfile.write(raw)
        with server(Handler) as base:
            adapter = GithubIssues(); adapter.initialize({"api_base": base, "allow_loopback_http": True, "token_env": "AGENA_TEST_UNUSED_CREDENTIAL"})
            request = {"binding_id": "fixture", "config": {"repository": "owner/repo", "labels": ["triage"]}, "cursor": None, "limit": 2, "now_ms": now + 5000, "not_before_ms": now}
            first = adapter.poll(request)
            self.assertEqual([e["id"] for e in first["events"]], ["github:owner/repo:10:opened"])
            self.assertIn("next", first["cursor"])
            request["cursor"] = first["cursor"]
            second = adapter.poll(request)
            self.assertEqual([e["id"] for e in second["events"]], ["github:owner/repo:8:opened"])
            self.assertNotIn("next", second["cursor"])
            self.assertEqual(second["cursor"]["watermark_ms"], now + 3000)
            request["cursor"] = second["cursor"]
            third = adapter.poll(request)
            self.assertEqual(third["events"], [])
            self.assertEqual(third["cursor"], second["cursor"])
            self.assertEqual(requests[-1][1]["If-None-Match"], '"fixture-head"')
            self.assertEqual({k.lower(): v for k, v in requests[0][1].items()}["x-github-api-version"], "2026-03-10")

    def test_rate_limit_does_not_advance_cursor_or_fetch_other_origins(self):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args): pass
            def do_GET(self):
                self.send_response(429); self.send_header("Retry-After", "91"); self.end_headers()
        with server(Handler) as base:
            adapter = GithubIssues(); adapter.initialize({"api_base": base, "allow_loopback_http": True, "token_env": "AGENA_TEST_UNUSED_CREDENTIAL"})
            request = {"binding_id": "fixture", "config": {"repository": "owner/repo"}, "cursor": {"repository": "owner/repo", "watermark_ms": 1}, "limit": 32, "now_ms": 1000, "not_before_ms": 1}
            result = adapter.poll(request)
            self.assertEqual(result["retry_after_seconds"], 91)
            self.assertEqual(result["cursor"], request["cursor"])
            request["cursor"]["next"] = "https://example.invalid/steal-token"
            with self.assertRaises(ValueError): adapter.poll(request)

IDENTITY={"tenant":"fixture-tenant","account":"fixture-bot","kind":"group"}
def channel_context():
    return {"identity":dict(IDENTITY),"actor_verified":True,"is_member":True,"addressed_to_bot":True,"verified_at_ms":int(time.time()*1000)}

class ChatContract(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="agena-chat-adapter-test-")
        self.variable = "AGENA_CHAT_ADAPTER_TEST_SECRET"
        self.previous = os.environ.get(self.variable)
        os.environ[self.variable] = "a-local-fixture-secret-at-least-32-bytes"
        self.adapter = WebhookChat()
        self.adapter.initialize({"secret_env": self.variable, "database_path": str(Path(self.directory.name) / "bridge.db"), "listen_port": 0, "spool_only": True})
        self.endpoint = f"http://127.0.0.1:{self.adapter.server.server_port}/messages"
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def tearDown(self):
        self.adapter.close()
        self.directory.cleanup()
        if self.previous is None: os.environ.pop(self.variable, None)
        else: os.environ[self.variable] = self.previous
    def post(self, event, age=0, signed=True):
        body = json.dumps(event).encode(); timestamp = str(int(time.time() * 1000) - age)
        sig = signature(self.adapter.secret, timestamp, body) if signed else "sha256=bad"
        request = urllib.request.Request(self.endpoint, data=body, headers={"X-Agena-Timestamp": timestamp, "X-Agena-Signature": sig}, method="POST")
        try:
            with self.http.open(request, timeout=5) as r: return r.status, json.load(r)
        except urllib.error.HTTPError as r:
            with r: return r.code, json.load(r)
    def event(self, identity="one", text="hello", resource="account:room"):
        return {"id": identity, "event_type": "message", "resource": resource, "actor": "operator", "text": text, "occurred_at_ms": int(time.time() * 1000), "channel":channel_context()}
    def test_signature_replay_content_conflict_and_poison_message_rejection(self):
        event = self.event()
        self.assertEqual(self.post(event, signed=False)[0], 401)
        self.assertEqual(self.post(event, age=301000)[0], 401)
        self.assertEqual(self.post(event)[0], 202)
        self.assertTrue(self.post(event)[1]["replayed"])
        self.assertEqual(self.post({**event, "text": "changed"})[0], 409)
        self.assertEqual(self.post({**self.event("bad"), "thread": {"bad": True}})[0], 400)
        self.assertEqual(self.post({**self.event("empty"), "text": "  "})[0], 400)
        self.assertEqual(self.post({**self.event("bot"), "bot": "false"})[0], 400)
        request = {"config": {"conversation": "account:room"}, "cursor": None, "limit": 32,"channel_identity":dict(IDENTITY)}
        batch = self.adapter.poll(request)
        self.assertEqual(len(batch["events"]), 1)
        self.assertEqual(self.adapter.poll({**request, "cursor": batch["cursor"]})["events"], [])
    def test_namespaced_identity_prevents_message_id_collisions_and_cursor_reuse(self):
        first = self.event("same-id", "group one")
        second = self.event("same-id", "group two")
        second["channel"]["identity"]["tenant"] = "other-tenant"
        self.assertEqual(self.post(first)[0], 202)
        self.assertEqual(self.post(second)[0], 202)
        request = {"config":{"conversation":"account:room"},"cursor":None,"limit":32,"channel_identity":dict(IDENTITY)}
        one = self.adapter.poll(request)
        self.assertEqual([e["text"] for e in one["events"]], ["group one"])
        other = {**IDENTITY,"tenant":"other-tenant"}
        two = self.adapter.poll({**request,"channel_identity":other})
        self.assertEqual([e["text"] for e in two["events"]], ["group two"])
        with self.assertRaises(ValueError):
            self.adapter.poll({**request,"channel_identity":other,"cursor":one["cursor"]})
        forged=self.event("reserved")
        forged["actor"]="@service"
        self.assertEqual(self.post(forged)[0],400)
        legacy=self.event("legacy");del legacy["channel"]
        self.assertEqual(self.post(legacy)[0],400)

    def test_signed_platform_removal_requires_negative_verified_membership(self):
        removed = self.event("platform-removed", "verified platform membership removal")
        removed["event_type"] = "membership.revoked"
        self.assertEqual(self.post(removed)[0], 400)
        removed["channel"]["is_member"] = False
        removed["channel"]["actor_verified"] = False
        self.assertEqual(self.post(removed)[0], 400)
        removed["channel"]["actor_verified"] = True
        removed["channel"]["addressed_to_bot"] = False
        self.assertEqual(self.post(removed, signed=False)[0], 401)
        self.assertEqual(self.post(removed)[0], 202)
        self.assertTrue(self.post(removed)[1]["replayed"])
        batch = self.adapter.poll({"config": {"conversation": "account:room"}, "cursor": None,
                                   "limit": 32, "channel_identity": dict(IDENTITY)})
        self.assertEqual(len(batch["events"]), 1)
        self.assertEqual(batch["events"][0]["event_type"], "membership.revoked")
        self.assertFalse(batch["events"][0]["channel"]["is_member"])

    def test_channel_destination_is_pinned_and_send_receipts_are_idempotent(self):
        request = {"binding_id": "b", "config": {"conversation": "account:room"}, "resource": "account:room", "thread": None, "delivery_id": "stable-key", "text": "reply", "channel":dict(IDENTITY)}
        first = self.adapter.send(request)
        self.assertEqual(self.adapter.send(request), first)
        self.assertEqual(first["remote_id"], "spool:stable-key")
        with self.assertRaises(ValueError): self.adapter.send({**request, "resource": "another:room"})
        with self.assertRaises(ValueError): self.adapter.send({**request, "text": "changed receipt content"})
        with self.adapter.database() as db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM outgoing").fetchone()[0], 1)

class ProtocolContract(unittest.TestCase):
    def test_reference_plugin_uses_sdk_generated_manifest_and_real_content_length_rpc(self):
        requests = [
            {"jsonrpc": "2.0", "id": 1, "method": "meta/manifest", "params": {}},
            {"jsonrpc": "2.0", "id": 2, "method": "meta/init", "params": {"protocol_version": 3, "settings": {}}},
            {"jsonrpc": "2.0", "id": 3, "method": "service/invoke", "params": {"service": "agena.integration", "api_version": 1, "method": "describe", "input": {}}},
            {"jsonrpc": "2.0", "id": 4, "method": "meta/shutdown", "params": {}},
        ]
        frames = b""
        for request in requests:
            body = json.dumps(request).encode(); frames += f"Content-Length: {len(body)}\r\n\r\n".encode() + body
        result = subprocess.run([os.sys.executable, str(Path(__file__).with_name("github_issues.py"))], input=frames, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        stream = io.BytesIO(result.stdout)
        responses = [read_frame(stream) for _ in requests]
        self.assertTrue(all("result" in r for r in responses), responses)
        self.assertEqual(responses[0]["result"]["services"]["exports"][0]["id"], "agena.integration")
        self.assertEqual(responses[2]["result"]["kind"], "event_source")

if __name__ == "__main__":
    unittest.main()
