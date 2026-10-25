#!/usr/bin/env python3
"""Exercise plugin permissions through an isolated real server and local fixtures.

No real credentials, external network requests, user configuration, or user
sessions are used. See result.json and http.jsonl in the fresh output directory.
"""
from __future__ import annotations

import argparse
import http.server
import json
from pathlib import Path
import queue
import socket
import subprocess
import threading
import time
import traceback
from urllib.parse import urlsplit
from typing import Any

import http_experience as fixture


class LocalServices:
    def __init__(self) -> None:
        self.calls: queue.Queue[tuple[str, dict[str, Any]]] = queue.Queue()
        self.search_requests = 0
        self.page_requests = 0
        self.proxy_requests = 0
        self.reviews = 0
        self.allow_auto = True
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_: Any) -> None:
                pass

            def do_GET(self) -> None:
                path = self.path
                if path.startswith('http://'):
                    owner.proxy_requests += 1
                    path = urlsplit(path).path
                if path.startswith('/page/'):
                    owner.page_requests += 1
                    raw = b'<html><title>Permission fixture</title><main><h1>Authorized local page</h1><p>This page is a controlled regression fixture with enough readable evidence for extraction.</p></main></html>'
                    self.send_response(200)
                    self.send_header('Content-Type', 'text/html')
                    self.send_header('Content-Length', str(len(raw)))
                    self.end_headers()
                    self.wfile.write(raw)
                    return
                if path.startswith('/search'):
                    owner.search_requests += 1
                    body = {"results": [{"url": "https://example.org/permission-test", "title": "Authorized fixture", "content": "controlled result"}]}
                else:
                    body = {"object": "list", "data": [{"id": "fake-model", "object": "model"}]}
                raw = json.dumps(body).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def do_POST(self) -> None:
                try:
                    request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    tools = request.get('tools', [])
                    is_review = any('approve_action' in json.dumps(tool) for tool in tools)
                    if is_review:
                        owner.reviews += 1
                        call = None
                        text = json.dumps({"should_block": not owner.allow_auto, "reason": "controlled local approval" if owner.allow_auto else "Destructive or irreversible damage: controlled local rejection"})
                    else:
                        try:
                            tool, args = owner.calls.get_nowait()
                        except queue.Empty:
                            call = None
                            text = "Permission fixture finished."
                        else:
                            call = {"type": "function_call", "id": f"fc_{time.time_ns()}", "call_id": f"call_{time.time_ns()}", "name": "tools_call", "arguments": json.dumps({"tool": tool, "input": args})}
                            text = ''
                    if not request.get('stream', False):
                        raw = json.dumps({"id": f"resp_{time.time_ns()}", "status": "completed", "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": text}]}], "usage": {"input_tokens": 5, "output_tokens": 3, "total_tokens": 8}}).encode()
                        self.send_response(200)
                        self.send_header('Content-Type', 'application/json')
                        self.send_header('Content-Length', str(len(raw)))
                        self.end_headers()
                        self.wfile.write(raw)
                        return
                    self.send_response(200)
                    self.send_header('Content-Type', 'text/event-stream')
                    self.send_header('Connection', 'close')
                    self.end_headers()
                    self.close_connection = True
                    events = ([{"type": "response.output_item.added", "output_index": 0, "item": {**call, "arguments": ""}}, {"type": "response.output_item.done", "output_index": 0, "item": call}] if call else [{"type": "response.output_text.delta", "delta": text}])
                    events.append({"type": "response.completed", "response": {"id": f"resp_{time.time_ns()}", "status": "completed", "usage": {"input_tokens": 5, "output_tokens": 3, "total_tokens": 8}}})
                    for event in events:
                        self.wfile.write(b'data: ' + json.dumps(event).encode() + b'\n\n')
                        self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f'http://127.0.0.1:{self.server.server_port}'

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


