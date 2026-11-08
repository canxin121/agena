#!/usr/bin/env python3
"""Reference chat-channel plugin: signed inbound HTTP + durable outbox.

A platform adapter/relay converts provider-specific messages to ExternalEvent
and authenticates the platform sender BEFORE signing them for this plugin.
No Agena administrator token is exposed to the external channel. A real remote
gateway is configured once in plugin settings, never selected by message text.

spool_only=True is an explicit local demonstration mode, not external delivery.
"""
from __future__ import annotations
from contextlib import contextmanager
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import sqlite3
import threading
import time
import unicodedata
from typing import Any
import urllib.error
import urllib.parse
import urllib.request
from protocol import serve

MAX_EVENT = 64 * 1024

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args: Any, **kwargs: Any) -> None:
        return None

def compact(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()

def signature(secret: bytes, timestamp: str, payload: bytes) -> str:
    return "sha256=" + hmac.new(secret, timestamp.encode() + b"\n" + payload, hashlib.sha256).hexdigest()

class WebhookChat:
    def initialize(self, settings: dict[str, Any]) -> None:
        self.settings = settings
        variable = str(settings.get("secret_env", "AGENA_CHAT_BRIDGE_SECRET"))
        if not re.fullmatch(r"[A-Z_][A-Z0-9_]{0,100}", variable):
            raise ValueError("invalid secret environment reference")
        self.secret = os.environ.get(variable, "").encode()
        if not 32 <= len(self.secret) <= 4096:
            raise ValueError("configure a random secret of at least 32 bytes in the referenced environment variable")
        path = Path(settings["database_path"]).expanduser()
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.is_symlink():
            raise ValueError("database path must not be a symlink")
        self.path = str(path.resolve())
        with self.database() as db:
            db.executescript("""
                PRAGMA journal_mode=WAL;
                CREATE TABLE IF NOT EXISTS inbox_v2(sequence INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL, account TEXT NOT NULL, kind TEXT NOT NULL, resource TEXT NOT NULL, event_id TEXT NOT NULL, digest TEXT NOT NULL, event_json TEXT NOT NULL, UNIQUE(tenant,account,kind,resource,event_id));
                CREATE TABLE IF NOT EXISTS outgoing(delivery_id TEXT PRIMARY KEY, digest TEXT NOT NULL, payload_json TEXT NOT NULL, state TEXT NOT NULL, remote_id TEXT);
            """)
        os.chmod(self.path, 0o600)
        self.spool = bool(settings.get("spool_only", False))
        self.remote_idempotent = bool(settings.get("remote_idempotent", False))
        self.reply_url = str(settings.get("reply_url", ""))
        if not self.spool:
            url = urllib.parse.urlsplit(self.reply_url)
            test_http = bool(settings.get("allow_loopback_http", False)) and url.hostname in {"localhost", "127.0.0.1", "::1"}
            if url.scheme not in {"https", "http"} or (url.scheme != "https" and not test_http) or url.username or url.password or url.fragment:
                raise ValueError("reply_url must use HTTPS, or explicit loopback fixture HTTP")
        self.http = urllib.request.build_opener(NoRedirect(), urllib.request.ProxyHandler({}))
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args: Any) -> None:
                pass

            def respond(self, code: int, value: dict[str, Any]) -> None:
                payload = compact(value)
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def do_POST(self) -> None:
                if self.path != "/messages":
                    self.respond(404, {"error": "not found"})
                    return
                try:
                    self.connection.settimeout(5)
                    size = int(self.headers.get("Content-Length", "0"))
                    timestamp = self.headers.get("X-Agena-Timestamp", "")
                    supplied = self.headers.get("X-Agena-Signature", "")
                    if not 0 < size <= MAX_EVENT or self.headers.get("Transfer-Encoding"):
                        self.respond(413, {"error": "bounded Content-Length required"})
                        return
                    if abs(int(time.time() * 1000) - int(timestamp)) > 300000:
                        self.respond(401, {"error": "expired request"})
                        return
                    body = self.rfile.read(size)
                    if len(body) != size or not hmac.compare_digest(signature(owner.secret, timestamp, body), supplied):
                        self.respond(401, {"error": "invalid signature"})
                        return
                    event = json.loads(body)
                    allowed = {"id", "event_type", "resource", "actor", "thread", "bot", "occurred_at_ms", "text", "data", "channel"}
                    if not isinstance(event, dict) or set(event) - allowed or event.get("event_type") not in {"message", "membership.revoked"}:
                        raise ValueError("invalid normalized message")
                    for key in ("id", "resource", "actor"):
                        if not isinstance(event.get(key), str) or not 0 < len(event[key].encode()) <= 256 or any(unicodedata.category(c) == "Cc" for c in event[key]):
                            raise ValueError("invalid message identity")
                    if event["actor"] in {"@service", "@shared", "@unattributed", "*"} or any(c.isspace() for c in event["actor"]):
                        raise ValueError("reserved or invalid sender identity")
                    if not isinstance(event.get("text"), str) or not event["text"].strip() or not 0 < len(event["text"].encode()) <= 24576:
                        raise ValueError("invalid message body")
                    event.setdefault("thread", None)
                    event.setdefault("bot", False)
                    event.setdefault("data", None)
                    if type(event.get("bot")) is not bool or (event.get("thread") is not None and (not isinstance(event["thread"], str) or len(event["thread"].encode()) > 256 or any(unicodedata.category(c) == "Cc" for c in event["thread"]))):
                        raise ValueError("invalid thread or bot classification")
                    if len(compact(event["data"])) > 8192:
                        raise ValueError("event metadata too large")
                    if type(event.get("occurred_at_ms")) is not int:
                        raise ValueError("message creation time is required")
                    channel=event.get("channel")
                    if not isinstance(channel,dict) or set(channel)!={"identity","actor_verified","is_member","addressed_to_bot","verified_at_ms"}:
                        raise ValueError("verified channel context is required")
                    identity=channel.get("identity")
                    if not isinstance(identity,dict) or set(identity)!={"tenant","account","kind"} or identity["kind"] not in {"direct","group","public_channel","private_channel"}:
                        raise ValueError("invalid channel identity")
                    for field in ("tenant","account"):
                        value=identity[field]
                        if not isinstance(value,str) or not 0<len(value.encode())<=256 or any(c.isspace() or unicodedata.category(c)=="Cc" for c in value):
                            raise ValueError("invalid channel namespace")
                    if any(type(channel.get(field)) is not bool for field in ("actor_verified","is_member","addressed_to_bot")) or type(channel.get("verified_at_ms")) is not int:
                        raise ValueError("invalid membership evidence")
                    if event["event_type"] == "membership.revoked" and (not channel["actor_verified"] or channel["is_member"]):
                        raise ValueError("membership removal needs verified negative membership evidence")
                    normalized = compact(event)
                    digest = hashlib.sha256(normalized).hexdigest()
                    with owner.database() as db:
                        db.execute("BEGIN IMMEDIATE")
                        prior = db.execute("SELECT digest FROM inbox_v2 WHERE tenant=? AND account=? AND kind=? AND resource=? AND event_id=?", (identity["tenant"],identity["account"],identity["kind"],event["resource"], event["id"])).fetchone()
                        if prior:
                            if prior[0] != digest:
                                self.respond(409, {"error": "message identity already has different content"})
                                return
                            self.respond(200, {"accepted": True, "replayed": True})
                            return
                        if db.execute("SELECT COUNT(*) FROM inbox_v2").fetchone()[0] >= 10000:
                            self.respond(429, {"accepted": False, "error": "inbox capacity reached; archive acknowledged history"})
                            return
                        db.execute("INSERT INTO inbox_v2(tenant,account,kind,resource,event_id,digest,event_json) VALUES(?,?,?,?,?,?,?)", (identity["tenant"],identity["account"],identity["kind"],event["resource"], event["id"], digest, normalized.decode()))
                        db.commit()
                    self.respond(202, {"accepted": True})
                except (ValueError, TypeError, KeyError):
                    self.respond(400, {"error": "invalid normalized message"})
                except Exception:
                    self.respond(503, {"accepted": False, "error": "inbox commit failed; replay the SAME message identity"})

        host = str(settings.get("listen_host", "127.0.0.1"))
        port = int(settings.get("listen_port", 8769))
        if not 0 <= port <= 65535:
            raise ValueError("invalid listener port")
        self.server = ThreadingHTTPServer((host, port), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, name="agena-webhook-chat", daemon=True)
        self.thread.start()
        if settings.get("status_path"):
            status = Path(settings["status_path"])
            status.parent.mkdir(parents=True, exist_ok=True)
            status.write_text(json.dumps({"port": self.server.server_port, "spool_only": self.spool}))
            os.chmod(status, 0o600)

    @contextmanager
    def database(self):
        db = sqlite3.connect(self.path, timeout=5)
        db.execute("PRAGMA busy_timeout=5000")
        try:
            with db:
                yield db
        finally:
            db.close()

    def describe(self) -> dict[str, Any]:
        return {"kind": "channel", "title": "Signed webhook chat bridge", "description": "Platform-specific verification/normalization stays in the adapter. Exact sender/conversation/thread authorization stays in Agena. Demo spool mode is local-only; HTTPS reply mode calls the configured gateway.",
                "minimum_poll_seconds": 1, "idempotent_send": self.spool or self.remote_idempotent, "channel_context_version": 1,
                "config_schema": {"type": "object", "additionalProperties": False, "required": ["conversation"], "properties": {
                    "conversation": {"type": "string", "title": "External conversation", "minLength": 1, "maxLength": 256}}}}

    def poll(self, request: dict[str, Any]) -> dict[str, Any]:
        conversation=request["config"]["conversation"]
        identity=request.get("channel_identity")
        if not isinstance(identity,dict) or set(identity)!={"tenant","account","kind"}:
            raise ValueError("host must pin tenant, account and conversation kind")
        old=request.get("cursor") or {}
        if old and (old.get("conversation")!=conversation or old.get("identity")!=identity):
            raise ValueError("cursor belongs to another channel namespace")
        after=int(old.get("sequence",0));limit=min(max(int(request["limit"]),1),32)
        with self.database() as db:
            db.execute("BEGIN")
            rows=db.execute("SELECT sequence,event_json FROM inbox_v2 WHERE tenant=? AND account=? AND kind=? AND resource=? AND sequence>? ORDER BY sequence LIMIT ?",(identity["tenant"],identity["account"],identity["kind"],conversation,after,limit)).fetchall()
            head=db.execute("SELECT COALESCE(MAX(sequence),0) FROM inbox_v2").fetchone()[0]
        cursor={"identity":identity,"conversation":conversation,"sequence":rows[-1][0] if len(rows)==limit else max(after,head)}
        return {"events":[json.loads(row[1]) for row in rows],"cursor":cursor,"retry_after_seconds":1}

    def send(self, request: dict[str, Any]) -> dict[str, Any]:
        if request["resource"] != request["config"]["conversation"]:
            raise ValueError("outbound destination differs from the configured conversation")
        identity=request.get("channel")
        if not isinstance(identity,dict) or set(identity)!={"tenant","account","kind"}:
            raise ValueError("outbound channel identity is required")
        key = request["delivery_id"]
        payload = {"channel":identity,"delivery_id": key, "conversation": request["resource"], "thread": request.get("thread"), "text": request["text"]}
        body = compact(payload)
        if len(body) > MAX_EVENT:
            raise ValueError("outbound body too large")
        digest = hashlib.sha256(body).hexdigest()
        with self.database() as db:
            db.execute("BEGIN IMMEDIATE")
            old = db.execute("SELECT digest,state,remote_id FROM outgoing WHERE delivery_id=?", (key,)).fetchone()
            if old and old[0] != digest:
                raise ValueError("delivery identity reused with different content")
            if old and old[1] == "sent":
                return {"disposition": "sent", "remote_id": old[2], "retry_after_seconds": None}
            if old and not (self.spool or self.remote_idempotent):
                return {"disposition": "uncertain", "remote_id": old[2], "retry_after_seconds": None}
            if self.spool:
                db.execute("INSERT INTO outgoing(delivery_id,digest,payload_json,state,remote_id) VALUES(?,?,?,'sent',?) ON CONFLICT(delivery_id) DO UPDATE SET state='sent',remote_id=excluded.remote_id", (key, digest, body.decode(), "spool:" + key))
                return {"disposition": "sent", "remote_id": "spool:" + key, "retry_after_seconds": None}
            db.execute("INSERT INTO outgoing(delivery_id,digest,payload_json,state) VALUES(?,?,?,'sending') ON CONFLICT(delivery_id) DO UPDATE SET state='sending'", (key, digest, body.decode()))
        timestamp = str(int(time.time() * 1000))
        headers = {"Content-Type": "application/json", "Idempotency-Key": key, "X-Agena-Timestamp": timestamp, "X-Agena-Signature": signature(self.secret, timestamp, body)}
        disposition, remote_id = "uncertain", None
        try:
            with self.http.open(urllib.request.Request(self.reply_url, data=body, headers=headers, method="POST"), timeout=10) as response:
                raw = response.read(MAX_EVENT + 1)
                if len(raw) > MAX_EVENT:
                    raise ValueError("reply acknowledgement too large")
                result = json.loads(raw)
                if result.get("accepted") is True:
                    disposition, remote_id = "sent", str(result.get("message_id", key))[:256]
                elif result.get("accepted") is False and result.get("no_effect") is True:
                    disposition = "rejected"
        except Exception:
            # Timeout/5xx/invalid ACK are ambiguous external effects, not a
            # promise that no message was delivered. Host decides on replay.
            pass
        with self.database() as db:
            db.execute("UPDATE outgoing SET state=?,remote_id=? WHERE delivery_id=?", (disposition, remote_id, key))
        return {"disposition": disposition, "remote_id": remote_id, "retry_after_seconds": None}

    def close(self) -> None:
        if getattr(self, "server", None):
            self.server.shutdown()
            self.server.server_close()
            self.thread.join(timeout=2)

if __name__ == "__main__":
    serve("webhook_chat", WebhookChat())
