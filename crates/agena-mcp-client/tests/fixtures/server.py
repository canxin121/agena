"""Isolated MCP/OAuth fixture; never imports user configuration or executes tools."""
import argparse
import base64
import hashlib
import json
import sys
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from queue import Empty, Queue
from urllib.parse import parse_qs, urlencode, urlsplit

parser = argparse.ArgumentParser()
parser.add_argument('--transport', choices=['stdio', 'http'], default='stdio')
parser.add_argument('--protocol', choices=['legacy', 'modern'], default='legacy')
parser.add_argument('--capabilities', choices=['tools', 'resources', 'mixed'], default='tools')
parser.add_argument('--log')
parser.add_argument('--port-file')
parser.add_argument('--notifications', action='store_true')
parser.add_argument('--tasks', action='store_true')
parser.add_argument('--oauth', action='store_true')
parser.add_argument('--registration', choices=['dcr', 'preregistered', 'metadata'], default='dcr')
parser.add_argument('--issuer-base')
parser.add_argument('--authorization-url')
args = parser.parse_args()
# PID namespaces can assign PID 2 to every restarted fixture.
instance_id = uuid.uuid4().hex
changed = False
challenge = None
base = None
input_ready = threading.Condition()
input_replies = {}
task_lock = threading.Lock()
tasks = {}
stdio_task_inputs = {}
standalone_inputs = Queue()
TASKS = 'io.modelcontextprotocol/tasks'
RELATED_TASK = 'io.modelcontextprotocol/related-task'

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
    if args.tasks:
        if args.protocol == 'modern':
            value['extensions'] = {TASKS: {}}
        else:
            value['tasks'] = {'cancel': {}, 'requests': {'tools': {'call': {}}}}
    return value

def emit(value):
    print(json.dumps(value), flush=True)

def input_request(label):
    return {'method': 'elicitation/create', 'params': {
        'mode': 'form', 'message': label, 'requestedSchema': {
            'type': 'object', 'properties': {'answer': {'type': 'string'}}, 'required': ['answer'],
        },
    }}

def picker_request(label, modern):
    params = {'message': label, 'requestedSchema': {
        'type': 'object', 'properties': {'template': {
            'type': 'openai/imagePicker', 'items': [
                {'id': 'month', 'title': 'Monthly review', 'image': 'https://example.test/thumb'},
                {'id': 'week', 'title': 'Weekly review', 'image': 'https://example.test/other'},
            ],
        }}, 'required': ['template'],
    }}
    if modern:
        params['mode'] = 'form'
    return {'method': 'openai/elicitation/create' if modern else 'openai/form', 'params': params}

def input_result(request, response):
    text = json.dumps(response)
    if request['method'] == 'prompts/get':
        value = {'messages': [{'role': 'user', 'content': {'type': 'text', 'text': text}}]}
    elif request['method'] == 'resources/read':
        value = {'contents': [{'uri': request['params']['uri'], 'text': text}]}
    else:
        value = {'content': [{'type': 'text', 'text': text}]}
    return result(request, value)

def modern_input(request, label):
    params = request['params']
    state = params.get('requestState')
    if state == f'answer/{label}':
        replies = params['inputResponses']
        if sorted(replies) != ['form', 'roots'] or 'roots' not in replies['roots']:
            return failure(request, -32602, 'Missing input responses')
        return input_result(request, replies['form'])
    if state == f'wait/{label}':
        required = {'requestState': f'answer/{label}', 'inputRequests': {
            'form': (picker_request(label, True) if params.get('name') == 'ask-picker' else input_request(label)),
            'roots': {'method': 'roots/list'},
        }}
    elif state is None:
        required = {'requestState': f'wait/{label}'}
    else:
        return failure(request, -32602, 'Unexpected request state')
    return {'jsonrpc': '2.0', 'id': request['id'], 'result': dict(required, resultType='input_required')}

