#!/usr/bin/env python3
"""Installable, read-only GitHub new-Issue event source for Agena.

No GitHub code or credentials are added to the core Runtime. Only enabled host
grants call poll(). Initial history predating activation is deliberately skipped.
Pagination checkpoints and provider backoff are committed by the host with the
normalized inbox, so a crash/lost poll response does not advance past work.
"""
from __future__ import annotations
from datetime import datetime, timezone
import json
import os
import re
from typing import Any
import urllib.error
import urllib.parse
import urllib.request
from protocol import serve

MAX_BODY = 1024 * 1024
API_VERSION = "2026-03-10"

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args: Any, **kwargs: Any) -> None:
        return None

def timestamp(value: str) -> int:
    return int(datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000)

def text_bound(text: Any, maximum: int) -> str:
    return str(text or "").encode()[:maximum].decode("utf-8", errors="ignore")

class GithubIssues:
    def initialize(self, settings: dict[str, Any]) -> None:
        self.base = str(settings.get("api_base", "https://api.github.com")).rstrip("/")
        url = urllib.parse.urlsplit(self.base)
        loopback_test = bool(settings.get("allow_loopback_http", False)) and url.hostname in {"127.0.0.1", "localhost", "::1"}
        if url.username or url.password or url.query or url.fragment or (url.scheme != "https" and not loopback_test):
            raise ValueError("use HTTPS; HTTP requires explicit loopback fixture opt-in")
        if url.scheme not in {"http", "https"}:
            raise ValueError("invalid API origin")
        variable = str(settings.get("token_env", "AGENA_GITHUB_TOKEN"))
        if not re.fullmatch(r"[A-Z_][A-Z0-9_]{0,100}", variable):
            raise ValueError("token_env must name an environment variable")
        self.token = os.environ.get(variable, "")
        if "\r" in self.token or "\n" in self.token or len(self.token) > 4096:
            raise ValueError("invalid credential")
        self.http = urllib.request.build_opener(NoRedirect(), urllib.request.ProxyHandler({}))

    def describe(self) -> dict[str, Any]:
        return {"kind": "event_source", "title": "GitHub new issues", "description": "Read-only repository issue discovery with conditional requests and durable pagination. Pull requests are excluded; no comments, labels, commits or PRs are published by this adapter.",
                "minimum_poll_seconds": 60, "idempotent_send": False,
                "config_schema": {"type": "object", "additionalProperties": False, "required": ["repository"], "properties": {
                    "repository": {"type": "string", "title": "Repository", "description": "Exact owner/repository, matching the host grant resource.", "minLength": 3, "maxLength": 180},
                    "labels": {"type": "array", "items": {"type": "string", "maxLength": 100}, "maxItems": 16, "default": []}}}}

    def _validated_url(self, url: str, repository: str) -> str:
        base, candidate = urllib.parse.urlsplit(self.base), urllib.parse.urlsplit(url)
        expected_path = base.path + f"/repos/{repository}/issues"
        if (base.scheme, base.netloc, expected_path) != (candidate.scheme, candidate.netloc, candidate.path) or candidate.username or candidate.fragment:
            raise ValueError("pagination must stay on the configured repository/origin")
        return url

    def poll(self, request: dict[str, Any]) -> dict[str, Any]:
        config = request["config"]
        repository = config["repository"]
        if not isinstance(repository, str) or not re.fullmatch(r"[A-Za-z0-9_.-]{1,80}/[A-Za-z0-9_.-]{1,100}", repository):
            raise ValueError("invalid owner/repository")
        if any(part in {".", ".."} for part in repository.split("/")):
            raise ValueError("invalid repository path")
        labels = config.get("labels") or []
        if not isinstance(labels, list) or len(labels) > 16 or any(not isinstance(v, str) for v in labels):
            raise ValueError("invalid labels")
        old = request.get("cursor") or {}
        if not isinstance(old, dict) or (old.get("repository") not in (None, repository)):
            raise ValueError("cursor belongs to another repository")
        limit = min(max(int(request["limit"]), 1), 32)
        activation = int(request["not_before_ms"])
        watermark = int(old.get("watermark_ms", activation))
        since = datetime.fromtimestamp(activation / 1000, timezone.utc).isoformat().replace("+00:00", "Z")
        first = self.base + f"/repos/{repository}/issues?" + urllib.parse.urlencode({"state": "all", "sort": "created", "direction": "desc", "per_page": limit, "since": since})
        url = self._validated_url(old.get("next") or first, repository)
        headers = {"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": API_VERSION, "User-Agent": "Agena-GitHub-Issue-Source/1.0"}
        if self.token:
            headers["Authorization"] = "Bearer " + self.token
        if not old.get("next") and old.get("etag"):
            headers["If-None-Match"] = str(old["etag"])
        query = urllib.request.Request(url, headers=headers)
        try:
            response = self.http.open(query, timeout=10)
        except urllib.error.HTTPError as error:
            with error:
                if error.code == 304:
                    return {"events": [], "cursor": old, "retry_after_seconds": 60}
                # A permission denial is not a successful empty poll. Only
                # explicit rate-limit evidence gets checkpoint-preserving
                # backoff; other 403s become visible host diagnostics.
                limited = error.code == 429
                if error.code == 403:
                    message = error.read(16384).decode("utf-8", errors="replace").lower()
                    limited = bool(error.headers.get("Retry-After")) or error.headers.get("X-RateLimit-Remaining") == "0" or "rate limit" in message
                if limited:
                    now = int(request["now_ms"]) // 1000
                    try:
                        retry = max(60, int(error.headers.get("Retry-After", "0")), int(error.headers.get("X-RateLimit-Reset", "0")) - now)
                    except ValueError:
                        retry = 60
                    return {"events": [], "cursor": old, "retry_after_seconds": min(retry, 86400)}
                raise ValueError(f"GitHub read failed with HTTP {error.code}") from None
        with response:
            raw = response.read(MAX_BODY + 1)
            if len(raw) > MAX_BODY:
                raise ValueError("GitHub page exceeds byte budget")
            issues = json.loads(raw)
            if not isinstance(issues, list) or len(issues) > limit:
                raise ValueError("invalid GitHub issue page")
            etag = response.headers.get("ETag")
            links = response.headers.get("Link", "")
        events: list[dict[str, Any]] = []
        newest = int(old.get("scan_max_ms", watermark))
        oldest: int | None = None
        floor = max(activation, watermark - 120_000)
        for issue in issues:
            created = timestamp(issue["created_at"])
            oldest = min(oldest, created) if oldest is not None else created
            newest = max(newest, created)
            if "pull_request" in issue or created < floor:
                continue
            issue_labels = [v["name"] if isinstance(v, dict) else v for v in issue.get("labels", [])]
            if not all(label in issue_labels for label in labels):
                continue
            number = int(issue["number"])
            if number <= 0:
                raise ValueError("invalid issue number")
            title = text_bound(issue.get("title"), 1024)
            body = text_bound(issue.get("body"), 16 * 1024)
            actor = text_bound(issue.get("user", {}).get("login", "unknown"), 128)
            events.append({"id": f"github:{repository}:{number}:opened", "event_type": "issue.opened", "resource": repository,
                "actor": actor, "thread": None, "bot": issue.get("user", {}).get("type") == "Bot", "occurred_at_ms": created,
                "text": f"Issue #{number}: {title}\n\n{body}", "data": {"number": number, "title": title,
                    "url": text_bound(issue.get("html_url"), 1024), "labels": [text_bound(v, 100) for v in issue_labels[:16]]}})
        next_url = None
        for item in links.split(","):
            match = re.match(r'\s*<([^>]+)>;\s*rel="next"', item)
            if match:
                next_url = self._validated_url(match.group(1), repository)
                break
        continue_scan = bool(next_url and issues and oldest is not None and oldest >= floor)
        if continue_scan:
            cursor = {"repository": repository, "watermark_ms": watermark, "scan_max_ms": newest,
                "next": next_url, "scan_etag": old.get("scan_etag") if old.get("next") else etag}
        else:
            cursor = {"repository": repository, "watermark_ms": newest,
                "etag": old.get("scan_etag") if old.get("next") else etag}
        return {"events": events, "cursor": cursor, "retry_after_seconds": 60}

    def close(self) -> None:
        pass

if __name__ == "__main__":
    serve("github_issues", GithubIssues())
