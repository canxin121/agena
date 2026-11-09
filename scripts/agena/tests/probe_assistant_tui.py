"""Drive the actual Agena TUI in a private tmux server against a local fixture.

No global tmux session, production instance, or real model is touched. Every
assertion combines rendered terminal text with an independent HTTP assistant or integration fact.
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
    endpoint = os.environ["ASSISTANT_BACKEND"].rstrip("/")
    assert urllib.parse.urlparse(endpoint).hostname in {"localhost", "127.0.0.1", "::1"}
    source = int(os.environ["ASSISTANT_SOURCE"])
    binary = Path(os.environ["AGENA_TEST_BINARY"]).resolve(strict=True)
    evidence = Path(os.environ["ASSISTANT_EVIDENCE"])
    evidence.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="agena-tui-assistant-") as root:
        root = Path(root)
        socket = root / "tmux.sock"
        env = os.environ.copy()
        for key in ["AGENA_SERVER_TOKEN", "AGENA_SERVER_PASSWORD", "TMUX"]:
            env.pop(key, None)
        env.update({"AGENA_SERVER_URL": endpoint, "AGENA_DATA_DIR": str(root / "client-data"),
                    "TERM": "xterm-256color", "LANG": "C.UTF-8", "RUST_MIN_STACK": "8388608",
                    "TMUX_TMPDIR": str(root)})

        def tmux(*args: str, check: bool = True) -> str:
            result = subprocess.run(["tmux", "-S", str(socket), "-f", "/dev/null", *args],
                                    env=env, text=True, capture_output=True, timeout=10)
            if check and result.returncode:
                raise AssertionError(f"tmux {args}: {result.stderr}")
            return result.stdout

        def capture() -> str:
            return tmux("capture-pane", "-p", "-t", "assistant:0.0", "-S", "-80")

        def wait(check, name: str, seconds: float = 30):
            until = time.monotonic() + seconds
            while time.monotonic() < until:
                result = check()
                if result:
                    return result
                time.sleep(0.1)
            text = capture()
            (evidence / "terminal-failure.txt").write_text(text)
            raise AssertionError(f"timeout waiting for {name}\n{text}")

        def keys(*values: str):
            tmux("send-keys", "-t", "assistant:0.0", *values)

        def text(value: str):
            tmux("send-keys", "-t", "assistant:0.0", "-l", "--", value)

        def command(value: str):
            keys("Escape")
            keys("i")
            text(value)
            wait(lambda: value in capture() and "Command Palette" in capture(), "catalog suggestion")
            # The command palette renders from asynchronous catalog state.
            # Explicit selection is the same keyboard path a user takes.
            keys("Down")
            keys("Enter")

        try:
            invocation = shlex.join(["env", "-u", "AGENA_SERVER_TOKEN", "-u", "AGENA_SERVER_PASSWORD", str(binary), "--server", endpoint, "--data-dir", str(root / "client-data"),
                                     "tui", "--session", str(source), "--locale", "en-US",
                                     "--log-file", str(evidence / "tui.log")])
            # Retain an early CLI failure frame instead of losing the entire
            # private tmux server when its only pane exits.
            tmux("new-session", "-d", "-s", "assistant", "-x", "110", "-y", "34")
            tmux("set-option", "-t", "assistant", "remain-on-exit", "on")
            tmux("respawn-pane", "-k", "-t", "assistant:0.0", invocation)
            wait(lambda: "Agena" in capture() or "Session" in capture(), "initial terminal frame", 45)
            if os.environ.get("CHANNEL_ACCESS_PROBE") == "1":
                def channel_detail():
                    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    with opener.open(f"{endpoint}/api/v1/sessions/{source}/integrations",timeout=10) as response:
                        binding=json.load(response)["bindings"][0]
                    with opener.open(f"{endpoint}/api/v1/sessions/{source}/integrations/{binding['id']}",timeout=10) as response:
                        return json.load(response)
                command("/integrations")
                wait(lambda:"Channel access fixture" in capture(),"channel binding in terminal")
                before=channel_detail();policy=before["binding"]["spec"]["channel_policy"]
                keys("a")
                wait(lambda:'"members"' in capture() or '"service_grant"' in capture(),"native access policy editor")
                # The editor starts at EOF. Repeated Ctrl-U removes each line
                # then its newline; excess operations on an empty buffer are no-ops.
                keys(*(["C-u"]*(2*len(json.dumps(policy,indent=2).splitlines())+8)))
                policy["members"].append({"actor":"tui-member","role":"member","expires_at_ms":None})
                policy["per_user_daily_limit"]=23
                text(json.dumps(policy,ensure_ascii=False,separators=(",",":")))
                keys("M-Enter")
                wait(lambda:any(m["actor"]=="tui-member" for m in channel_detail()["binding"]["spec"]["channel_policy"]["members"]),"native role policy saved")
                result=channel_detail()
                assert result["binding"]["spec"]["channel_policy"]["per_user_daily_limit"]==23
                assert result["access_audit"][0]["actor"]=="instance_operator"
                wait(lambda:"a access" in capture() and "Saving" not in capture() and "Loading" not in capture(),"native policy callback settled")
                (evidence/"channel-access-tui-wide.txt").write_text(capture())
                tmux("resize-window","-t","assistant:0","-x","48","-y","20")
                def resized_access_panel():
                    # tmux resize updates pane geometry before the app handles
                    # SIGWINCH. Wait for a NEW 48-column frame, not matching
                    # text still visible in the old wide frame/scrollback.
                    frame=tmux("capture-pane","-p","-t","assistant:0.0")
                    lines=frame.splitlines()
                    return bool(lines and len(lines[0])==48 and "a access" in frame and "Saving" not in frame and "Loading" not in frame)
                wait(resized_access_panel,"native panel actually redrawn at 48 columns")
                keys("a")
                wait(lambda:"Ctrl+Enter" in capture(),"narrow access editor")
                (evidence/"channel-access-tui-narrow.txt").write_text(capture())
                keys("Escape")
                assert tmux("display-message","-p","-t","assistant:0.0","#{pane_dead}").strip()=="0"
                print(json.dumps({"ok":True,"source":source,"checks":["native policy editor","actual role assignment","per-user limit","audit","resize"],"evidence":str(evidence)}))
                return
            if os.environ.get("INTEGRATIONS_PROBE") == "1":
                def integrations():
                    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    with opener.open(f"{endpoint}/api/v1/sessions/{source}/integrations", timeout=10) as response:
                        return json.load(response)
                command("/integrations")
                wait(lambda: "example.github_issues" in capture(), "external provider in terminal")
                keys("n")
                wait(lambda: "owner/repository" in capture(), "generated integration draft")
                keys("M-Enter")
                created = wait(lambda: integrations()["bindings"], "disabled grant saved")[0]
                assert created["state"] == "disabled"
                wait(lambda: "disabled" in capture(), "disabled grant displayed")
                keys("e")
                wait(lambda: "Enable" in capture() and "Verify" in capture(), "explicit enable review")
                assert integrations()["bindings"][0]["state"] == "disabled"
                keys("Enter")
                wait(lambda: integrations()["bindings"][0]["state"] == "enabled", "enable grant")
                wait(lambda: "enabled" in capture(), "enabled grant displayed")
                keys("p")
                wait(lambda: "Pause" in capture(), "pause review")
                keys("Enter")
                wait(lambda: integrations()["bindings"][0]["state"] == "paused", "pause grant")
                (evidence / "integrations-tui-wide.txt").write_text(capture())
                tmux("resize-window", "-t", "assistant:0", "-x", "48", "-y", "20")
                wait(lambda: "automation" in capture(), "integration panel after resize")
                (evidence / "integrations-tui-narrow.txt").write_text(capture())
                assert tmux("display-message", "-p", "-t", "assistant:0.0", "#{pane_dead}").strip() == "0"
                print(json.dumps({"ok": True, "source": source, "binding": created["id"], "checks": ["external provider", "disabled draft", "explicit enable", "pause", "resize"], "evidence": str(evidence)}))
                return
            if os.environ.get("ASSISTANT_PROBE") == "1":
                def assistant():
                    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    with opener.open(f"{endpoint}/api/v1/sessions/{source}/assistant", timeout=5) as response:
                        return json.load(response)
                def chat(value: str, marker: str):
                    keys("Escape")
                    keys("i")
                    text(value)
                    wait(lambda: value in capture(), "composed assistant request")
                    keys("Enter")
                    wait(lambda: marker in capture(), marker, 60)
                chat("以后叫你小鱼，叫我阿林", "assistant-ui-persona-saved")
                chat("记住我希望报告先给结论", "assistant-ui-memory-saved")
                chat("两秒后提醒我核对结果", "assistant-ui-reminder-scheduled")
                wait(lambda: "assistant-ui-contact-delivered" in capture(), "proactive reminder in original terminal", 60)
                chat("一小时后再提醒我一次", "assistant-ui-later-scheduled")
                before = assistant()
                assert before["profile"]["preferences"]["name"] == "小鱼"
                future = next(r for r in before["reminders"] if r["state"] == "active")
                command("/assistant")
                wait(lambda: "Personality" in capture() and "Minimum" in capture(), "assistant terminal panel")
                keys("Enter")
                keys("C-a", "C-k")
                text("TUI小鱼")
                keys("M-Enter")
                keys("s")
                wait(lambda: assistant()["profile"]["preferences"]["name"] == "TUI小鱼", "save terminal persona")
                (evidence / "assistant-tui-profile.txt").write_text(capture())
                keys("2")
                wait(lambda: "ui-report-style" in capture(), "terminal memory list")
                keys("e")
                keys("C-a", "C-k")
                text("assistant-ui-memory-corrected: action items first")
                keys("M-Enter")
                wait(lambda: any("assistant-ui-memory-corrected" in m["body"] for m in assistant()["memories"]), "terminal memory correction")
                wait(lambda: "ui-report-style" in capture(), "memory list after correction")
                keys("d")
                wait(lambda: "Delete this saved memory" in capture(), "forget confirmation")
                keys("Enter")
                wait(lambda: not assistant()["memories"], "actual terminal memory deletion")
                keys("3")
                wait(lambda: "assistant-ui-future-contact" in capture(), "terminal reminders")
                keys("c")
                wait(lambda: "Apply this change" in capture(), "reminder cancellation confirmation")
                keys("Enter")
                wait(lambda: any(r["id"] == future["id"] and r["state"] == "cancelled" for r in assistant()["reminders"]), "terminal cancel schedule")
                tmux("resize-window", "-t", "assistant:0", "-x", "48", "-y", "20")
                wait(lambda: "Assistant" in capture(), "compact assistant panel")
                (evidence / "assistant-tui-narrow.txt").write_text(capture())
                assert tmux("display-message", "-p", "-t", "assistant:0.0", "#{pane_dead}").strip() == "0"
                print(json.dumps({"ok": True, "source": source, "checks": ["natural persona", "natural memory", "proactive original-chat reminder", "native profile editor", "memory edit/delete", "cancel reminder", "resize"], "evidence": str(evidence)}, ensure_ascii=False))
                return
            raise AssertionError("select CHANNEL_ACCESS_PROBE, INTEGRATIONS_PROBE or ASSISTANT_PROBE")
        finally:
            tmux("kill-server", check=False)


if __name__ == "__main__":
    main()
