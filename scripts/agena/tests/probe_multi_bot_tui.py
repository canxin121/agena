"""Real named Bot selection/persona acceptance in a private tmux server.

The backend and model are the Rust scenario's isolated fixtures. No production
service, terminal server, settings, credentials, or database is opened.
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
    a = json.loads(os.environ["BOT_PROFILE_A"])
    b = json.loads(os.environ["BOT_PROFILE_B"])
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def api(route: str):
        with opener.open(endpoint + route, timeout=15) as response:
            assert 200 <= response.status < 300
            return json.load(response)

    a = api(f"/api/v1/bots/{a['id']}")
    b = api(f"/api/v1/bots/{b['id']}")
    for profile in (a, b):
        resource = api(f"/api/v1/sessions/{profile['session_id']}")
        state = api(f"/api/v1/sessions/{profile['session_id']}/state")
        assert resource["workspace_id"] is None and state["session"]["workspace_id"] is None
        assert state["execution"].get("effective_workspace_root") is None
    with tempfile.TemporaryDirectory(prefix="agena-multi-bot-tui-") as temporary:
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
            return tmux("capture-pane", "-p", "-t", "bots:0.0")

        def wait(check, description: str, seconds: float = 45):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                result = check()
                if result:
                    return result
                time.sleep(0.1)
            frame = capture()
            (evidence / "multi-tui-failure.txt").write_text(frame)
            raise AssertionError(f"timeout: {description}\n{frame}")

        def keys(*values: str):
            tmux("send-keys", "-t", "bots:0.0", *values)

        def text(value: str):
            file = root / "paste.txt"
            file.write_text(value)
            tmux("load-buffer", "-b", "bot-input", str(file))
            tmux("paste-buffer", "-b", "bot-input", "-t", "bots:0.0", "-p")
            wait(lambda: value in capture(), "pasted text appears in terminal")

        def command(value: str):
            text(value)
            keys("Down", "Enter")

        def choose(profile):
            command("/bot list")
            wait(lambda: "Named Bots" in capture() and profile["name"] in capture(), "real named Bot picker")
            profiles = api("/api/v1/bots")["items"]
            index = next(i for i, item in enumerate(profiles) if item["id"] == profile["id"])
            keys(*(["Up"] * len(profiles)))
            if index:
                keys(*(["Down"] * index))
            (evidence / f"multi-tui-picker-{profile['id']}.txt").write_text(capture())
            keys("Enter")
            wait(lambda: profile["name"] in capture() and "Named Bots" not in capture(), f"selected {profile['name']}")

        try:
            invocation = shlex.join(["env", "-u", "AGENA_SERVER_TOKEN", "-u", "AGENA_SERVER_PASSWORD",
                                    str(binary), "--server", endpoint, "--data-dir", str(root / "client-data"),
                                    "tui", "--locale", "en-US", "--log-file", str(evidence / "multi-tui.log")])
            tmux("new-session", "-d", "-s", "bots", "-x", "110", "-y", "34")
            tmux("set-option", "-t", "bots", "remain-on-exit", "on")
            tmux("respawn-pane", "-k", "-t", "bots:0.0", invocation)
            wait(lambda: "AI Bot" in capture(), "initial hub Bot entry")
            # Hub's list key is a real native control and avoids relying on the
            # server default before explicitly choosing the two named profiles.
            keys("l")
            wait(lambda: "Named Bots" in capture() and a["name"] in capture(), "initial named picker")
            profiles = api("/api/v1/bots")["items"]
            index = next(i for i, item in enumerate(profiles) if item["id"] == a["id"])
            keys(*(["Up"] * len(profiles)))
            if index:
                keys(*(["Down"] * index))
            keys("Enter")
            wait(lambda: "multi-web-a-reply" in capture(), "A's Web history in TUI")
            assert "global-ui-image.png" in capture(), "Web attachment missing from native TUI transcript"
            (evidence / "multi-tui-attachment.txt").write_text(capture())
            assert "multi-web-b-reply" not in capture(), "B history leaked to A"
            assert os.environ["BOT_PROJECT_PATH"] not in capture(), "global Bot header exposes project cwd"
            text("multi-tui-a-message"); keys("Enter")
            wait(lambda: "multi-tui-a-reply" in capture(), "real TUI A reply", 60)
            (evidence / "multi-tui-a-wide.txt").write_text(capture())
            choose(b)
            wait(lambda: "multi-web-b-reply" in capture(), "B's matching Web history")
            assert "multi-tui-a-reply" not in capture(), "A history leaked to B"
            assert os.environ["BOT_PROJECT_PATH"] not in capture(), "global Bot header exposes project cwd"
            text("multi-tui-b-message"); keys("Enter")
            wait(lambda: "multi-tui-b-reply" in capture(), "real TUI B reply", 60)
            command("/bot edit")
            wait(lambda: "s save" in capture(), "selected Bot persona form")
            form = capture()
            assert "Additional system instructions" in form, form
            assert "A UI persona" not in form, form  # Editing B, never A.
            for removed in ("Address user", "Tone", "Language"):
                assert removed not in form, form
            keys("Enter")
            wait(lambda: "Alt+Enter" in capture(), "name field editor")
            keys("C-a", "C-k"); text("Borealis TUI edited"); keys("M-Enter")
            wait(lambda: "s save" in capture() and "Borealis TUI edited" in capture(), "name draft applied")
            keys("Down", "Enter")
            wait(lambda: "Alt+Enter" in capture(), "instructions editor")
            keys("C-a", "C-k"); text("TUI edited B personality persisted"); keys("M-Enter")
            wait(lambda: "s save" in capture(), "persona draft applied")
            (evidence / "multi-tui-editor-wide.txt").write_text(capture())
            keys("s")
            wait(lambda: api(f"/api/v1/bots/{b['id']}")["name"] == "Borealis TUI edited", "TUI saved profile through real Web API")
            saved_b = api(f"/api/v1/bots/{b['id']}")
            assert saved_b["persona"]["instructions"] == "TUI edited B personality persisted"
            saved_a = api(f"/api/v1/bots/{a['id']}")
            assert saved_a["name"] == "Aurora Web edited"
            assert saved_a["persona"]["instructions"] == "Web edited A personality persisted"
            # Successful save returns to the native list; reopen B.
            wait(lambda: "Named Bots" in capture(), "return from saved persona form")
            keys("Enter")
            wait(lambda: "multi-tui-b-reply" in capture(), "history retained after persona edit")
            text("multi-tui-b-draft")
            tmux("resize-window", "-t", "bots:0", "-x", "42", "-y", "18")
            wait(lambda: int(tmux("display-message", "-p", "-t", "bots:0.0", "#{pane_width}").strip()) == 42
                 and "multi-tui-b-draft" in capture(), "42x18 chat with usable composer")
            (evidence / "multi-tui-b-42x18.txt").write_text(capture())
            result = {"ok": True, "a": a["id"], "b": b["id"], "checks": ["named picker A/B", "matching Web history", "real A/B chat", "isolated transcripts", "native persona edit", "API persistence", "A unchanged", "42x18 layout"], "evidence": str(evidence)}
            (evidence / "multi-tui-result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2))
            print(json.dumps(result, ensure_ascii=False))
        finally:
            tmux("kill-server", check=False)


if __name__ == "__main__":
    main()
