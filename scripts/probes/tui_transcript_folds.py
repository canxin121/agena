#!/usr/bin/env python3
"""Verify recent-only reply folding and batch shortcuts in an isolated real PTY."""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import socket
import subprocess
import threading
import time
import traceback

import http_experience as fixture
import tui_experience as tui
from tui_session_marks import DelayedProxy


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--cols', type=int, default=80)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    trace = fixture.JsonLines(out / 'http.jsonl')
    log = (out / 'server.log').open('wb')
    result = {'status': 'failed', 'binary_sha256': fixture.file_digest(binary), 'cols': args.cols, 'cases': []}
    process = provider = proxy = client = None
    tui.fixture = fixture
    tui.ROWS, tui.COLS = 40, args.cols
    try:
        provider = fixture.Provider(30, trace)
        workspace = fixture.prepare_configuration(out, provider.base_url)
        for index in range(40):
            (workspace / f'fold-item-{index:03}.txt').write_text('owned fold fixture\n')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        env = fixture.clean_environment(out)
        env['AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES'] = '1'
        process = subprocess.Popen(
            [str(binary), '--data-dir', str(out / 'data'), 'server', '--host', '127.0.0.1',
             '--port', str(port), '--workspace', str(workspace)],
            cwd=out / 'process-cwd', env=env, stdout=log, stderr=subprocess.STDOUT)
        api = fixture.Api(f'http://127.0.0.1:{port}', 30, trace)
        fixture.wait_server(process, api, out / 'data/run/server.json')
        wid = api.call('POST', '/api/v1/workspaces/resolve', {'path': str(workspace), 'create_if_missing': True})[0]['id']
        sid = fixture.create_session(api, wid, 'Reply folding fixture')
        tools = provider.plan('fold_tools', 'fold-tools')
        answer = provider.plan('fold_answer', 'continuation', payloads=['FOLD_FINAL_ANSWER\n'])
        fixture.submit(api, sid, 'Read the owned fold fixture files.')
        fixture.wait_idle(api, sid)
        provider.assert_step(tools)
        provider.assert_step(answer)
        parts = api.parts(sid)
        fixture.require(sum(p['kind'] == 'tool_call' for p in parts) == 40, 'fixture did not persist all tool parts')
        proxy = DelayedProxy(port, trace)
        threading.Thread(target=proxy.serve_forever, daemon=True).start()
        proxied_api = fixture.Api(f'http://127.0.0.1:{proxy.server_port}', 30, trace)
        client = tui.Pty(binary, proxied_api, workspace, sid, out / 'tui', env)
        client.wait_text('FOLD_FINAL_ANSWER', 'opened')
        client.send(b'\x1b', 'view-mode')
        time.sleep(0.15)
        client.send(b'gg', 'view-reply-start')

        def files():
            return sorted(set(int(value) for value in re.findall(r'fold-item-(\d{3})', client.text())))

        def checkpoint(label):
            fixture.require(not client.failure, str(client.failure))
            (client.directory / f'{label}.txt').write_text(client.text())
            result['cases'].append(label)

        def wait_files(expected):
            fixture.poll(lambda: files() == expected, 10, f'wrong recent file set; screen={client.text()}')
            fixture.require('EARLIEST_REASONING_SENTINEL' not in client.text(), 'oldest reasoning leaked through folding')

        def enter_count(value, label):
            # Separate editing from submission, just as a human would. The
            # terminal's IME/paste guard treats a machine-speed text+Enter
            # burst as committed text, rather than a dialog submission.
            client.send(b'\x01\x0b' + str(value).encode(), label + '-type')
            time.sleep(0.2)
            client.send(b'\r', label + '-submit')

        def require_complete_history():
            # Count every tool across both viewport ends. Checking only that
            # the oldest reasoning appears misses a prematurely retired gap
            # which can still hide the first real tool.
            client.send(b'gg', 'inspect-history-prefix')
            client.wait_text('fold-item-000.txt', 'first-tool-revealed')
            seen = set(files())
            client.send(b'G', 'inspect-history-suffix')
            client.wait_text('FOLD_FINAL_ANSWER', 'history-suffix')
            seen.update(files())
            fixture.require(seen == set(range(40)), f'expanded history lost tools: {sorted(set(range(40)) - seen)}')

        client.wait_text('older parts hidden', 'fold-visible')
        client.wait_text('37 older parts hidden', 'hidden-count-includes-loaded-boundary-once')
        wait_files(list(range(36, 40)))
        fixture.require('choose count' not in client.text(), 'fold marker still asks for Enter')
        checkpoint('only-latest-five-parts-with-server-gap')

        # The editor is reached directly from the final answer, not its marker.
        client.send(b'G', 'select-answer-before-count-editor')
        client.send(b'\x1b[13;5u', 'ctrl-enter-edit-batch')
        client.wait_text('Older-part batch size', 'batch-editor')
        enter_count(2, 'save-batch-two')
        fixture.poll(lambda: 'Older-part batch size' not in client.text(), 5, 'batch editor did not close')
        client.send(b'gg', 'show-saved-batch')
        client.wait_text('load 2', 'batch-two-hint')
        wait_files(list(range(36, 40)))
        checkpoint('shortcut-edits-batch-without-expanding')

        client.send(b'gg\x1b[B\x1b[B\r', 'enter-reveal-two')
        client.send(b'gg', 'show-revealed-parts')
        wait_files(list(range(34, 40)))
        checkpoint('enter-reveals-exact-batch-from-fold-marker')

        client.send(b'\x1b[13;5u', 'edit-invalid-count')
        client.wait_text('Older-part batch size', 'invalid-editor')
        enter_count(0, 'reject-zero-count')
        client.wait_text('Enter a whole number', 'invalid-count-error')
        client.send(b'\x1b', 'cancel-invalid-count')
        fixture.poll(lambda: 'Older-part batch size' not in client.text(), 5, 'cancel did not close editor')
        client.wait_text('load 2', 'cancel-keeps-two')
        checkpoint('invalid-count-and-cancel-preserve-setting')

        client.send(b'\x1b[13;5u', 'edit-large-batch')
        client.wait_text('Older-part batch size', 'large-batch-editor')
        enter_count(50, 'save-batch-fifty')
        fixture.poll(lambda: 'Older-part batch size' not in client.text(), 5, 'large batch editor did not close')
        client.send(b'gg\x1b[B\x1b[B\r', 'load-local-and-server-prefix')
        client.send(b'gg', 'show-explicitly-revealed-history')
        client.wait_text('EARLIEST_REASONING_SENTINEL', 'all-older-parts-revealed')
        fixture.poll(lambda: 'older parts hidden' not in client.text(), 15, 'fold did not exhaust after explicit reveal')
        with proxy.lock:
            paths = [path for method, path in proxy.requests if method == 'GET']
        fixture.require(any('cursor=' in path and '/parts' in path for path in paths), 'server fold page was never fetched')
        require_complete_history()
        checkpoint('large-batch-fetches-missing-history-and-exhausts-fold')

        client.send(b'zCgg', 'collapse-all-reply-lists')
        client.wait_text('older parts hidden', 'recollapsed')
        wait_files(list(range(36, 40)))
        client.wait_text('load 50', 'batch-persists-after-collapse')
        checkpoint('collapse-keeps-latest-five-and-saved-batch')
        client.send(b'\x1b[13;6u', 'expand-all-ctrl-shift-enter')
        client.send(b'gg', 'show-expanded-history')
        client.wait_text('EARLIEST_REASONING_SENTINEL', 'expand-all-retained')
        fixture.require('older parts hidden' not in client.text(), 'expand-all shortcut left a local fold')
        require_complete_history()
        checkpoint('expand-all-shortcut-still-works')
        result['status'] = 'passed'
    except BaseException:
        result['error'] = traceback.format_exc()
    finally:
        if proxy is not None:
            proxy.reads.set()
            proxy.writes.set()
            proxy.project_writes.set()
        if client is not None:
            client.close()
        if proxy is not None:
            proxy.shutdown()
            proxy.server_close()
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if provider is not None:
            provider.close()
        trace.close()
        log.close()
        fixture.write_json(out / 'result.json', result)
    print(result)
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
