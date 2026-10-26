"""Run with AGENA_TEST_BINARY pointing to a freshly built Agena executable.

Uses an isolated data directory and loopback listener. No external OAuth client
or browser is needed; redirect responses are inspected without following them.
"""

import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import re
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time
import unittest
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, urlencode, urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


@unittest.skipUnless(os.environ.get('AGENA_TEST_BINARY'), 'Set AGENA_TEST_BINARY to run HTTP OAuth integration tests')
class McpOAuthTests(unittest.TestCase):
    DEFAULT_PASSWORD = 'isolated-shared-server-password'

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='agena-mcp-oauth-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.workspace = self.root / 'workspace'
        self.workspace.mkdir()
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            self.port = listener.getsockname()[1]
        self.base = f'http://127.0.0.1:{self.port}'
        self.opener = build_opener(ProxyHandler({}), NoRedirect())
        self.process = None
        self.addCleanup(self.stop)
        self.start(initial=True)

    def start(self, initial=False, password=DEFAULT_PASSWORD, extra_environment=None):
        environment = {key: value for key, value in os.environ.items() if not key.startswith('AGENA_MCP_')}
        environment.pop('AGENA_SERVER_PASSWORD', None)
        environment.pop('AGENA_SERVER_UI_PASSWORD', None)
        self.password = password
        self.management_headers = {}
        args = [str(Path(os.environ['AGENA_TEST_BINARY']).resolve()), '--data-dir', str(self.root / 'data'),
                'server', '--host', '127.0.0.1', '--port', str(self.port), '--workspace', str(self.workspace)]
        if initial:
            args += ['--mcp-enabled', 'false', '--mcp-server-url', f'127.0.0.1:{self.port}']
        if password is not None:
            environment['AGENA_SERVER_PASSWORD'] = password
        for key, value in (extra_environment or {}).items():
            if value is None:
                environment.pop(key, None)
            else:
                environment[key] = value
        self.process = subprocess.Popen(args, env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                self.fail(f'Isolated Agena server exited with status {self.process.returncode}')
            try:
                status, _, _ = self.request('/api/v1/server/mcp')
                if status in (200, 401):
                    if password is not None:
                        session = self.json_request('/auth/session', 'POST', {'password': password})
                        self.management_headers = {'Authorization': 'Bearer ' + session['token']}
                    return
            except (OSError, URLError):
                pass
            time.sleep(0.05)
        self.fail('Isolated Agena server did not become ready within 30 seconds')

    def stop(self):
        if self.process is None:
            return
        self.process.terminate()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        self.process = None

    def request(self, path, method='GET', payload=None, form=False, headers=None):
        request_headers = dict(headers or {})
        data = None
        if payload is not None:
            data = (urlencode(payload) if form else json.dumps(payload)).encode()
            request_headers['Content-Type'] = 'application/x-www-form-urlencoded' if form else 'application/json'
        request = Request(self.base + path, data=data, headers=request_headers, method=method)
        try:
            response = self.opener.open(request, timeout=5)
        except HTTPError as error:
            response = error
        with response:
            body = response.read()
            if 'application/json' in response.headers.get('Content-Type', ''):
                body = json.loads(body)
            return response.code, response.headers, body

    def json_request(self, path, method='GET', payload=None, expected=200, **kwargs):
        if path.startswith('/api/v1/server/mcp') and 'headers' not in kwargs:
            kwargs['headers'] = self.management_headers
        status, _, body = self.request(path, method, payload, **kwargs)
        self.assertEqual(status, expected, f'{method} {path}: {body!r}')
        return body

    def authorization_request(self):
        client = self.json_request('/register', 'POST', {
            'client_name': 'Isolated OAuth integration test',
            'redirect_uris': ['https://client.example.com/callback'],
            'token_endpoint_auth_method': 'none',
        }, expected=201)
        verifier = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk'
        challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('=')
        authorization = {
            'response_type': 'code', 'client_id': client['client_id'],
            'redirect_uri': 'https://client.example.com/callback', 'state': 'integration-state',
            'code_challenge': challenge, 'code_challenge_method': 'S256',
        }
        return verifier, authorization

    def authorize(self, password):
        verifier, authorization = self.authorization_request()
        status, headers, _ = self.request('/oauth/authorize?' + urlencode(authorization))
        self.assertEqual(status, 200)
        self.assertIn("frame-ancestors 'none'", headers['Content-Security-Policy'])
        status, _, _ = self.request('/oauth/authorize', 'POST', authorization | {'password': 'incorrect'}, form=True)
        self.assertEqual(status, 401)
        status, headers, _ = self.request('/oauth/authorize', 'POST', authorization | {'password': password}, form=True)
        self.assertEqual(status, 303)
        callback = urlsplit(headers['Location'])
        self.assertEqual(callback.hostname, 'client.example.com')
        parameters = parse_qs(callback.query)
        self.assertEqual(parameters['iss'], [self.base])
        self.assertEqual(parameters['state'], ['integration-state'])
        exchange = {
            'grant_type': 'authorization_code', 'client_id': authorization['client_id'],
            'redirect_uri': authorization['redirect_uri'], 'code': parameters['code'][0],
            'code_verifier': verifier,
        }
        tokens = self.json_request('/oauth/token', 'POST', exchange, form=True)
        self.json_request('/oauth/token', 'POST', exchange, form=True, expected=400)
        return authorization['client_id'], tokens

    def assert_token_accepted(self, token, expected):
        # Unknown tool names exercise authentication and routing without
        # executing a filesystem, shell, network or other operational tool.
        status, _, body = self.request('/mcp', 'POST', {
            'jsonrpc': '2.0', 'id': 1,
            'method': 'tools/call', 'params': {
                'name': '__oauth_integration_probe__', 'arguments': {},
                '_meta': {'io.modelcontextprotocol/protocolVersion': '2025-11-25'},
            },
        }, headers={
            'Authorization': f'Bearer {token}', 'Accept': 'application/json, text/event-stream',
            'MCP-Protocol-Version': '2025-11-25',
        })
        if expected:
            self.assertEqual(status, 404)
            self.assertIn('__oauth_integration_probe__', body.get('error', {}).get('message', ''))
        else:
            self.assertEqual(status, 200)
        challenged = 'mcp/www_authenticate' in body.get('result', {}).get('_meta', {})
        self.assertEqual(challenged, not expected)

    def test_authorization_rotation_revocation_and_single_address_control(self):
        control_path = '/api/v1/server/mcp'
        initial = self.json_request(control_path)
        self.assertFalse(initial['enabled'])
        self.assertEqual(initial['serverUrl'], self.base)
        self.json_request('/.well-known/oauth-authorization-server', expected=404)
        enabled = self.json_request(control_path, 'PUT', {'enabled': True})
        self.assertTrue(enabled['ready'])
        password = self.password
        self.assertTrue(enabled['oauth']['passwordConfigured'])
        self.assertEqual(enabled['oauth']['passwordSource'], 'server')
        self.assertNotIn('generatedOauthPassword', enabled)
        self.assertNotIn('fallbackToUiPassword', enabled['oauth'])
        for method in ('PUT', 'DELETE'):
            self.json_request(control_path + '/oauth/password', method, expected=404)
        metadata = self.json_request('/.well-known/oauth-authorization-server')
        self.assertEqual(metadata['issuer'], self.base)
        self.assertEqual(metadata['token_endpoint'], self.base + '/oauth/token')
        self.assertEqual(metadata['token_endpoint_auth_methods_supported'], ['none'])
        resource = self.json_request('/.well-known/oauth-protected-resource/mcp')
        self.assertEqual(resource['resource'], self.base + '/mcp')
        self.assertEqual(resource['authorization_servers'], [self.base])
        catalog = self.json_request('/mcp', 'POST', {
            'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list',
            'params': {'_meta': {'io.modelcontextprotocol/protocolVersion': '2025-11-25'}},
        }, headers={'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-11-25'})
        for tool in catalog['result']['tools']:
            self.assertIn(tool['securitySchemes'][0]['type'], ('oauth2', 'noauth'))
        forwarded = self.json_request('/.well-known/oauth-authorization-server', headers={
            'X-Forwarded-Host': 'attacker.example.com', 'X-Forwarded-Proto': 'https',
        })
        self.assertEqual(forwarded['issuer'], self.base)
        self.json_request('/.well-known/oauth-authorization-server', headers={'Host': 'attacker.example.com'}, expected=400)
        self.json_request('/.well-known/oauth-authorization-server', headers={'Origin': 'https://attacker.example.com'}, expected=403)
        self.assert_token_accepted('invalid', False)
        client, issued = self.authorize(password)
        self.assert_token_accepted(issued['access_token'], True)
        rotated = self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client, 'refresh_token': issued['refresh_token'],
        }, form=True)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client, 'refresh_token': issued['refresh_token'],
        }, form=True, expected=400)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client, 'refresh_token': rotated['refresh_token'],
        }, form=True, expected=400)
        client, revocable = self.authorize(password)
        self.json_request('/oauth/revoke', 'POST', {'token': revocable['refresh_token'], 'client_id': client}, form=True)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client, 'refresh_token': revocable['refresh_token'],
        }, form=True, expected=400)
        self.json_request('/oauth/revoke', 'POST', {'token': revocable['access_token'], 'client_id': client}, form=True)
        self.assert_token_accepted(revocable['access_token'], False)
        old_client, before_password_change = self.authorize(password)
        self.stop()
        self.start(password=password)
        self.assert_token_accepted(before_password_change['access_token'], True)
        after_restart = self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': old_client,
            'refresh_token': before_password_change['refresh_token'],
        }, form=True)
        password = 'isolated-integration-password-replacement'
        self.stop()
        self.start(password=password)
        self.assert_token_accepted(before_password_change['access_token'], False)
        self.assert_token_accepted(after_restart['access_token'], False)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': old_client,
            'refresh_token': after_restart['refresh_token'],
        }, form=True, expected=400)
        self.json_request('/auth/session', 'POST', {'password': self.DEFAULT_PASSWORD}, expected=401)
        _, old_authorization = self.authorization_request()
        status, _, _ = self.request('/oauth/authorize', 'POST', old_authorization | {
            'password': self.DEFAULT_PASSWORD,
        }, form=True)
        self.assertEqual(status, 401)
        _, before_disable = self.authorize(password)
        self.json_request(control_path, 'PUT', {'enabled': False})
        self.json_request('/.well-known/oauth-authorization-server', expected=404)
        self.json_request(control_path, 'PUT', {'enabled': True})
        self.assert_token_accepted(before_disable['access_token'], False)
        self.json_request(control_path, 'PUT', {
            'publicUrl': 'https://tunnel.example.com/v1/mcp/existing-tunnel',
            'oauthIssuerUrl': 'https://auth.example.com',
        })
        unified = self.json_request(control_path, 'PUT', {'serverUrl': 'agena.example.com:8443'})
        self.assertEqual(unified['serverUrl'], 'https://agena.example.com:8443')
        self.assertEqual(unified['resourceUrl'], 'https://agena.example.com:8443/mcp')
        self.assertEqual(unified['oauth']['issuer'], 'https://agena.example.com:8443')
        self.assertIsNone(unified['oauthIssuerUrl'])
        self.assertTrue(unified['ready'])
        self.assertFalse(unified['warnings'])
        self.json_request(control_path, 'PUT', {'serverUrl': 'http://192.168.1.2:3210'}, expected=400)
        cleared = self.json_request(control_path, 'PUT', {'serverUrl': None})
        self.assertIsNone(cleared['serverUrl'])
        self.assertEqual(cleared['resourceUrl'], self.base + '/mcp')
        self.assertEqual(cleared['oauth']['issuer'], self.base)

    def test_one_server_password_protects_management_and_oauth(self):
        password = self.password
        control_path = '/api/v1/server/mcp'
        self.json_request(control_path, expected=401, headers={})
        self.json_request('/auth/session', 'POST', {'password': 'incorrect'}, expected=401)
        enabled = self.json_request(control_path, 'PUT', {'enabled': True})
        self.assertTrue(enabled['ready'])
        self.assertEqual(enabled['oauth']['passwordSource'], 'server')
        self.assertNotIn('generatedOauthPassword', enabled)
        _, tokens = self.authorize(password)
        self.assert_token_accepted(tokens['access_token'], True)
        self.json_request(control_path, headers={'Authorization': 'Bearer ' + tokens['access_token']}, expected=401)
        remote = self.json_request(control_path, 'PUT', {
            'serverUrl': 'https://agena.example.com',
        })
        self.assertTrue(remote['ready'])
        self.assertFalse(remote['warnings'])

    def tui_environment(self):
        return {
            **{key: value for key, value in os.environ.items() if not key.startswith('AGENA_')},
            'AGENA_DATA_DIR': str(self.root / 'data'),
            'AGENA_SERVER_URL': self.base,
            'TERM': 'xterm-256color',
        }

    def test_tui_uses_server_password_environment_for_default_and_explicit_launch(self):
        binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        for command in ([], ['tui']):
            with self.subTest(command=command):
                result = subprocess.run(
                    [binary, *command], cwd=self.workspace,
                    env=self.tui_environment() | {'AGENA_SERVER_PASSWORD': self.password},
                    stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15,
                )
                # The authenticated handshake, runtime reads and workspace
                # resolution must all finish before terminal setup rejects pipes.
                self.assertIn('requires both stdin and stdout', result.stderr)
                self.assertNotIn('cannot connect TUI', result.stderr)
                self.assertNotIn(self.password, result.stdout + result.stderr)

        # A browser login token remains a compatible optional client credential.
        token = self.management_headers['Authorization'].removeprefix('Bearer ')
        result = subprocess.run(
            [binary, 'tui'], cwd=self.workspace,
            env=self.tui_environment() | {'AGENA_SERVER_TOKEN': token},
            stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15,
        )
        self.assertIn('requires both stdin and stdout', result.stderr)
        self.assertNotIn(token, result.stdout + result.stderr)

    def test_tui_reports_missing_or_incorrect_server_password_as_configuration(self):
        binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        for password, expected in ((None, 'requires a password'), ('incorrect-do-not-echo', 'password is incorrect')):
            with self.subTest(password_configured=password is not None):
                environment = self.tui_environment()
                if password is not None:
                    environment['AGENA_SERVER_PASSWORD'] = password
                result = subprocess.run(
                    [binary], cwd=self.workspace, env=environment,
                    stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('configuration error:', result.stderr)
                self.assertIn(expected, result.stderr)
                self.assertIn('AGENA_SERVER_PASSWORD', result.stderr)
                self.assertNotIn('UI authentication', result.stderr)
                if password is not None:
                    self.assertNotIn(password, result.stdout + result.stderr)

    @unittest.skipUnless(os.name == 'posix', 'PTY password input requires POSIX')
    def test_tui_password_prompt_hides_input_and_restores_terminal_on_failure_or_cancel(self):
        import pty
        import select
        import termios

        binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        for entered, expected in (
            (b'wrong-hidden-password\r', 'password is incorrect'),
            (b'cancel-hidden-password\x03', 'password entry cancelled'),
        ):
            with self.subTest(expected=expected):
                master, slave = pty.openpty()
                before = termios.tcgetattr(slave)
                process = subprocess.Popen(
                    [binary, 'tui'], cwd=self.workspace, env=self.tui_environment(),
                    stdin=slave, stdout=slave, stderr=slave, start_new_session=True,
                )
                try:
                    output = bytearray()
                    sent = False
                    deadline = time.monotonic() + 15
                    while time.monotonic() < deadline:
                        if select.select([master], [], [], 0.1)[0]:
                            output.extend(os.read(master, 65536))
                        if not sent and b'Server password: ' in output:
                            os.write(master, entered)
                            sent = True
                        if process.poll() is not None:
                            while select.select([master], [], [], 0)[0]:
                                output.extend(os.read(master, 65536))
                            break
                    self.assertIsNotNone(process.poll(), 'password prompt did not finish')
                    self.assertTrue(sent, output.decode(errors='replace'))
                    self.assertIn(expected.encode(), output)
                    self.assertNotIn(entered.split(b'\r')[0].split(b'\x03')[0], output)
                    self.assertEqual(termios.tcgetattr(slave), before)
                finally:
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=5)
                    os.close(master)
                    os.close(slave)

    def test_client_metadata_uses_configured_proxy_dns_and_still_rejects_local_destinations(self):
        connections = []

        class ProxyFixture(BaseHTTPRequestHandler):
            def do_CONNECT(self):
                connections.append(self.path)
                self.send_response(502)
                self.send_header('Content-Length', '0')
                self.end_headers()

            def log_message(self, *args):
                pass

        proxy = ThreadingHTTPServer(('127.0.0.1', 0), ProxyFixture)
        worker = threading.Thread(target=proxy.serve_forever, daemon=True)
        worker.start()
        self.addCleanup(proxy.server_close)
        self.addCleanup(proxy.shutdown)
        self.stop()
        environment = {name: None for name in (
            'http_proxy', 'HTTP_PROXY', 'https_proxy', 'HTTPS_PROXY',
            'all_proxy', 'ALL_PROXY', 'no_proxy', 'NO_PROXY',
        )}
        environment['https_proxy'] = f'http://127.0.0.1:{proxy.server_port}'
        self.start(extra_environment=environment)
        self.json_request('/api/v1/server/mcp', 'PUT', {'enabled': True})
        _, authorization = self.authorization_request()
        # This hostname cannot resolve locally. CONNECT must reach the selected
        # proxy, whose deliberate failure still rejects the unvalidated client.
        status, headers, body = self.request('/oauth/authorize?' + urlencode(authorization | {
            'client_id': 'https://metadata-cannot-resolve.invalid/client.json',
        }))
        self.assertEqual(status, 400)
        self.assertEqual(body['error'], 'invalid_client')
        self.assertNotIn('Location', headers)
        # Provider version discovery can also use this process-wide proxy at
        # startup. Check the metadata destination independently of that traffic.
        self.assertEqual(connections.count('metadata-cannot-resolve.invalid:443'), 1)
        for client_id in (
            'https://127.0.0.1/client.json', 'https://[::1]/client.json',
            'https://198.18.1.80/client.json', 'https://169.254.169.254/client.json',
            'https://10.0.0.5/client.json', 'https://service.local/client.json',
        ):
            status, _, body = self.request('/oauth/authorize?' + urlencode(authorization | {
                'client_id': client_id,
            }))
            self.assertEqual(status, 400)
            self.assertEqual(body['error'], 'invalid_client')
        self.assertEqual(connections.count('metadata-cannot-resolve.invalid:443'), 1)
        self.assertFalse(set(connections).intersection({
            '127.0.0.1:443', '[::1]:443', '198.18.1.80:443',
            '169.254.169.254:443', '10.0.0.5:443', 'service.local:443',
        }))

    def test_login_and_oauth_share_failed_password_limit(self):
        self.json_request('/api/v1/server/mcp', 'PUT', {'enabled': True})
        _, authorization = self.authorization_request()
        for _ in range(7):
            self.json_request('/auth/session', 'POST', {'password': 'incorrect'}, expected=401)
        status, headers, _ = self.request('/oauth/authorize', 'POST', authorization | {
            'password': 'incorrect',
        }, form=True)
        self.assertEqual(status, 429)
        self.assertGreater(int(headers['Retry-After']), 0)
        self.json_request('/auth/session', 'POST', {'password': self.password}, expected=429)

    def test_retired_password_environment_cannot_silently_disable_login(self):
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith('AGENA_MCP_')}
        environment.pop('AGENA_SERVER_PASSWORD', None)
        environment['AGENA_SERVER_UI_PASSWORD'] = 'legacy-password-must-not-be-logged'
        result = subprocess.run([
            str(Path(os.environ['AGENA_TEST_BINARY']).resolve()),
            '--data-dir', str(self.root / 'legacy-launch'), 'server',
            '--host', '127.0.0.1', '--port', '0', '--workspace', str(self.workspace),
        ], env=environment, capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        diagnostics = result.stdout + result.stderr
        self.assertIn('AGENA_SERVER_UI_PASSWORD has been renamed to AGENA_SERVER_PASSWORD', diagnostics)
        self.assertNotIn(environment['AGENA_SERVER_UI_PASSWORD'], diagnostics)

    def test_removing_server_password_revokes_grants_and_never_generates_an_mcp_password(self):
        control_path = '/api/v1/server/mcp'
        self.json_request(control_path, 'PUT', {'enabled': True})
        client, tokens = self.authorize(self.password)
        self.stop()
        self.start(password=None)
        session = self.json_request('/auth/session')
        self.assertTrue(session['disabled'])
        control = self.json_request(control_path)
        self.assertFalse(control['ready'])
        self.assertFalse(control['oauth']['passwordConfigured'])
        self.assertNotIn('generatedOauthPassword', control)
        self.assertTrue(any('AGENA_SERVER_PASSWORD' in warning for warning in control['warnings']))
        self.assert_token_accepted(tokens['access_token'], False)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client,
            'refresh_token': tokens['refresh_token'],
        }, form=True, expected=400)
        # Password removal also clears registrations; a newly registered client
        # receives an explicit unavailable response rather than a new password.
        _, authorization = self.authorization_request()
        status, _, body = self.request('/oauth/authorize', 'POST', authorization | {
            'password': self.DEFAULT_PASSWORD,
        }, form=True)
        self.assertEqual(status, 503)
        self.assertIn(b'password', body.lower())
        self.stop()
        self.start()
        self.assert_token_accepted(tokens['access_token'], False)

    def test_legacy_mcp_password_is_discarded_and_its_grants_are_revoked(self):
        self.json_request('/api/v1/server/mcp', 'PUT', {'enabled': True})
        _, tokens = self.authorize(self.password)
        self.stop()
        with sqlite3.connect(self.root / 'data/agena.db') as database:
            control = json.loads(database.execute(
                "SELECT value_json FROM agena_server_state WHERE key='mcp.server.control'",
            ).fetchone()[0])
            old_verifier = json.loads(database.execute(
                "SELECT value_json FROM agena_server_state WHERE key='auth.serverPasswordVerifier'",
            ).fetchone()[0])
            control['oauth_password_phc'] = old_verifier
            control.pop('server_password_fingerprint', None)
            database.execute("DELETE FROM agena_server_state WHERE key='auth.serverPasswordVerifier'")
            database.execute("UPDATE agena_server_state SET value_json=? WHERE key='mcp.server.control'",
                             (json.dumps(control),))
        self.start(password='shared-password-after-migration')
        self.assert_token_accepted(tokens['access_token'], False)
        _, authorization = self.authorization_request()
        status, _, _ = self.request('/oauth/authorize', 'POST', authorization | {
            'password': self.DEFAULT_PASSWORD,
        }, form=True)
        self.assertEqual(status, 401)
        _, migrated = self.authorize(self.password)
        self.assert_token_accepted(migrated['access_token'], True)
        with sqlite3.connect(self.root / 'data/agena.db') as database:
            saved = json.loads(database.execute(
                "SELECT value_json FROM agena_server_state WHERE key='mcp.server.control'",
            ).fetchone()[0])
        self.assertNotIn('oauth_password_phc', saved)
        self.assertTrue(saved['server_password_fingerprint'])

    def test_browser_consent_errors_and_passwordless_cancellation(self):
        self.json_request('/api/v1/server/mcp', 'PUT', {'enabled': True})
        password = self.password
        client = self.json_request('/register', 'POST', {
            'client_name': '<script>alert(1)</script> {{SCRIPT}} & "client"',
            'redirect_uris': ['https://client.example.com/callback'],
            'token_endpoint_auth_method': 'none',
        }, expected=201)
        verifier = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk'
        authorization = {
            'response_type': 'code', 'client_id': client['client_id'],
            'redirect_uri': 'https://client.example.com/callback',
            'state': 'browser-state<&"', 'code_challenge_method': 'S256',
            'code_challenge': base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('='),
            'scope': 'agena:tools',
        }
        browser = {'Accept': 'text/html', 'Accept-Language': 'en-US,en;q=0.9'}
        status, headers, body = self.request('/oauth/authorize?' + urlencode(authorization), headers=browser)
        self.assertEqual(status, 200)
        html = body.decode()
        self.assertIn('lang="en-US"', html)
        self.assertIn('Connect to Agena', html)
        self.assertIn('&lt;script&gt;alert(1)&lt;/script&gt; {{SCRIPT}} &amp; &quot;client&quot;', html)
        self.assertNotIn('<script>alert(1)', html)
        self.assertNotIn('data-i18n="offline_title"', html)
        self.assertEqual(html.count('name="client_id"'), 1)
        inline_script = re.search(r'<script>(.*?)</script>', html, re.S).group(1)
        script_hash = base64.b64encode(hashlib.sha256(inline_script.encode()).digest()).decode()
        policy = headers['Content-Security-Policy']
        self.assertIn("script-src 'sha256-" + script_hash + "'", policy)
        self.assertIn("form-action 'self' https://client.example.com", policy)
        self.assertIn("frame-ancestors 'none'", policy)
        self.assertEqual(headers['X-Content-Type-Options'], 'nosniff')
        self.assertEqual(headers['X-Frame-Options'], 'DENY')
        self.assertEqual(headers['Referrer-Policy'], 'no-referrer')
        self.assertEqual(headers['Cache-Control'], 'no-store')

        status, headers, _ = self.request('/oauth/authorize', 'POST', authorization | {
            'decision': 'deny', 'ui_locale': 'en-US',
        }, form=True, headers=browser)
        self.assertEqual(status, 303)
        callback = parse_qs(urlsplit(headers['Location']).query)
        self.assertEqual(callback['error'], ['access_denied'])
        self.assertEqual(callback['state'], [authorization['state']])
        self.assertEqual(callback['iss'], [self.base])
        self.assertNotIn('code', callback)

        status, _, body = self.request('/oauth/authorize', 'POST', authorization | {
            'decision': 'allow', 'password': 'do-not-echo-this-password', 'ui_locale': 'ar-SA',
        }, form=True, headers=browser)
        self.assertEqual(status, 401)
        html = body.decode()
        self.assertIn('lang="ar-SA" dir="rtl"', html)
        self.assertIn('role="alert"', html)
        self.assertIn('aria-invalid="true"', html)
        self.assertNotIn('do-not-echo-this-password', html)

        # A completed form can still authorize after a denial and a failed
        # password attempt, and the callback keeps the protocol's state/issuer.
        status, headers, _ = self.request('/oauth/authorize', 'POST', authorization | {
            'decision': 'allow', 'password': password,
        }, form=True, headers=browser)
        self.assertEqual(status, 303)
        self.assertIn('code', parse_qs(urlsplit(headers['Location']).query))

        # An untrusted callback stays local; HTML is only used for browsers.
        invalid = authorization | {'redirect_uri': 'https://attacker.example.com/callback'}
        status, headers, body = self.request('/oauth/authorize?' + urlencode(invalid), headers=browser)
        self.assertEqual(status, 400)
        self.assertIn('text/html', headers['Content-Type'])
        self.assertNotIn('Location', headers)
        self.assertIn('Unable to connect', body.decode())
        self.assertIn('redirect_uri is not registered', body.decode())
        status, headers, body = self.request('/oauth/authorize?' + urlencode(invalid))
        self.assertEqual(status, 400)
        self.assertEqual(body['error'], 'invalid_request')
        self.assertIn('application/json', headers['Content-Type'])
        status, headers, body = self.request('/oauth/authorize?client_id=a&client_id=b', headers=browser)
        self.assertEqual(status, 400)
        self.assertIn('text/html', headers['Content-Type'])
        self.assertIn('Unable to connect', body.decode())
        status, headers, body = self.request('/oauth/authorize', 'POST', {}, headers=browser)
        self.assertEqual(status, 400)
        self.assertIn('text/html', headers['Content-Type'])

        # Validated-client errors must still go back through OAuth rather than
        # strand the client on a local error page after the redesign.
        invalid_pkce = authorization | {'code_challenge_method': 'plain'}
        status, headers, _ = self.request('/oauth/authorize?' + urlencode(invalid_pkce), headers=browser)
        self.assertEqual(status, 303)
        callback = parse_qs(urlsplit(headers['Location']).query)
        self.assertEqual(callback['error'], ['invalid_request'])
        self.assertEqual(callback['iss'], [self.base])
        self.assertEqual(callback['state'], [authorization['state']])


if __name__ == '__main__':
    unittest.main()
