#!/usr/bin/env python3
"""Verify real web search through an existing trusted Fake-IP TUN network.

Uses isolated Agena data/workspaces and a loopback mock model; search and fetch
use the real internet. Does not change DNS, proxy settings or user sessions.
Requires the caller to provide the exact synthetic CIDRs owned by their TUN.
"""
from __future__ import annotations

import argparse
import ipaddress
import json
from pathlib import Path
import socket
import subprocess
import traceback

import http_experience as fixture
from plugin_permissions import LocalServices, tool_parts


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--fake-ip-range', action='append', required=True)
    parser.add_argument('--browser', action='store_true', help='also verify rendering with installed Chrome/Chromium')
    parser.add_argument('--proxy', help='also verify real HTTP search/fetch using this proxy without Fake-IP exceptions')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    networks = [ipaddress.ip_network(value) for value in args.fake_ip_range]
    dns = sorted({item[4][0] for item in socket.getaddrinfo('www.bing.com', 443, type=socket.SOCK_STREAM)})
    fixture.require(any(any(ipaddress.ip_address(value) in net for net in networks) for value in dns),
                    f'the current DNS does not use the supplied synthetic ranges: {dns}')
    result = {'status': 'failed', 'binary_sha256': fixture.file_digest(binary),
              'dns': dns, 'fake_ip_ranges': args.fake_ip_range, 'cases': []}
    try:
        for label in ['strict', 'configured'] + (['proxy'] if args.proxy else []):
            enabled = label != 'strict'
            phase = out / label
            phase.mkdir()
            services = LocalServices()
            trace = fixture.JsonLines(phase / 'http.jsonl')
            process = None
            log = (phase / 'server.log').open('wb')
            try:
                workspace = fixture.prepare_configuration(phase, services.url + '/v1')
                path = workspace / '.agena/agena.json'
                config = json.loads(path.read_text())
                config['plugins'] = {'list': {'agena.web': {
                    'package': {'kind': 'static'},
                    'settings': {'network': {'fake_ip_ranges': args.fake_ip_range if label == 'configured' else [],
                                             **({'proxy': args.proxy} if label == 'proxy' else {})},
                                 'browser': {'enabled': args.browser and label == 'configured'}},
                }}}
                fixture.write_json(path, config)
                with socket.socket() as sock:
                    sock.bind(('127.0.0.1', 0))
                    port = sock.getsockname()[1]
                env = fixture.clean_environment(phase)
                env['AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES'] = '1'
                process = subprocess.Popen(
                    [str(binary), '--data-dir', str(phase / 'data'), 'server', '--host', '127.0.0.1',
                     '--port', str(port), '--workspace', str(workspace)],
                    cwd=phase / 'process-cwd', env=env, stdout=log, stderr=subprocess.STDOUT)
                api = fixture.Api(f'http://127.0.0.1:{port}', 65, trace)
                fixture.wait_server(process, api, phase / 'data/run/server.json')
                wid = api.call('POST', '/api/v1/workspaces/resolve',
                               {'path': str(workspace), 'create_if_missing': True})[0]['id']

                def run(name: str, tool: str, inputs: dict, *, denied: bool = False,
                        internet: str = 'allow', needs_results: bool = False) -> None:
                    sid = fixture.create_session(api, wid, name)
                    api.call('PUT', f'/api/v1/sessions/{sid}/permission', {'permission': {
                        'tools': {'default': 'allow'},
                        'network': {'internet': internet, 'private': 'allow', 'loopback': 'allow'},
                    }})
                    services.calls.put((tool, inputs))
                    fixture.submit(api, sid, name)
                    fixture.wait_idle(api, sid)
                    parts = tool_parts(api, sid)
                    fixture.write_json(phase / f'{name}.json', {'state': api.state(sid), 'parts': parts})
                    fixture.require(len(parts) == 1, f'{name}: expected one tool call')
                    part = parts[0]
                    if denied:
                        failure = part['content'].get('error', {}).get('failure', {})
                        fixture.require(part['state'] == 'failed' and failure.get('category') == 'permission_denied',
                                        f'{name}: expected typed policy denial: {part}')
                    else:
                        fixture.require(part['state'] == 'completed', f'{name}: {part}')
                        if needs_results:
                            blocks = part.get('presentation', {}).get('blocks', [])
                            fixture.require(any(block.get('type') == 'table' and block.get('rows') for block in blocks),
                                            f'{name}: search completed without real result rows')
                    result['cases'].append({'phase': label, 'name': name, 'passed': True, 'summary': part.get('summary', ''),
                                            'presentation': part.get('presentation')})

                if not enabled:
                    run('strict_fake_dns_denied', 'web.search', {'query': 'League of Legends 2026 schedule',
                                                               'engine': 'bing'}, denied=True)
                else:
                    run('real_multi_engine_search', 'web.search',
                        {'query': '英雄联盟 赛事 赛程 2026 LPL LCK 世界赛', 'max_results': 15}, needs_results=True)
                    run('real_public_fetch', 'web.fetch', {'url': 'https://example.com/', 'render_js': False})
                    if args.browser and label == 'configured':
                        run('real_browser_fetch', 'web.fetch', {'url': 'https://example.com/', 'render_js': True})
                    run('internet_deny_preserved', 'web.search', {'query': 'must be denied', 'engine': 'bing'},
                        denied=True, internet='deny')
                    for name, url in [('literal_fake_v4_denied', 'http://198.18.0.52/'),
                                      ('literal_fake_v6_denied', 'http://[fdfe:dcba:9876::59]/'),
                                      ('real_loopback_denied', services.url + '/page/private')]:
                        run(name, 'web.fetch', {'url': url, 'render_js': False}, denied=True)
                    fixture.require(services.page_requests == 0, 'the private endpoint received a request')
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
        result['status'] = 'passed'
    except BaseException:
        result['error'] = traceback.format_exc()
        raise
    finally:
        fixture.write_json(out / 'result.json', result)
    print(json.dumps({key: value if key != 'cases' else [
        {field: case[field] for field in ('phase', 'name', 'passed', 'summary')} for case in value]
        for key, value in result.items()}, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