def task_reply(request, value, result_type='complete'):
    if args.protocol == 'modern':
        value = dict(value, resultType=result_type)
    return {'jsonrpc': '2.0', 'id': request['id'], 'result': value}

def task_state(task, status):
    value = {'taskId': task['id'], 'status': status, 'createdAt': '2025-11-25T10:30:00Z',
             'lastUpdatedAt': '2025-11-25T10:30:00Z'}
    if args.protocol == 'modern':
        return dict(value, ttlMs=None, pollIntervalMs=20)
    return dict(value, ttl=None, pollInterval=20)

def task_call(request):
    params = request['params']
    if args.protocol == 'modern':
        extensions = params['_meta']['io.modelcontextprotocol/clientCapabilities']['extensions']
        if TASKS not in extensions or 'task' in params:
            return failure(request, -32602, 'Missing modern Tasks capability')
    elif params.get('task') != {}:
        return failure(request, -32601, 'This tool requires task augmentation')
    arguments = params.get('arguments', {})
    task_id = uuid.uuid4().hex
    task = dict(id=task_id, kind=arguments.get('kind', 'completed'),
                label=arguments.get('label', 'task'), polls=0, responses=None)
    with task_lock:
        tasks[task_id] = task
    state = task_state(task, 'working')
    if args.protocol == 'legacy':
        return task_reply(request, {'task': state})
    return task_reply(request, state, 'task')

def task_content(task):
    text = json.dumps(task['responses']['form']) if task['responses'] else task['label']
    return {'content': [{'type': 'text', 'text': text}], 'isError': task['kind'] == 'tool-error'}

def task_operation(request):
    method = request['method']
    params = request['params']
    with task_lock:
        task = tasks.get(params['taskId'])
        if task is None:
            return failure(request, -32602, 'Unknown task')
        if method == 'tasks/cancel':
            task['kind'] = 'cancelled'
            state = task_state(task, 'cancelled') if args.protocol == 'legacy' else {}
            return task_reply(request, state)
        if method == 'tasks/update' and args.protocol == 'modern':
            answers = params['inputResponses']
            if sorted(answers) != ['form', 'roots'] or 'roots' not in answers['roots']:
                return failure(request, -32602, 'Missing task input responses')
            task['responses'] = answers
            task['updated_at'] = task['polls']
            return task_reply(request, {})
        if method == 'tasks/result' and args.protocol == 'legacy':
            return legacy_task_result(request, task)
        if method != 'tasks/get':
            return failure(request)
        task['polls'] += 1
        kind = task['kind']
        if kind == 'working':
            return task_reply(request, task_state(task, 'working'))
        if task['polls'] == 1:
            return task_reply(request, task_state(task, 'working'))
        if kind in ('input', 'picker', 'standalone', 'cross', 'untagged') and (
                task['responses'] is None or task['polls'] == task['updated_at'] + 1):
            state = task_state(task, 'input_required')
            if args.protocol == 'modern':
                form = (picker_request(task['label'], True) if kind == 'picker'
                        else input_request(task['label']))
                state['inputRequests'] = {'form': form, 'roots': {'method': 'roots/list'}}
            return task_reply(request, state)
        if kind == 'rpc-error':
            state = task_state(task, 'failed')
            if args.protocol == 'modern':
                state['error'] = {'code': -32042, 'message': 'fixture task failed'}
        elif kind == 'cancelled':
            state = task_state(task, 'cancelled')
        else:
            state = task_state(task, 'completed')
            if kind == 'wrong-id':
                state['taskId'] = 'wrong-task'
            if args.protocol == 'modern':
                state['result'] = task_content(task)
        return task_reply(request, state)