def tool_parts(api: fixture.Api, sid: int) -> list[dict[str, Any]]:
    return [part for part in api.parts(sid) if part['kind'] == 'tool_call']


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    services = LocalServices()
    secondary = LocalServices()
    trace = fixture.JsonLines(out / 'http.jsonl')
    process = None
    result: dict[str, Any] = {'status': 'failed', 'cases': [], 'binary_sha256': fixture.file_digest(binary)}
    log = (out / 'server.log').open('wb')
    try:
        workspace = fixture.prepare_configuration(out, services.url + '/v1')
        config_path = workspace / '.agena/agena.json'
        config = json.loads(config_path.read_text())
        config['plugins'] = {'list': {'agena.web': {'package': {'kind': 'static'}, 'settings': {'search': {'provider': 'searxng', 'endpoint': services.url + '/search', 'allow_private_endpoint': True}, 'browser': {'enabled': False}}}}}
        fixture.write_json(config_path, config)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        environment = fixture.clean_environment(out)
        environment['AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES'] = '1'
        process = subprocess.Popen([str(binary), '--data-dir', str(out / 'data'), 'server', '--host', '127.0.0.1', '--port', str(port), '--workspace', str(workspace)], cwd=out / 'process-cwd', env=environment, stdout=log, stderr=subprocess.STDOUT)
        api = fixture.Api(f'http://127.0.0.1:{port}', 25, trace)
        result['server'] = fixture.wait_server(process, api, out / 'data/run/server.json')
        wid = api.call('POST', '/api/v1/workspaces/resolve', {'path': str(workspace), 'create_if_missing': True})[0]['id']

        def session(name: str, permission: dict[str, Any], tool: str, inputs: dict[str, Any]) -> int:
            sid = fixture.create_session(api, wid, name)
            api.call('PUT', f'/api/v1/sessions/{sid}/permission', {'permission': permission})
            services.calls.put((tool, inputs))
            fixture.submit(api, sid, name)
            return sid

        def reply(sid: int, kind: str) -> None:
            pending = fixture.permission_wait(api, sid)
            api.call('POST', f'/api/v1/sessions/{sid}/permission-replies', {'reply': {'request_id': pending['request_id'], 'kind': kind, 'reason': 'isolated permission regression probe'}})

        def complete(sid: int, name: str, failure: str | None = None) -> None:
            fixture.wait_idle(api, sid)
            parts = tool_parts(api, sid)
            fixture.require(parts, f'{name}: tool did not run')
            fixture.write_json(out / f'{name}.json', {'state': api.state(sid), 'parts': parts})
            if failure:
                fixture.require(failure in json.dumps(parts).lower(), f'{name}: expected {failure}: {parts}')
                failures = [part['content'].get('error', {}).get('failure', {}) for part in parts if part.get('state') == 'failed']
                fixture.require(failures and all(item.get('category') == 'permission_denied' for item in failures), f'{name}: lost permission classification: {failures}')
            else:
                fixture.require(all(part.get('state') not in ('failed', 'error') for part in parts), f'{name}: unexpected tool failure: {parts}')
            result['cases'].append({'name': name, 'session_id': sid, 'passed': True})

        # All filesystem targets get an exact rule, overriding the shipped tmp allowance.
        for mode, decision in [('ask', 'allow_once'), ('ask', 'deny_once'), ('deny', None)]:
            name = f'write_{mode}_{decision or "policy"}'
            path = workspace / f'{name}/file.txt'
            permission = {'path': {'rules': {str(path): {'read': 'allow', 'write': mode}}}}
            sid = session(name, permission, 'fs.write', {'path': str(path), 'content': name, 'create_parents': True})
            if decision:
                fixture.permission_wait(api, sid)
                time.sleep(0.15)
                fixture.require(not path.parent.exists(), f'{name}: mutation before approval')
                reply(sid, decision)
            complete(sid, name, 'declined' if decision == 'deny_once' else 'denied' if mode == 'deny' else None)
            fixture.require(path.exists() == (decision == 'allow_once'), f'{name}: wrong write effect')

        # A directory grant must not override a narrower child rule.
        discovery = workspace / 'discovery'
        discovery.mkdir()
        blocked = discovery / 'blocked.rs'
        blocked.write_text('fn secret_marker() {}\n')
        child_permission = {'path': {'rules': {str(discovery): {'read': 'allow'}, str(blocked): {'read': 'deny'}}}}
        for tool, inputs in [('fs.glob', {'path': str(discovery), 'pattern': '**/*'}), ('fs.grep', {'path': str(discovery), 'pattern': 'secret_marker'}), ('code.search_ast', {'path': str(discovery), 'language': 'rust', 'pattern': 'fn $NAME() { $$$ }'})]:
            name = tool.replace('.', '_') + '_child_denied'
            sid = session(name, child_permission, tool, inputs)
            complete(sid, name, 'denied')

        # Concrete records, including command aliases, cannot bypass child rules.
        skill = workspace / '.agena/skills/permission-fixture/SKILL.md'
        skill.parent.mkdir(parents=True)
        skill.write_text('---\nname: permission-fixture\ndescription: controlled permission fixture\naliases: [permission-alias]\n---\nPrivate fixture instructions.\n')
        sid = session('commands_child_denied', {'path': {'rules': {str(skill): {'read': 'deny'}}}}, 'commands.get', {'name': 'permission-alias'})
        complete(sid, 'commands_child_denied', 'denied')

        sid = session('memory_write_allowed', {}, 'memory.write', {'name': 'permission-record', 'content': 'controlled private memory'})
        complete(sid, 'memory_write_allowed')
        records = list((out / 'data/workspaces').rglob('permission-record.md'))
        fixture.require(len(records) == 1, f'memory record was not created in one workspace: {records}')
        record = records[0]
        for tool, inputs in [('memory.get', {'name': 'permission-record.md'}), ('memory.list', {}), ('memory.search', {'query': 'controlled'})]:
            name = tool.replace('.', '_') + '_child_denied'
            sid = session(name, {'path': {'rules': {str(record): {'read': 'deny'}}}}, tool, inputs)
            complete(sid, name, 'denied')
        sid = session('memory_index_write_denied', {'path': {'rules': {str(record.parent / 'MEMORY.md'): {'write': 'deny'}}}}, 'memory.write', {'name': 'blocked-record', 'content': 'must not be written'})
        complete(sid, 'memory_index_write_denied', 'denied')
        fixture.require(not (record.parent / 'blocked-record.md').exists(), 'memory record was written before index permission')

        # Ask must block outbound I/O; AllowOnce opens it; Deny and Auto rejection never send.
        for mode, decision in [('ask', 'allow_once'), ('ask', 'deny_once'), ('deny', None), ('auto', None)]:
            name = f'search_{mode}_{decision or "policy"}'
            before = services.search_requests
            sid = session(name, {'network': {'loopback': mode}}, 'web.search', {'query': name})
            if decision:
                fixture.permission_wait(api, sid)
                time.sleep(0.15)
                fixture.require(services.search_requests == before, f'{name}: network request before approval')
                reply(sid, decision)
            complete(sid, name, 'declined' if decision == 'deny_once' else 'denied' if mode == 'deny' else None)
            fixture.require(services.search_requests - before == int(decision == 'allow_once' or mode == 'auto'), f'{name}: wrong outbound effect')
            if mode == 'ask' and decision == 'allow_once':
                approved_session = sid

        before = services.search_requests
        services.calls.put(('web.search', {'query': 'new call needs a new grant'}))
        fixture.submit(api, approved_session, 'AllowOnce must not authorize a later tool call')
        reply(approved_session, 'deny_once')
        complete(approved_session, 'search_allow_once_does_not_leak', 'declined')
        fixture.require(services.search_requests == before, 'AllowOnce leaked to another call')

        before_pages = services.page_requests + secondary.page_requests
        urls = [services.url + '/page/a', secondary.url + '/page/b']
        sid = session('fetch_many_serial_approvals', {'network': {'loopback': 'ask'}}, 'web.fetch_many', {'urls': urls, 'render_js': False})
        pending = fixture.permission_wait(api, sid)
        fixture.require(services.page_requests + secondary.page_requests == before_pages, 'batch sent requests before approval')
        reply(sid, 'allow_once')
        next_pending = fixture.permission_wait(api, sid)
        fixture.require(next_pending['request_id'] != pending['request_id'], 'batch reused an unrelated endpoint grant')
        reply(sid, 'allow_once')
        complete(sid, 'fetch_many_serial_approvals', 'denied')
        # Approval does not override the independent public-address boundary.
        fixture.require(services.page_requests + secondary.page_requests == before_pages, 'batch bypassed the public-address guard')

        sid = session('fetch_many_denied', {'network': {'loopback': 'deny'}}, 'web.fetch_many', {'urls': urls, 'render_js': False})
        complete(sid, 'fetch_many_denied', 'denied')
        fixture.require(services.page_requests + secondary.page_requests == before_pages, 'denied batch sent a request')

        services.allow_auto = False
        before = services.search_requests
        sid = session('search_auto_rejected', {'network': {'loopback': 'auto'}}, 'web.search', {'query': 'search_auto_rejected'})
        complete(sid, 'search_auto_rejected', 'denied')
        fixture.require(services.search_requests == before, 'Auto rejection sent a network request')

        before = services.search_requests
        sid = session('search_cancelled', {'network': {'loopback': 'ask'}}, 'web.search', {'query': 'search_cancelled'})
        fixture.permission_wait(api, sid)
        api.call('POST', f'/api/v1/sessions/{sid}/cancel', {})
        fixture.wait_idle(api, sid)
        fixture.require(services.search_requests == before, 'Cancelled approval sent a network request')
        fixture.require(not fixture.pending_permissions(api.state(sid)), 'Cancelled interaction remains pending')
        result['cases'].append({'name': 'search_cancelled', 'session_id': sid, 'passed': True})
        fixture.require(services.reviews >= 2, 'Auto approval model was not called')

        # A trusted HTTP proxy resolves this intentionally nonexistent public
        # hostname. Authorization must still run against the original URL,
        # before the proxy sees a single request.
        proxy_settings = {'search': {'provider': 'searxng',
                                    'endpoint': 'http://proxy-search.fixture.invalid/search'},
                          'network': {'proxy': services.url}, 'browser': {'enabled': False}}
        config['plugins']['list']['agena.web']['settings'] = proxy_settings
        fixture.write_json(config_path, config)
        generation = api.get('/api/v1/health')['generation']
        api.call('POST', '/api/v1/runtime/reload', {})
        fixture.poll(lambda: api.get('/api/v1/health')['generation'] > generation,
                     api.timeout, 'proxy settings did not reload')
        effective = api.get('/api/v1/plugins/agena.web/settings')['effective']
        fixture.require(effective['search']['endpoint'] == proxy_settings['search']['endpoint'],
                        'workspace search settings did not reload')
        for mode, decision in [('ask', 'allow_once'), ('ask', 'deny_once'), ('deny', None), ('auto', None)]:
            name = f'proxy_search_{mode}_{decision or "policy"}'
            before = services.proxy_requests
            services.allow_auto = False
            sid = session(name, {'network': {'internet': mode}}, 'web.search', {'query': name})
            if decision:
                fixture.permission_wait(api, sid)
                time.sleep(0.15)
                fixture.require(services.proxy_requests == before, f'{name}: contacted proxy before approval')
                reply(sid, decision)
            complete(sid, name, None if decision == 'allow_once' else 'declined' if decision == 'deny_once' else 'denied')
            fixture.require(services.proxy_requests - before == int(decision == 'allow_once'),
                            f'{name}: wrong proxy effect')
        result.update(status='passed', search_requests=services.search_requests, automatic_reviews=services.reviews)
    except BaseException:
        result['error'] = traceback.format_exc()
        raise
    finally:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        log.close()
        trace.close()
        services.close()
        secondary.close()
        fixture.write_json(out / 'result.json', result)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
