"""Real terminal live-reply checks against disposable TestKit sessions only."""
from __future__ import annotations
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import urllib.parse


def main() -> None:
    backend = os.environ["LIVE_BACKEND"]
    assert urllib.parse.urlparse(backend).hostname in {"127.0.0.1", "localhost", "::1"}
    cases = json.loads(os.environ["LIVE_CASES"])
    evidence = Path(os.environ["LIVE_EVIDENCE"])
    binary = Path(os.environ["AGENA_TEST_BINARY"]).resolve(strict=True)
    report = []
    with tempfile.TemporaryDirectory(prefix="agena-live-tui-") as temporary:
        root = Path(temporary)
        socket = root / "tmux.sock"
        env = os.environ.copy()
        for name in ("AGENA_SERVER_TOKEN", "AGENA_SERVER_PASSWORD", "TMUX"):
            env.pop(name, None)
        env.update(TERM="xterm-256color", LANG="C.UTF-8", TMUX_TMPDIR=str(root), RUST_MIN_STACK="8388608")

        def tmux(*args: str, check: bool = True) -> str:
            result = subprocess.run(["tmux", "-S", str(socket), "-f", "/dev/null", *args], env=env, text=True, capture_output=True, timeout=15)
            if check and result.returncode:
                raise AssertionError(f"tmux {args}: {result.stderr}")
            return result.stdout

        def capture() -> str:
            return tmux("capture-pane", "-p", "-t", "live:0.0")

        def wait_for(text: str, label: str, seconds: float = 40) -> None:
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if text in capture():
                    return
                time.sleep(0.05)
            frame = capture()
            (evidence / "tui-failure.txt").write_text(frame)
            raise AssertionError(f"timeout: {label}\n{frame}")

        try:
            for item in cases:
                invocation = shlex.join(["env", "-u", "AGENA_SERVER_TOKEN", "-u", "AGENA_SERVER_PASSWORD", str(binary), "--server", backend, "--data-dir", str(root / item["label"]), "tui", "--session", str(item["session_id"]), "--locale", "en-US", "--log-file", str(evidence / (item["label"] + "-tui.log"))])
                tmux("new-session", "-d", "-s", "live", "-x", "120", "-y", "40", invocation)
                # Bootstrap must display exactly the history created by Web.
                wait_for(item["web_reply"], "initial matching conversation")
                # An explicitly opened transcript starts in NAV mode. Use its
                # real insert binding before pasting into the composer.
                tmux("send-keys", "-t", "live:0.0", "i")
                paste = root / "prompt.txt"
                paste.write_text(item["tui_prompt"])
                tmux("load-buffer", "-b", "input", str(paste))
                tmux("paste-buffer", "-b", "input", "-t", "live:0.0", "-p")
                wait_for(item["tui_prompt"], "composer input")
                started = time.monotonic()
                tmux("send-keys", "-t", "live:0.0", "Enter")
                wait_for(item["tui_reply"], "live reply without refresh or further keys", 8)
                report.append({"client":"tui", "label":item["label"], "sessionId":item["session_id"], "submitToVisibleMs":round((time.monotonic()-started)*1000)})
                (evidence / (item["label"] + "-tui.txt")).write_text(capture())
                (evidence / "tui-latency.json").write_text(json.dumps(report, indent=2))
                tmux("kill-session", "-t", "live")
            print("TUI_LIVE_REPLY", json.dumps(report))
        finally:
            tmux("kill-server", check=False)


if __name__ == "__main__":
    main()