def legacy_task_result(request, task, answer=None):
    if task['kind'] in ('rpc-error', 'cancelled'):
        return failure(request, -32042, 'fixture task failed')
    if answer is not None:
        if 'code' in answer:
            return result(request, {'content': [{'type': 'text', 'text': json.dumps(answer)}]})
        if answer.pop('_meta', {}).get(RELATED_TASK, {}).get('taskId') != task['id']:
            return failure(request, -32602, 'Missing related-task metadata')
        value = {'content': [{'type': 'text', 'text': json.dumps(answer)}]}
    else:
        value = task_content(task)
    return result(request, dict(value, _meta={RELATED_TASK: {'taskId': task['id']}}))

def legacy_task_input(request):
    if not (args.tasks and args.protocol == 'legacy' and request.get('method') == 'tasks/result'):
        return None
    with task_lock:
        task = tasks.get(request['params']['taskId'])
        if task is None or task['kind'] not in ('input', 'standalone', 'cross', 'untagged'):
            return None
        related = task['id']
        if task['kind'] == 'cross':
            related = next(item['id'] for item in tasks.values() if item['kind'] == 'standalone')
        elicitation = input_request(task['label'])
        if task['kind'] != 'untagged':
            elicitation['params']['_meta'] = {RELATED_TASK: {'taskId': related}}
        return task, dict(elicitation, jsonrpc='2.0', id=f'input/task/{request["id"]}')

