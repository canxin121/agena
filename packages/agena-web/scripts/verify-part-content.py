"""Exercise the production Part components against controlled canonical SSE.

Start `bun scripts/part-content-server.mjs`, then run with Python + Playwright.
AGENA_WEBKIT_EXECUTABLE optionally selects an already installed WebKit binary.
Screenshots and the measured SSE-to-emulator-paint results go to --output.
This fixture does not measure producer, server, or network latency.
"""
import argparse
import json
import os
import re
from pathlib import Path

from playwright.sync_api import sync_playwright


def verify(page, output):
    page.goto("http://127.0.0.1:5175/tests/fixtures/part-content.html", wait_until="networkidle")
    page.wait_for_function("partFixture.streams.size === 2 && partFixture.terminals.size === 2")
    page.evaluate("window.savedShell = document.querySelector('[data-fixture=shell] .xterm')")
    page.evaluate("partFixture.emit('shell',{type:'log',stream:'stdout',text:'  Building…\\n\\x1b[32mPASS\\x1b[0m  module 1\\n\\n'})")
    page.wait_for_function("partFixture.terminals.get('shell').buffer.active.getLine(1)?.translateToString(true).includes('PASS')")
    shell = page.locator('[data-fixture="shell"]')
    shell.get_by_role("button", name="复制", exact=True).click()
    page.wait_for_function("partFixture.copiedText.length > 0")
    assert page.evaluate("partFixture.copiedText") == "  Building…\nPASS  module 1\n\n"

    page.evaluate("partFixture.emit('shell',{type:'log',stream:'stdout',text:'中文与 emoji 🧪\\nprogress 10%\\rprogress 90%\\n'})")
    page.evaluate("partFixture.emit('shell',{type:'log',stream:'stderr',text:'warning: diagnostic channel\\n'})")
    page.wait_for_function("partFixture.terminals.get('shell').buffer.active.getLine(5)?.translateToString(true).includes('warning')")
    assert "stderr" in shell.inner_text()

    attrs = {"foreground": {"type": "indexed", "index": 2}, "background": None,
             "bold": True, "dim": False, "italic": False, "underline": False, "inverse": False}
    screen = {"rows": 6, "cols": 60, "cursor_row": 3, "cursor_col": 2,
              "cursor_visible": True, "alternate_screen": False, "bracketed_paste": False,
              "application_cursor": False,
              "cells": [{"row": 0, "col": 0, "text": "Agena terminal · 实时输出", "attributes": attrs},
                        {"row": 2, "col": 0, "text": "$ build running…", "attributes": attrs}]}
    page.evaluate("screen => partFixture.emit('pty',{type:'terminal',screen})", screen)
    page.wait_for_function("partFixture.terminals.get('pty').buffer.active.getLine(2)?.translateToString(true).includes('running')")
    changed = {**screen, "cells": [{"row": 2, "col": 0, "text": "$ build completed ✓", "attributes": attrs}]}
    page.evaluate("screen => partFixture.emit('pty',{type:'terminal_patch',base_cursor:{epoch:'00000000-0000-0000-0000-000000000001',sequence:1},screen,rows_changed:[2]})", changed)
    page.wait_for_function("partFixture.terminals.get('pty').buffer.active.getLine(2)?.translateToString(true).includes('completed')")
    assert page.evaluate("partFixture.terminals.get('pty').cols === 60 && partFixture.terminals.get('pty').rows === 6")
    assert page.evaluate("partFixture.terminals.get('pty').buffer.active.getLine(0).translateToString(true).includes('实时输出')")
    assert page.locator('[data-fixture="pty"] .content-terminal').bounding_box()["height"] < 160

    # Indexed ANSI colors follow the observer's theme without changing facts.
    page.evaluate("partFixture.theme(true)")
    page.wait_for_function("partFixture.terminals.get('pty').options.theme.green === '#a8cc8c'")
    assert re.fullmatch(r"#[0-9a-f]{6}", page.evaluate("partFixture.terminals.get('pty').options.theme.background"))
    page.screenshot(path=str(output / "part-streaming-running-dark.png"), full_page=True)
    page.evaluate("partFixture.theme(false)")
    page.wait_for_function("partFixture.terminals.get('pty').options.theme.green === '#276a3d'")
    page.screenshot(path=str(output / "part-streaming-running-light.png"), full_page=True)

    # Freeze a selected observation while the canonical resource advances.
    page.evaluate("partFixture.terminals.get('shell').select(2,0,5)")
    page.wait_for_function("partFixture.terminals.get('shell').getSelection().length > 0")
    before = page.evaluate("partFixture.terminals.get('shell').buffer.active.length")
    page.evaluate("partFixture.emit('shell',{type:'log',stream:'stdout',text:'received while selection is frozen\\n'})")
    page.wait_for_timeout(100)
    assert page.evaluate("partFixture.terminals.get('shell').buffer.active.length") == before
    shell.locator('[data-content-output] button').first.click()
    page.wait_for_function("!partFixture.terminals.get('shell').getSelection()")
    page.wait_for_function("partFixture.terminals.get('shell').buffer.active.getLine(6)?.translateToString(true).includes('frozen')")

    page.evaluate("partFixture.emit('shell',{type:'log',stream:'stdout',text:Array.from({length:100},(_,i)=>`retained line ${i}\\n`).join('')})")
    page.wait_for_function("partFixture.terminals.get('shell').buffer.active.baseY > 50")
    page.evaluate("partFixture.terminals.get('shell').scrollToTop()")
    page.wait_for_timeout(50)
    page.evaluate("partFixture.emit('shell',{type:'log',stream:'stdout',text:'new output while scrolled up\\n'})")
    page.wait_for_timeout(100)
    assert page.evaluate("partFixture.terminals.get('shell').buffer.active.viewportY === 0")
    shell.locator('[data-content-output] button').first.click()
    page.wait_for_function("partFixture.terminals.get('shell').buffer.active.viewportY === partFixture.terminals.get('shell').buffer.active.baseY")

    page.evaluate("window.savedPrefix = document.querySelector('[data-fixture=markdown] [data-markdown-segment]')")
    page.evaluate("partFixture.markdown('# Streamed answer\\n\\nStable first paragraph.\\n\\nLive tail grows with 中文 and **formatting**.')")
    page.wait_for_timeout(200)
    assert page.evaluate("savedPrefix === document.querySelector('[data-fixture=markdown] [data-markdown-segment]')")

    code = page.locator('[data-fixture="code"]')
    page.evaluate("partFixture.setCode(Array.from({length:40},(_,i)=>`code ${i}`).join('\\n'))")
    page.wait_for_timeout(100)
    code.get_by_role("button", name=re.compile("展开代码块")).first.click()
    page.evaluate("partFixture.setCode(Array.from({length:50},(_,i)=>`code ${i}`).join('\\n'))")
    page.wait_for_timeout(100)
    assert "code 49" in code.inner_text()
    code.get_by_role("button", name=re.compile("收起代码块")).first.click()
    page.evaluate("partFixture.setCode('now small')")
    page.wait_for_timeout(100)
    page.evaluate("partFixture.setCode(Array.from({length:40},(_,i)=>`code ${i}`).join('\\n'))")
    page.wait_for_timeout(100)
    assert "code 39" not in code.inner_text()

    page.evaluate("async () => { for (let i=0;i<40;i++) { partFixture.emit('shell',{type:'log',stream:'stdout',text:`latency:${i}:${performance.now().toFixed(3)}\\n`}); await new Promise(r=>setTimeout(r,25)); } }")
    page.wait_for_function("partFixture.metrics.length >= 40")
    page.evaluate("partFixture.emit('shell',{type:'log',stream:'stdout',text:'Build succeeded · 40 modules\\n'},'complete')")
    page.evaluate("partFixture.emit('pty',{type:'log',stream:'stdout',text:''},'complete')")
    page.wait_for_timeout(250)
    assert page.evaluate("savedShell === document.querySelector('[data-fixture=shell] .xterm') && partFixture.expanded.shell")
    assert "实时输出" not in shell.locator('[data-content-output]').inner_text()
    page.evaluate("partFixture.theme(true)")
    page.wait_for_function("partFixture.terminals.get('pty').options.theme.green === '#a8cc8c'")
    page.screenshot(path=str(output / "part-streaming-dark.png"), full_page=True)
    page.evaluate("partFixture.theme(false)")
    page.wait_for_function("partFixture.terminals.get('pty').options.theme.green === '#276a3d'")
    page.screenshot(path=str(output / "part-streaming-light.png"), full_page=True)
    metrics = sorted(item["ms"] for item in page.evaluate("partFixture.metrics"))
    result = {"scope": "controlled SSE / production Vue components / Xterm write / two animation frames",
              "samples": len(metrics), "p50_ms": metrics[len(metrics)//2],
              "p95_ms": metrics[len(metrics)*95//100], "max_ms": max(metrics),
              "verified": ["ANSI, CR, Chinese and emoji", "exact whitespace copy", "stdout/stderr",
                           "PTY snapshot and row patch", "selection and scroll freeze / follow",
                           "stable completion DOM and expansion", "Markdown prefix DOM", "CodeBlock user state",
                           "dark and light ANSI palette, native PTY height and screenshots"]}
    (output / "part-streaming-browser.json").write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n")
    print(json.dumps(result, ensure_ascii=False))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default="/tmp/agena-part-visual")
    args = parser.parse_args()
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        executable = os.environ.get("AGENA_WEBKIT_EXECUTABLE")
        browser = p.webkit.launch(headless=True, **({"executable_path": executable} if executable else {}))
        try:
            page = browser.new_page(viewport={"width": 1280, "height": 1050}, device_scale_factor=1)
            page.set_default_timeout(15000)
            try:
                verify(page, output)
            except Exception:
                print(page.evaluate("[...partFixture.terminals].map(([id,t])=>({id,rows:Array.from({length:Math.min(t.buffer.active.length,120)},(_,i)=>t.buffer.active.getLine(i)?.translateToString(true)),selection:t.getSelection(),viewport:t.buffer.active.viewportY}))"), flush=True)
                page.screenshot(path=str(output / "part-streaming-failure.png"), full_page=True)
                raise
        finally:
            browser.close()


if __name__ == "__main__":
    main()
