"""Canonical Bot acceptance in a real TUI and a private tmux server.

Uses a local isolated fixture, never the operator's service, tmux or credentials.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request


def main() -> None:
    endpoint = os.environ["BOT_BACKEND"].rstrip("/")
    assert urllib.parse.urlparse(endpoint).hostname in {"localhost", "127.0.0.1", "::1"}
    binary = Path(os.environ["AGENA_TEST_BINARY"]).resolve(strict=True)
    evidence = Path(os.environ["BOT_EVIDENCE"])
    evidence.mkdir(parents=True, exist_ok=True)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def bot_id() -> int:
        request = urllib.request.Request(f"{endpoint}/api/v1/bot/session", method="POST")
        with opener.open(request, timeout=15) as response:
            result = json.load(response)
        assert isinstance(result["session_id"], int) and result["session_id"] > 0
        return result["session_id"]

    canonical = bot_id()
    with opener.open(urllib.request.Request(f"{endpoint}/api/v1/bot/session", method="POST"), timeout=15) as response:
        canonical_name = json.load(response)["name"]
    with tempfile.TemporaryDirectory(prefix="agena-bot-tui-") as temporary:
        root = Path(temporary)
        socket = root / "tmux.sock"
        env = os.environ.copy()
        for name in ("AGENA_SERVER_TOKEN", "AGENA_SERVER_PASSWORD", "TMUX"):
            env.pop(name, None)
        env.update(AGENA_SERVER_URL=endpoint, AGENA_DATA_DIR=str(root / "client-data"),
                   TERM="xterm-256color", LANG="C.UTF-8", RUST_MIN_STACK="8388608", TMUX_TMPDIR=str(root))

        def tmux(*args: str, check: bool = True) -> str:
            result = subprocess.run(["tmux", "-S", str(socket), "-f", "/dev/null", *args],
                                    env=env, text=True, capture_output=True, timeout=15)
            if check and result.returncode:
                raise AssertionError(f"tmux {args}: {result.stderr}")
            return result.stdout

        def capture() -> str:
            return tmux("capture-pane", "-p", "-t", "bot:0.0")

        def wait(check, description: str, seconds: float = 45):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                result = check()
                if result:
                    return result
                time.sleep(0.1)
            frame = capture()
            (evidence / "bot-tui-failure.txt").write_text(frame)
            raise AssertionError(f"timeout: {description}\n{frame}")

        def keys(*values: str):
            tmux("send-keys", "-t", "bot:0.0", *values)

        def text(value: str):
            file = root / "paste.txt"
            file.write_text(value)
            tmux("load-buffer", "-b", "bot-input", str(file))
            tmux("paste-buffer", "-b", "bot-input", "-t", "bot:0.0", "-p")
            time.sleep(0.2)

        def command(value: str):
            text(value)
            time.sleep(0.3)
            keys("Down", "Enter")

        try:
            invocation = shlex.join(["env", "-u", "AGENA_SERVER_TOKEN", "-u", "AGENA_SERVER_PASSWORD",
                                    str(binary), "--server", endpoint, "--data-dir", str(root / "client-data"),
                                    "tui", "--locale", "en-US", "--log-file", str(evidence / "bot-tui.log")])
            tmux("new-session", "-d", "-s", "bot", "-x", "110", "-y", "34")
            tmux("set-option", "-t", "bot", "remain-on-exit", "on")
            tmux("respawn-pane", "-k", "-t", "bot:0.0", invocation)
            wait(lambda: "AI Bot" in capture(), "discoverable Bot on initial hub")
            (evidence / "bot-tui-hub.txt").write_text(capture())
            keys("C-b")
            wait(lambda: "bot-ui-web-reply" in capture(), "Web history in the same TUI Bot")
            assert bot_id() == canonical
            (evidence / "bot-tui-chat-wide.txt").write_text(capture())
            text("bot-ui-tui-message")
            keys("Enter")
            wait(lambda: "bot-ui-tui-reply" in capture(), "real TUI Bot response", 60)
            command("/bot")
            wait(lambda: "bot-ui-tui-reply" in capture(), "reopening Bot retains transcript")
            assert bot_id() == canonical
            text("bot-ui-tui-draft")
            keys("C-b")
            wait(lambda: "bot-ui-tui-draft" in capture() and canonical_name in capture(), "draft survives repeat Bot shortcut")
            tmux("resize-window", "-t", "bot:0", "-x", "42", "-y", "18")
            wait(lambda: int(tmux("display-message", "-p", "-t", "bot:0.0", "#{pane_width}").strip()) == 42
                 and canonical_name in capture(), "42-column Bot redraw")
            time.sleep(0.3)
            frame = capture()
            assert "bot-ui-tui-draft" in frame, "composer remains usable at narrow size"
            (evidence / "bot-tui-chat-narrow.txt").write_text(frame)
            result = {"ok": True, "bot": canonical, "checks": ["initial hub entry", "Ctrl+B", "shared Web history", "real chat", "/bot", "repeat identity", "draft", "42x18 layout"], "evidence": str(evidence)}
            (evidence / "tui-result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2))
            print(json.dumps(result, ensure_ascii=False))
        finally:
            tmux("kill-server", check=False)


if __name__ == "__main__":
    main()
