"""Isolated MCP/OAuth fixture; never imports user configuration or executes tools."""
import argparse
import base64
import hashlib
import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlencode, urlsplit

parser = argparse.ArgumentParser()
parser.add_argument('--transport', choices=['stdio', 'http'], default='stdio')
parser.add_argument('--protocol', choices=['legacy', 'modern'], default='legacy')
parser.add_argument('--capabilities', choices=['tools', 'resources', 'mixed'], default='tools')
parser.add_argument('--log')
parser.add_argument('--port-file')
parser.add_argument('--notifications', action='store_true')
parser.add_argument('--oauth', action='store_true')
parser.add_argument('--registration', choices=['dcr', 'preregistered', 'metadata'], default='dcr')
parser.add_argument('--issuer-base')
parser.add_argument('--authorization-url')
args = parser.parse_args()
changed = False
challenge = None
base = None

def record(method, metadata=None):
    if args.log:
        with open(args.log, 'a') as stream:
            stream.write(json.dumps({'method': method, 'metadata': metadata}) + '\n')

def result(request, value):
    if args.protocol == 'modern':
        value = dict(value, resultType='complete', ttlMs=0, cacheScope='private')
    return {'jsonrpc': '2.0', 'id': request['id'], 'result': value}

def failure(request, code=-32601, message='Method not supported'):
    return {'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': code, 'message': message}}

def capabilities():
    value = {}
    if args.capabilities in ['tools', 'mixed']:
        value['tools'] = {'listChanged': args.notifications}
    if args.capabilities in ['resources', 'mixed']:
        value.update(resources={}, prompts={})
    return value

def emit(value):
    print(json.dumps(value), flush=True)