def rpc(request, headers=None):
    global changed
    method = request['method']
    params = request.get('params', {})
    metadata = params.get('_meta', {})
    details = {'version': metadata.get('io.modelcontextprotocol/protocolVersion'),
               'capabilities': metadata.get('io.modelcontextprotocol/clientCapabilities'),
               'header_version': headers.get('Mcp-Protocol-Version') if headers else None}
    if args.tasks:
        details.update(params=params, time=time.monotonic())
    record(method, details)
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
        if args.tasks and params['capabilities'].get('tasks') != {}:
            return failure(request, -32602, 'Missing legacy Tasks capability')
        return result(request, {'protocolVersion': '2025-11-25', 'capabilities': capabilities(),
            'serverInfo': {'name': 'fixture', 'version': '1.0'}})
    if method == 'tools/list' and 'tools' in capabilities():
        names = ['updated'] if changed else ['ping', 'slow', 'disconnect']
        tools = [{'name': name, 'inputSchema': {'type': 'object', 'properties': {}}} for name in names]
        if args.tasks:
            for name, support in [('task', 'required'), ('task-optional', 'optional'),
                                  ('task-forbidden', 'forbidden')]:
                tool = {'name': name, 'inputSchema': {'type': 'object'}}
                if args.protocol == 'legacy':
                    tool['execution'] = {'taskSupport': support}
                tools.append(tool)
        return result(request, {'tools': tools})
    if method == 'tools/call' and 'tools' in capabilities():
        if args.tasks and params['name'] in ('task', 'task-optional'):
            return task_call(request)
        if args.tasks and 'task' in params:
            return failure(request, -32602, 'Unexpected task augmentation')
        if args.protocol == 'modern' and params['name'] in ('ask', 'ask-picker'):
            return modern_input(request, params.get('arguments', {}).get('label', 'tool'))
        if params['name'] == 'disconnect':
            sys.exit(0)
        if params['name'] == 'slow':
            time.sleep(0.3)
        return result(request, {'content': [{'type': 'text', 'text': instance_id}]})
    if args.tasks and method.startswith('tasks/'):
        return task_operation(request)
    if method == 'resources/list' and 'resources' in capabilities():
        return result(request, {'resources': [{'uri': 'fixture://notes', 'name': 'Notes'}]})
    if method == 'resources/templates/list' and 'resources' in capabilities():
        return result(request, {'resourceTemplates': [{'uriTemplate': 'fixture://notes/{id}', 'name': 'Notes'}]})
    if method == 'resources/read' and 'resources' in capabilities():
        if args.protocol == 'modern' and params['uri'] == 'fixture://ask':
            return modern_input(request, 'resource')
        return result(request, {'contents': [{'uri': params['uri'], 'text': 'Fixture notes'}]})
    if method == 'prompts/list' and 'prompts' in capabilities():
        return result(request, {'prompts': [{'name': 'review', 'description': 'Review notes'}]})
    if method == 'prompts/get' and 'prompts' in capabilities():
        if args.protocol == 'modern' and params['name'] == 'ask':
            return modern_input(request, 'prompt')
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

    def elicitation_stream(self, request):
        params = request['params']
        label = params.get('arguments', {}).get('label', 'tool')
        elicitation = input_request(label)
        if params['name'] == 'ask-picker':
            elicitation = picker_request(label, False)
        if params['name'] == 'ask-url':
            elicitation['params'] = {'mode': 'url', 'message': label,
                'url': 'https://example.test/continue', 'elicitationId': 'example'}
        input_id = f'input/{request["id"]}'
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.end_headers()

        def send(value):
            self.wfile.write(b'data: ' + json.dumps(dict(value, jsonrpc='2.0')).encode() + b'\n\n')
            self.wfile.flush()

        send(dict(elicitation, id=input_id))
        if params['name'] == 'ask-ended':
            time.sleep(0.4)
            send(input_result(request, {'action':'ended'}))
        else:
            with input_ready:
                input_ready.wait_for(lambda: input_id in input_replies, timeout=6)
                reply = input_replies.pop(input_id, {})
            send(input_result(request, reply))

    def task_stream(self, request, task, elicitation):
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.end_headers()
        if task['kind'] == 'standalone':
            standalone_inputs.put(elicitation)
        else:
            self.send_event(elicitation)
        with input_ready:
            input_id = elicitation['id']
            input_ready.wait_for(lambda: input_id in input_replies, timeout=6)
            reply = input_replies.pop(input_id, {})
        self.send_event(legacy_task_result(request, task, reply))

    def send_event(self, value):
        self.wfile.write(b'data: ' + json.dumps(value).encode() + b'\n\n')
        self.wfile.flush()

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
        if args.tasks and args.protocol == 'legacy' and parsed.path == '/mcp':
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            try:
                while True:
                    self.send_event(standalone_inputs.get(timeout=6))
            except (Empty, BrokenPipeError, ConnectionResetError):
                return
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
        request = json.loads(data)
        if 'method' not in request and str(request.get('id', '')).startswith('input/'):
            with input_ready:
                input_replies[request['id']] = request.get('result', request.get('error'))
                input_ready.notify_all()
            reply = None
        elif (args.protocol == 'legacy' and request['method'] == 'tools/call'
              and request['params']['name'] in ('ask', 'ask-url', 'ask-ended', 'ask-picker')):
            return self.elicitation_stream(request)
        else:
            task_input = legacy_task_input(request)
            if task_input:
                record('tasks/result')
                return self.task_stream(request, *task_input)
            reply = rpc(request, self.headers)
        if reply is None:
            self.send_response(202)
            self.send_header('Content-Length', '0')
            self.end_headers()
        else:
            headers = ({'Mcp-Session-Id': instance_id}
                       if args.tasks and request['method'] == 'initialize' and 'result' in reply else None)
            self.send_json(200, reply, headers)

if args.transport == 'stdio':
    for line in sys.stdin:
        request = json.loads(line)
        if 'method' not in request:
            pending = stdio_task_inputs.pop(request['id'], None)
            if pending:
                emit(legacy_task_result(*pending, request.get('result', request.get('error'))))
            continue
        task_input = legacy_task_input(request)
        if task_input:
            task, elicitation = task_input
            record('tasks/result')
            stdio_task_inputs[elicitation['id']] = (request, task)
            emit(elicitation)
            continue
        reply = rpc(request)
        if reply is not None:
            emit(reply)
else:
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    base = 'http://127.0.0.1:' + str(server.server_port)
    Path(args.port_file).write_text(base)
    server.serve_forever()