def rpc(request, headers=None):
    global changed
    method = request['method']
    params = request.get('params', {})
    metadata = params.get('_meta', {})
    record(method, {'version': metadata.get('io.modelcontextprotocol/protocolVersion'),
                    'capabilities': metadata.get('io.modelcontextprotocol/clientCapabilities'),
                    'header_version': headers.get('Mcp-Protocol-Version') if headers else None})
    if 'id' not in request:
        return None
    if method == 'server/discover':
        if args.protocol == 'legacy':
            return failure(request)
        return result(request, {'supportedVersions': ['2026-07-28'], 'capabilities': capabilities(),
            '_meta': {'io.modelcontextprotocol/serverInfo': {'name': 'fixture', 'version': '1.0'}}})
    if method == 'initialize':
        if args.protocol == 'modern':
            return failure(request)
        return result(request, {'protocolVersion': '2025-11-25', 'capabilities': capabilities(),
            'serverInfo': {'name': 'fixture', 'version': '1.0'}})
    if method == 'tools/list' and 'tools' in capabilities():
        names = ['updated'] if changed else ['ping', 'slow', 'disconnect']
        return result(request, {'tools': [{'name': name, 'inputSchema': {'type': 'object', 'properties': {}}} for name in names]})
    if method == 'tools/call' and 'tools' in capabilities():
        if params['name'] == 'disconnect':
            sys.exit(0)
        if params['name'] == 'slow':
            time.sleep(0.3)
        return result(request, {'content': [{'type': 'text', 'text': str(os.getpid())}]})
    if method == 'resources/list' and 'resources' in capabilities():
        return result(request, {'resources': [{'uri': 'fixture://notes', 'name': 'Notes'}]})
    if method == 'resources/templates/list' and 'resources' in capabilities():
        return result(request, {'resourceTemplates': [{'uriTemplate': 'fixture://notes/{id}', 'name': 'Notes'}]})
    if method == 'resources/read' and 'resources' in capabilities():
        return result(request, {'contents': [{'uri': params['uri'], 'text': 'Fixture notes'}]})
    if method == 'prompts/list' and 'prompts' in capabilities():
        return result(request, {'prompts': [{'name': 'review', 'description': 'Review notes'}]})
    if method == 'prompts/get' and 'prompts' in capabilities():
        return result(request, {'messages': [{'role': 'user', 'content': {'type': 'text', 'text': 'Review fixture notes'}}]})
    if method == 'subscriptions/listen' and args.notifications and args.transport == 'stdio':
        meta = {'io.modelcontextprotocol/subscriptionId': request['id']}
        emit({'jsonrpc': '2.0', 'method': 'notifications/subscriptions/acknowledged',
              'params': {'notifications': params['notifications'], '_meta': meta}})
        changed = True
        emit({'jsonrpc': '2.0', 'method': 'notifications/tools/list_changed', 'params': {'_meta': meta}})
        return None
    if method == 'ping':
        return result(request, {})
    return failure(request)

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def send_json(self, status, value, headers=None):
        data = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        global challenge
        parsed = urlsplit(self.path)
        record('GET ' + parsed.path)
        if parsed.path.startswith('/.well-known/oauth-protected-resource'):
            return self.send_json(200, {'resource': base + '/mcp', 'authorization_servers': [args.issuer_base or base], 'scopes_supported': ['read']})
        if parsed.path.startswith('/.well-known/oauth-authorization-server') or parsed.path.startswith('/.well-known/openid-configuration'):
            metadata = {'issuer': base, 'authorization_endpoint': args.authorization_url or base + '/authorize',
                'token_endpoint': base + '/token',
                'response_types_supported': ['code'], 'grant_types_supported': ['authorization_code', 'refresh_token'],
                'code_challenge_methods_supported': ['S256'], 'token_endpoint_auth_methods_supported': ['none', 'client_secret_basic', 'client_secret_post'],
                'scopes_supported': ['read'], 'authorization_response_iss_parameter_supported': True}
            if args.registration == 'dcr':
                metadata['registration_endpoint'] = base + '/register'
            elif args.registration == 'metadata':
                metadata['client_id_metadata_document_supported'] = True
            return self.send_json(200, metadata)
        if parsed.path == '/authorize':
            query = parse_qs(parsed.query)
            challenge = query['code_challenge'][0]
            location = query['redirect_uri'][0] + '?' + urlencode({'code': 'fixture-code', 'state': query['state'][0], 'iss': base})
            return self.send_json(302, {}, {'Location': location})
        if args.oauth and parsed.path == '/mcp':
            return self.send_json(401, {}, {'WWW-Authenticate': 'Bearer resource_metadata="' + base + '/.well-known/oauth-protected-resource"'})
        self.send_json(405, {})

    def do_POST(self):
        parsed = urlsplit(self.path)
        data = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        if parsed.path == '/register':
            record('POST /register')
            if args.registration != 'dcr':
                return self.send_json(404, {})
            registration = json.loads(data)
            return self.send_json(201, dict(registration, client_id='fixture-client', token_endpoint_auth_method='none'))
        if parsed.path == '/token':
            record('POST /token')
            body = parse_qs(data.decode())
            if args.registration == 'preregistered':
                expected = base64.b64encode(b'pre-registered:registration-secret').decode()
                basic_matches = self.headers.get('Authorization') == 'Basic ' + expected
                form_matches = body.get('client_id') == ['pre-registered'] and body.get('client_secret') == ['registration-secret']
                if not basic_matches and not form_matches:
                    return self.send_json(401, {'error': 'invalid_client'})
            verifier = body.get('code_verifier', [''])[0]
            actual = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('=')
            if body.get('code') != ['fixture-code'] or actual != challenge:
                return self.send_json(400, {'error': 'invalid_grant'})
            return self.send_json(200, {'access_token': 'fixture-access', 'refresh_token': 'fixture-refresh',
                'token_type': 'Bearer', 'expires_in': 3600, 'scope': 'read'})
        if args.oauth and self.headers.get('Authorization') != 'Bearer fixture-access':
            return self.send_json(401, {}, {'WWW-Authenticate': 'Bearer resource_metadata="' + base + '/.well-known/oauth-protected-resource"'})
        reply = rpc(json.loads(data), self.headers)
        if reply is None:
            self.send_response(202)
            self.send_header('Content-Length', '0')
            self.end_headers()
        else:
            self.send_json(200, reply)

if args.transport == 'stdio':
    for line in sys.stdin:
        reply = rpc(json.loads(line))
        if reply is not None:
            emit(reply)
else:
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    base = 'http://127.0.0.1:' + str(server.server_port)
    Path(args.port_file).write_text(base)
    server.serve_forever()
