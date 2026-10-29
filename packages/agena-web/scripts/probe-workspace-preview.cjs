#!/usr/bin/env node
// Isolated preview gateway + real Chromium regression probe. Node >= 22 and a
// freshly built Agena binary are required; no Playwright/npm dependency is used.
//
//   AGENA_BIN=/absolute/path/to/target/debug/agena \
//     node packages/agena-web/scripts/probe-workspace-preview.cjs
//
// Optional: CHROME_BIN, PROBE_REPORT (JSON), PROBE_SCREENSHOT (PNG).
// AGENA_PROBE_AGENT=0 skips the default operator -> browser tool integration.
// Tool/network allow settings below apply only to this temporary test server.
// This starts its own password-protected server, workspace, data directory,
// Chrome profile, and owned preview fixture. It never contacts the live service.
// The tiny workbench shell tests browser/gateway boundaries, not Vue component
// rendering. HMR checks the Vite WebSocket subprotocol and update transport.

const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const fs = require('node:fs/promises')
const http = require('node:http')
const net = require('node:net')
const os = require('node:os')
const path = require('node:path')
const { spawn } = require('node:child_process')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
let interrupted
const checkInterrupted = () => {
  if (interrupted) throw interrupted
}

async function until(read, predicate, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let value
  while (Date.now() < deadline) {
    checkInterrupted()
    value = await read()
    if (predicate(value)) return value
    await sleep(100)
  }
  throw new Error(`Timed out: ${label}; last value: ${JSON.stringify(value)}`)
}

async function freePort() {
  const server = net.createServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const port = server.address().port
  await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
  return port
}

// Physical connections are restricted to the probe's loopback listener. Host
// remains the URL authority so capability-host routing is exercised exactly.
async function request(rawUrl, { method = 'GET', headers = {}, body, timeout = 5_000 } = {}) {
  checkInterrupted()
  const url = new URL(rawUrl)
  assert.equal(url.protocol, 'http:', 'probe only makes loopback HTTP requests')
  const payload = body === undefined ? undefined : typeof body === 'string' ? body : JSON.stringify(body)
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        host: '127.0.0.1',
        port: Number(url.port),
        path: url.pathname + url.search,
        method,
        headers: {
          host: url.host,
          ...(payload === undefined
            ? {}
            : { 'content-type': 'application/json', 'content-length': Buffer.byteLength(payload) }),
          ...headers,
        },
      },
      (response) => {
        const chunks = []
        response.on('data', (chunk) => chunks.push(chunk))
        response.once('error', reject)
        response.once('end', () => {
          const text = Buffer.concat(chunks).toString('utf8')
          let json
          try {
            json = JSON.parse(text)
          } catch {
            /* HTML and plain responses are intentional. */
          }
          resolve({ status: response.statusCode, headers: response.headers, text, json })
        })
      },
    )
    req.once('error', reject)
    req.setTimeout(timeout, () => req.destroy(new Error('HTTP probe request timed out')))
    req.end(payload)
  })
}

async function openEventStream(rawUrl, headers = {}) {
  const url = new URL(rawUrl)
  return new Promise((resolve, reject) => {
    const state = { chunks: 0, closed: false }
    const req = http.get(
      {
        host: '127.0.0.1',
        port: Number(url.port),
        path: url.pathname + url.search,
        headers: { host: url.host, ...headers },
      },
      (response) => {
        response.on('data', () => {
          state.chunks += 1
        })
        response.once('end', () => {
          state.closed = true
        })
        response.once('close', () => {
          state.closed = true
        })
        response.once('error', () => {
          state.closed = true
        })
        state.status = response.statusCode
        state.close = () => {
          response.destroy()
          req.destroy()
        }
        resolve(state)
      },
    )
    req.once('error', (error) => {
      state.closed = true
      reject(error)
    })
    req.setTimeout(8_000, () => req.destroy(new Error('SSE probe stalled')))
  })
}

class Cdp {
  constructor(socket) {
    this.socket = socket
    this.nextId = 1
    this.pending = new Map()
    socket.addEventListener('message', (event) => {
      const message = JSON.parse(String(event.data))
      if (!message.id) return
      const pending = this.pending.get(message.id)
      if (!pending) return
      this.pending.delete(message.id)
      clearTimeout(pending.timer)
      if (message.error) pending.reject(new Error(`CDP ${pending.method}: ${message.error.message}`))
      else pending.resolve(message.result)
    })
    socket.addEventListener('close', () => {
      for (const item of this.pending.values()) {
        clearTimeout(item.timer)
        item.reject(new Error('CDP connection closed'))
      }
      this.pending.clear()
    })
  }
  static async connect(url) {
    const socket = new WebSocket(url)
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        socket.close()
        reject(new Error('CDP connection timed out'))
      }, 10_000)
      socket.addEventListener(
        'open',
        () => {
          clearTimeout(timer)
          resolve()
        },
        { once: true },
      )
      socket.addEventListener(
        'error',
        () => {
          clearTimeout(timer)
          reject(new Error('CDP connection failed'))
        },
        { once: true },
      )
    })
    return new Cdp(socket)
  }
  send(method, params = {}, sessionId) {
    checkInterrupted()
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`CDP ${method} timed out`))
      }, 15_000)
      this.pending.set(id, { resolve, reject, timer, method })
      this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }))
    })
  }
  async evaluate(sessionId, expression) {
    const result = await this.send(
      'Runtime.evaluate',
      { expression, awaitPromise: true, returnByValue: true },
      sessionId,
    )
    if (result.exceptionDetails) throw new Error(`Page evaluation failed: ${JSON.stringify(result.exceptionDetails)}`)
    return result.result?.value
  }
  close() {
    this.socket.close()
  }
}

function websocketFrame(opcode, payload) {
  const data = Buffer.from(payload)
  assert.ok(data.length <= 65_535)
  if (data.length < 126) return Buffer.concat([Buffer.from([0x80 | opcode, data.length]), data])
  const header = Buffer.alloc(4)
  header[0] = 0x80 | opcode
  header[1] = 126
  header.writeUInt16BE(data.length, 2)
  return Buffer.concat([header, data])
}

// RFC 6455 fixture: short text messages, close, and ping/pong are sufficient to
// test the actual gateway bridge without installing a WebSocket server package.
function fixtureWebsocket(req, socket, head, config) {
  if (req.url !== '/mount/hmr' || !req.headers['sec-websocket-key']) {
    socket.end('HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n')
    return
  }
  const accept = crypto
    .createHash('sha1')
    .update(req.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11')
    .digest('base64')
  const protocols = String(req.headers['sec-websocket-protocol'] || '')
    .split(',')
    .map((value) => value.trim())
  const protocol = protocols.includes('vite-hmr') ? 'vite-hmr' : null
  socket.write(
    'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' +
      accept +
      '\r\n' +
      (protocol ? 'Sec-WebSocket-Protocol: ' + protocol + '\r\n' : '') +
      '\r\n',
  )
  socket.write(websocketFrame(1, JSON.stringify({ type: 'connected' })))
  let buffered = Buffer.alloc(0)
  function consume(chunk) {
    buffered = Buffer.concat([buffered, chunk])
    while (buffered.length >= 2) {
      const opcode = buffered[0] & 0x0f
      const masked = (buffered[1] & 0x80) !== 0
      let length = buffered[1] & 0x7f
      let offset = 2
      if (length === 126) {
        if (buffered.length < 4) return
        length = buffered.readUInt16BE(2)
        offset = 4
      } else if (length === 127) {
        socket.destroy(new Error('fixture rejects oversized frames'))
        return
      }
      if (!masked) {
        socket.destroy(new Error('client frame must be masked'))
        return
      }
      if (buffered.length < offset + 4 + length) return
      const mask = buffered.subarray(offset, offset + 4)
      offset += 4
      const payload = Buffer.from(buffered.subarray(offset, offset + length))
      buffered = buffered.subarray(offset + length)
      for (let index = 0; index < payload.length; index++) payload[index] ^= mask[index % 4]
      if (opcode === 8) {
        socket.end(websocketFrame(8, payload))
        return
      }
      if (opcode === 9) {
        socket.write(websocketFrame(10, payload))
        continue
      }
      if (opcode !== 1) continue
      const text = payload.toString('utf8')
      if (text === 'probe-update')
        socket.write(
          websocketFrame(
            1,
            JSON.stringify({
              type: 'update',
              updates: [
                { type: 'js-update', path: '/assets/app.js', acceptedPath: '/assets/app.js', timestamp: Date.now() },
              ],
            }),
          ),
        )
      else
        socket.write(
          websocketFrame(
            1,
            JSON.stringify({
              type: 'echo',
              data: text,
              nonce: config.nonce,
              origin: req.headers.origin,
              cookie: req.headers.cookie || '',
            }),
          ),
        )
    }
  }
  socket.on('data', consume)
  socket.on('error', () => {})
  if (head.length) consume(head)
}

function fixtureScript(config) {
  return `(() => {
    const nonce = ${JSON.stringify(config.nonce)};
    const controlOrigin = ${JSON.stringify(config.controlOrigin)};
    const report = { asset: true, path: location.pathname, origin: location.origin, errors: [] };
    document.getElementById('fixture').textContent = 'Preview fixture: root asset loaded';
    document.body.dataset.assetLoaded = 'true';
    document.getElementById('probe-click').addEventListener('click', () => {
      document.getElementById('click-result').textContent = 'Managed browser click verified';
    });
    document.addEventListener('securitypolicyviolation', (event) => {
      if (event.effectiveDirective === 'connect-src' && event.blockedURI.startsWith(controlOrigin)) {
        document.getElementById('csp-result').textContent = 'CSP blocked external connection';
      }
    });
    document.getElementById('probe-connect').addEventListener('click', () => {
      fetch(controlOrigin + '/api/v1/workbench/preview/sessions').catch(() => {});
    });
    // The real browser tool opens this document at top level. Keep its initial
    // load deterministic: background HTTP has no authority once a tool ends.
    // Explicit clicks below test actions during their own active tool calls.
    if (parent === window) return;
    const publish = (kind, detail) => parent.postMessage({ kind, nonce, ...detail }, controlOrigin);
    const run = async () => {
      localStorage.setItem('agena-probe-app', nonce);
      report.appStorage = localStorage.getItem('agena-probe-app') === nonce;
      try { report.parentStorage = parent.localStorage.getItem('agena-probe-workbench'); }
      catch (error) { report.parentStorageBlocked = error.name === 'SecurityError'; }
      try { report.parentDom = parent.document.title; }
      catch (error) { report.parentDomBlocked = error.name === 'SecurityError'; }
      const login = await fetch('/login', { method: 'POST', credentials: 'include' });
      report.loginStatus = login.status;
      const authenticated = await fetch('/echo', { credentials: 'include', headers: { Authorization: 'Bearer probe-app-' + nonce } });
      report.appIdentity = await authenticated.json();
      const redirect = await fetch('/account/login', { credentials: 'include' });
      report.redirect = { url: redirect.url, body: await redirect.json() };
      history.pushState({}, '', '/spa/deep?probe=' + encodeURIComponent(nonce));
      const fallback = await fetch(location.pathname + location.search);
      report.spaFallback = fallback.status === 200 && (await fallback.text()).includes('Preview integration fixture');
      try {
        const response = await fetch(controlOrigin + '/api/v1/workbench/preview/sessions');
        report.controlRead = { status: response.status, body: await response.text() };
      } catch (error) { report.controlReadBlocked = error.name === 'TypeError'; }
      report.visible = document.getElementById('fixture').getBoundingClientRect().height > 0;
      const socket = new WebSocket(location.origin.replace(/^http/, 'ws') + '/hmr', 'vite-hmr');
      window.probeSocket = socket;
      const opened = new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('fixture WebSocket handshake timed out')), 10000);
        socket.addEventListener('open', () => { clearTimeout(timer); report.websocketProtocol = socket.protocol; socket.send('probe-echo'); socket.send('probe-update'); resolve(); }, { once: true });
        socket.addEventListener('error', () => { clearTimeout(timer); reject(new Error('fixture WebSocket failed')); }, { once: true });
      });
      socket.addEventListener('message', (event) => {
        const message = JSON.parse(event.data);
        if (message.type === 'echo') report.websocketEcho = message;
        if (message.type === 'update') report.hmrUpdate = message;
        publish('preview-progress', { report });
      });
      socket.addEventListener('close', () => publish('preview-ws-closed', { code: socket.readyState }));
      await opened;
      publish('preview-progress', { report });
    };
    run().catch((error) => { report.errors.push(String(error.stack || error)); publish('preview-progress', { report }); });
  })();`
}

async function fixtureMain(configPath) {
  const config = JSON.parse(await fs.readFile(configPath, 'utf8'))
  const sockets = new Set()
  const server = http.createServer((req, res) => {
    const observed = {
      path: req.url,
      authorization: req.headers.authorization || '',
      cookie: req.headers.cookie || '',
      origin: req.headers.origin || '',
      hop: req.headers['x-probe-hop'] || '',
    }
    // Append only fixture request metadata; this never records workbench login.
    fs.appendFile(config.requestsPath, JSON.stringify(observed) + '\n').catch(() => {})
    const url = new URL(req.url, 'http://fixture.invalid')
    if (!url.pathname.startsWith('/mount/')) {
      res.writeHead(404)
      res.end('upstream base prefix missing')
      return
    }
    const route = url.pathname.slice('/mount'.length)
    res.setHeader(
      'Content-Security-Policy',
      "default-src 'self'; script-src 'self'; connect-src 'self' " + config.controlOrigin + "; frame-ancestors 'none'",
    )
    res.setHeader('X-Frame-Options', 'DENY')
    res.setHeader('Cache-Control', 'no-store')
    if (route === '/__ready') {
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ pid: process.pid, nonce: config.nonce }))
      return
    }
    if (route === '/assets/app.js') {
      res.writeHead(200, { 'content-type': 'text/javascript' })
      res.end(fixtureScript(config))
      return
    }
    if (route === '/events') {
      res.writeHead(200, { 'content-type': 'text/event-stream' })
      res.write('data: ready\n\n')
      const timer = setInterval(() => res.write('data: tick\n\n'), 100)
      res.once('close', () => clearInterval(timer))
      return
    }
    if (route === '/wait-headers') {
      const timer = setTimeout(() => {
        res.writeHead(200)
        res.end('late headers')
      }, 30_000)
      res.once('close', () => clearTimeout(timer))
      return
    }
    if (route === '/needs-login') {
      res.writeHead(401, { 'content-type': 'text/html' })
      res.end('<h1>Application login required</h1>')
      return
    }
    if (route === '/encoded-body') {
      res.writeHead(200, { 'content-type': 'text/html', 'content-encoding': 'x-probe-encoding' })
      res.end('<img src="/opaque-asset">')
      return
    }
    if (route === '/range') {
      const body = '<img src="/range">'
      res.writeHead(206, {
        'content-type': 'text/html',
        'content-range': `bytes 0-${Buffer.byteLength(body) - 1}/100`,
        'content-length': Buffer.byteLength(body),
      })
      res.end(body)
      return
    }
    if (route === '/login') {
      // A partitioned cookie tests login inside a cross-site iframe under modern
      // Chrome defaults. Loopback *.localhost is a potentially trustworthy site.
      res.setHeader('Set-Cookie', [
        'agena_probe=' + config.nonce + '; Path=/mount; Domain=127.0.0.1; HttpOnly; Secure; SameSite=None; Partitioned',
        'domain=' + config.nonce + '; Path=/mount; Domain=127.0.0.1; HttpOnly; Secure; SameSite=None; Partitioned',
      ])
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end('{"ok":true}')
      return
    }
    if (route === '/echo' || route === '/account/verify' || route.startsWith('/encoded/')) {
      res.setHeader('Connection', 'keep-alive, x-upstream-hop')
      res.setHeader('X-Upstream-Hop', 'must-not-reach-browser')
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ ...observed, path: route, query: url.search }))
      return
    }
    if (route === '/account/login') {
      res.writeHead(302, { location: 'verify?step=2' })
      res.end()
      return
    }
    res.writeHead(200, { 'content-type': 'text/html' })
    res.end(
      '<!doctype html><html><head><title>Preview integration fixture</title></head><body><h1 id="fixture">Loading preview fixture</h1><p>Root assets, SPA navigation, login cookies, and WebSocket transport.</p><button id="probe-click">Verify browser click</button><p id="click-result">Click pending</p><button id="probe-connect">Verify connection policy</button><p id="csp-result">Connection policy pending</p><script src="/assets/app.js"></script></body></html>',
    )
  })
  server.on('connection', (socket) => {
    sockets.add(socket)
    socket.on('close', () => sockets.delete(socket))
  })
  server.on('upgrade', (req, socket, head) => fixtureWebsocket(req, socket, head, config))
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(config.port, '127.0.0.1', resolve)
  })
  await fs.writeFile(config.readyPath, JSON.stringify({ pid: process.pid, nonce: config.nonce, configPath }))
  const shutdown = () => {
    for (const socket of sockets) socket.destroy()
    server.close(() => process.exit(0))
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}

function workbenchHtml() {
  return `<!doctype html><html><head><title>Agena preview isolation probe</title><style>body{font:16px system-ui;margin:20px}iframe{width:95vw;height:650px;border:2px solid #23865a}</style></head><body><h1>Isolated Agena preview probe</h1><iframe id="preview" sandbox="allow-scripts allow-forms allow-same-origin allow-downloads" referrerpolicy="no-referrer"></iframe><script>
    window.probeState = { reports: [], closed: false };
    window.installPreview = (url, nonce) => {
      window.probeState = { reports: [], closed: false };
      window.probeNonce = nonce;
      localStorage.setItem('agena-probe-workbench', 'workbench-secret-' + nonce);
      document.getElementById('preview').src = url;
    };
    addEventListener('message', (event) => {
      const frame = document.getElementById('preview');
      if (event.source !== frame.contentWindow || event.origin !== new URL(frame.src).origin || event.data.nonce !== window.probeNonce) return;
      if (event.data.kind === 'preview-progress') window.probeState.reports.push(event.data.report);
      if (event.data.kind === 'preview-ws-closed') window.probeState.closed = true;
    });
  </script></body></html>`
}

async function terminate(child, signal = 'SIGTERM') {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill(signal)
  const deadline = Date.now() + 8_000
  while (child.exitCode === null && child.signalCode === null && Date.now() < deadline) await sleep(50)
  if (child.exitCode === null && child.signalCode === null) {
    child.kill('SIGKILL')
    await until(
      () => Promise.resolve(child.exitCode !== null || child.signalCode !== null),
      Boolean,
      'owned child termination',
      3_000,
    )
  }
}

async function main() {
  assert.ok(typeof WebSocket === 'function', 'Node >= 22 with built-in WebSocket is required')
  const repo = path.resolve(__dirname, '../../..')
  const binary = path.resolve(process.env.AGENA_BIN || path.join(repo, 'target/debug/agena'))
  const chromeBinary = process.env.CHROME_BIN || '/usr/bin/google-chrome'
  await fs.access(binary)
  await fs.access(chromeBinary)
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-preview-probe-'))
  try {
    await runProbe(temporary, binary, chromeBinary)
  } finally {
    await fs.rm(temporary, { recursive: true, force: true })
  }
}

async function runProbe(temporary, binary, chromeBinary) {
  const reportPath = path.resolve(
    process.env.PROBE_REPORT || path.join(os.tmpdir(), path.basename(temporary) + '-report.json'),
  )
  const nonce = crypto.randomBytes(12).toString('hex')
  const password = 'preview-probe-' + crypto.randomBytes(24).toString('hex')
  const workspace = path.join(temporary, 'project')
  const ui = path.join(temporary, 'ui')
  const profile = path.join(temporary, 'chrome')
  const data = path.join(temporary, 'data')
  const probeAgent = process.env.AGENA_PROBE_AGENT !== '0'
  await Promise.all([workspace, ui, profile, data].map((directory) => fs.mkdir(directory, { recursive: true })))
  if (probeAgent) {
    // Runtime permission/plugin settings belong to the isolated data config;
    // the CLI --set override contract intentionally supports a smaller schema.
    await fs.writeFile(
      path.join(data, 'agena.json'),
      JSON.stringify(
        {
          permission: { network: { loopback: 'allow' }, tools: { default: 'allow' } },
          plugins: {
            list: {
              'agena.web': {
                enabled: true,
                package: { kind: 'static' },
                settings: { browser: { enabled: true, executable_path: chromeBinary } },
              },
            },
          },
        },
        null,
        2,
      ) + '\n',
    )
  }
  await fs.writeFile(path.join(ui, 'index.html'), workbenchHtml())
  const port = await freePort()
  const upstreamPort = await freePort()
  const origin = 'http://localhost:' + port
  // Exercise the localhost resolution/reservation path as well as the
  // fixture's explicit IPv4 listener throughout the real proxy/browser flow.
  const upstream = 'http://localhost:' + upstreamPort + '/mount/'
  const readyPath = path.join(temporary, 'fixture-ready.json')
  const requestsPath = path.join(temporary, 'fixture-requests.jsonl')
  const configPath = path.join(workspace, 'preview-fixture.json')
  await fs.writeFile(
    configPath,
    JSON.stringify({ nonce, controlOrigin: origin, port: upstreamPort, readyPath, requestsPath }),
  )
  const serverLogPath = path.join(temporary, 'server.log')
  const chromeLogPath = path.join(temporary, 'chrome.log')
  const serverLog = await fs.open(serverLogPath, 'w')
  const chromeLog = await fs.open(chromeLogPath, 'w')
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('AGENA_')))
  Object.assign(env, { AGENA_SERVER_PASSWORD: password, AGENA_DATA_DIR: data, AGENA_MCP_ENABLED: 'false' })
  let server,
    chrome,
    cdp,
    token,
    pageSession,
    agentSession,
    invokeAgentTool,
    previewCreated = false
  const checks = []
  const streams = []
  let failure
  const check = (name, details = {}) => {
    checks.push({ name, status: 'passed', ...details })
    console.log('PASS ' + name)
  }
  const api = async (method, route, body, expected = 200) => {
    const response = await request(origin + route, {
      method,
      headers: token ? { authorization: 'Bearer ' + token } : {},
      body,
    })
    assert.equal(response.status, expected, `${method} ${route}: ${response.text}`)
    return response.json
  }
  const access = (id = 'probe') =>
    api('POST', '/api/v1/workbench/preview/sessions/' + id + '/access', {
      backendOrigin: origin,
      workbenchOrigin: origin,
    })
  const waitFixture = async (pid) =>
    until(
      async () => {
        try {
          return (await request(upstream + '__ready')).json
        } catch {
          return null
        }
      },
      (value) => value?.pid === pid && value.nonce === nonce,
      'owned preview fixture readiness',
    )
  const install = async (grant) => {
    await cdp.evaluate(pageSession, `window.installPreview(${JSON.stringify(grant.url)}, ${JSON.stringify(nonce)})`)
    return until(
      async () => {
        const state = await cdp.evaluate(pageSession, 'window.probeState')
        const report = state.reports.at(-1)
        if (report?.errors.length) throw new Error('Preview page failed: ' + report.errors.join('\n'))
        return report
      },
      (report) => report?.websocketEcho && report.hmrUpdate,
      'iframe assets, login, navigation, and HMR',
    )
  }
  const signalHandler = () => {
    interrupted = new Error('probe interrupted')
  }
  process.once('SIGINT', signalHandler)
  process.once('SIGTERM', signalHandler)
  try {
    server = spawn(
      binary,
      [
        '--data-dir',
        data,
        'server',
        '--host',
        '127.0.0.1',
        '--port',
        String(port),
        '--workspace',
        workspace,
        '--ui-dir',
        ui,
        '--mcp-enabled',
        'false',
      ],
      { cwd: workspace, env, stdio: ['ignore', serverLog.fd, serverLog.fd] },
    )
    let serverSpawnError
    server.once('error', (error) => {
      serverSpawnError = error
    })
    await until(
      async () => {
        if (serverSpawnError) throw serverSpawnError
        if (server.exitCode !== null || server.signalCode !== null)
          throw new Error('Isolated Agena server exited: ' + (await fs.readFile(serverLogPath, 'utf8')))
        return request(origin + '/auth/session')
          .then((response) => response.status === 200 || response.status === 401)
          .catch(() => false)
      },
      Boolean,
      'password-protected Agena startup',
      120_000,
    )
    const noAuth = await request(origin + '/api/v1/workbench/preview/sessions/probe/access', {
      method: 'POST',
      body: { backendOrigin: origin, workbenchOrigin: origin },
    })
    assert.equal(noAuth.status, 401)
    const login = await api('POST', '/auth/session', { password })
    assert.equal(login.authenticated, true)
    assert.ok(login.token)
    token = login.token
    await api('POST', '/api/v1/workbench/preview/sessions', {
      id: 'control-loop',
      directory: workspace,
      runDirectory: workspace,
      command: process.execPath,
      args: [],
      logsPath: path.join(temporary, 'control-loop.log'),
      targetUrl: origin,
    })
    await api(
      'POST',
      '/api/v1/workbench/preview/sessions/control-loop/access',
      { backendOrigin: origin, workbenchOrigin: origin },
      403,
    )
    await api('GET', '/api/v1/workbench/preview/s/control-loop/api/v1/workbench/preview/sessions', undefined, 403)
    await api('GET', '/api/v1/workbench/preview/proxy?target=' + encodeURIComponent(origin), undefined, 403)
    await api('DELETE', '/api/v1/workbench/preview/sessions/control-loop')
    check('New and legacy gateways cannot proxy the real Agena control listener')
    await api('POST', '/api/v1/workbench/preview/sessions', {
      id: 'probe',
      directory: workspace,
      runDirectory: workspace,
      agenaSessionId: '-1',
      command: process.execPath,
      args: [__filename, '--fixture', configPath],
      logsPath: path.join(temporary, 'fixture.log'),
      targetUrl: upstream,
    })
    previewCreated = true
    const started = await api('POST', '/api/v1/workbench/preview/sessions/probe/start')
    assert.ok(started.pid > 0)
    await waitFixture(started.pid)
    let grant = await access()
    assert.notEqual(new URL(grant.url).origin, origin)
    assert.equal(new URL(grant.url).pathname, '/')
    assert.equal(new URL(grant.url).search, '')
    assert.ok(grant.expiresAt > Date.now())
    const renewed = await access()
    assert.equal(renewed.url, grant.url)
    assert.equal(renewed.generation, grant.generation)
    check('Password gate and authenticated isolated-origin access; renewal preserves URL')

    const document = await request(grant.url)
    assert.equal(document.status, 200)
    const csp = String(document.headers['content-security-policy'])
    assert.ok(csp.includes("default-src 'self'") && csp.includes("script-src 'self'"))
    assert.ok(csp.includes(origin))
    assert.ok(!csp.includes("frame-ancestors 'none'"))
    assert.equal(document.headers['x-frame-options'], undefined)
    const echo = await request(new URL('/echo', grant.url), {
      headers: { authorization: 'Bearer app-only', cookie: 'manual_app_cookie=kept' },
    })
    assert.equal(echo.json.authorization, 'Bearer app-only')
    assert.equal(echo.json.cookie, 'manual_app_cookie=kept')
    const loginResponse = await request(new URL('/login', grant.url), { method: 'POST' })
    assert.ok(loginResponse.headers['set-cookie'].every((cookie) => !/;\s*domain=/i.test(cookie)))
    assert.ok(loginResponse.headers['set-cookie'].every((cookie) => /;\s*Path=\/(?:;|$)/i.test(cookie)))
    assert.ok(loginResponse.headers['set-cookie'].some((cookie) => cookie.startsWith('domain=' + nonce + ';')))
    const redirect = await request(new URL('/account/login', grant.url))
    assert.equal(redirect.status, 302)
    assert.equal(redirect.headers.location, new URL('/account/verify?step=2', grant.url).href)
    const forbidden = await request(origin + '/api/v1/workbench/preview/sessions', {
      headers: { origin: new URL(grant.url).origin, authorization: 'Bearer ' + token },
    })
    assert.equal(forbidden.status, 403, 'preview Origin must be denied even with a valid control Bearer')
    check('Root proxy preserves app auth/cookies/CSP and rebases nested redirects once')
    check('Preview Origin cannot reach control API even with a valid control Bearer')
    check('Subpath login cookies map to the preview root and preserve a cookie named domain')

    const controlHeaders = { authorization: 'Bearer ' + token }
    const legacyBase = origin + '/api/v1/workbench/preview/s/probe/'
    const appUnauthorized = await request(legacyBase + 'needs-login', { method: 'HEAD', headers: controlHeaders })
    assert.equal(appUnauthorized.status, 401)
    assert.equal(appUnauthorized.headers['x-agena-preview-upstream'], '1')
    const controlUnauthorized = await request(legacyBase + 'needs-login', { method: 'HEAD' })
    assert.equal(controlUnauthorized.status, 401)
    assert.equal(controlUnauthorized.headers['x-agena-preview-upstream'], undefined)
    const legacyEcho = await request(legacyBase + 'echo', {
      headers: { ...controlHeaders, connection: 'keep-alive, x-probe-hop', 'x-probe-hop': 'must-not-reach-app' },
    })
    assert.equal(legacyEcho.json.hop, '')
    assert.equal(legacyEcho.headers['x-upstream-hop'], undefined)
    const encodedPath = 'encoded/a%2Fb%2525?literal=%2F'
    const rawPath = await request(legacyBase + encodedPath, { headers: controlHeaders })
    assert.equal(rawPath.json.path, '/encoded/a%2Fb%2525')
    assert.equal(rawPath.json.query, '?literal=%2F')
    const encodedBody = await request(legacyBase + 'encoded-body', { headers: controlHeaders })
    assert.equal(encodedBody.headers['content-encoding'], 'x-probe-encoding')
    assert.equal(encodedBody.text, '<img src="/opaque-asset">')
    const ranged = await request(legacyBase + 'range', { headers: { ...controlHeaders, range: 'bytes=0-16' } })
    assert.equal(ranged.status, 206)
    assert.equal(ranged.text, '<img src="/range">')
    assert.equal(ranged.headers['content-range'], `bytes 0-${Buffer.byteLength(ranged.text) - 1}/100`)
    check(
      'Legacy diagnostics distinguish application auth from control auth and preserve raw paths, encoding, and byte ranges',
    )
    check('Legacy proxy removes Connection-nominated request and response hop headers')

    chrome = spawn(
      chromeBinary,
      [
        '--headless=new',
        '--no-first-run',
        '--no-default-browser-check',
        '--disable-dev-shm-usage',
        '--no-proxy-server',
        '--remote-debugging-address=127.0.0.1',
        '--remote-debugging-port=0',
        '--user-data-dir=' + profile,
        '--host-resolver-rules=MAP *.localhost 127.0.0.1',
        ...(process.getuid?.() === 0 ? ['--no-sandbox'] : []),
        'about:blank',
      ],
      { stdio: ['ignore', chromeLog.fd, chromeLog.fd] },
    )
    let chromeSpawnError
    chrome.once('error', (error) => {
      chromeSpawnError = error
    })
    const endpoint = await until(
      async () => {
        if (chromeSpawnError) throw chromeSpawnError
        if (chrome.exitCode !== null || chrome.signalCode !== null)
          throw new Error('Probe Chrome exited: ' + (await fs.readFile(chromeLogPath, 'utf8')))
        try {
          const [debugPort, wsPath] = (await fs.readFile(path.join(profile, 'DevToolsActivePort'), 'utf8'))
            .trim()
            .split('\n')
          return debugPort && wsPath ? 'ws://127.0.0.1:' + debugPort + wsPath : null
        } catch {
          return null
        }
      },
      Boolean,
      'isolated Chrome CDP endpoint',
    )
    cdp = await Cdp.connect(endpoint)
    const created = await cdp.send('Target.createTarget', { url: 'about:blank' })
    const attached = await cdp.send('Target.attachToTarget', { targetId: created.targetId, flatten: true })
    pageSession = attached.sessionId
    await cdp.send('Page.enable', {}, pageSession)
    await cdp.send('Runtime.enable', {}, pageSession)
    await cdp.send('Network.enable', {}, pageSession)
    await cdp.send('Page.navigate', { url: origin }, pageSession)
    await until(
      () => cdp.evaluate(pageSession, 'typeof window.installPreview'),
      (value) => value === 'function',
      'workbench shell ready',
    )
    let result = await install(grant)
    assert.equal(result.asset, true)
    assert.equal(result.visible, true)
    assert.equal(result.origin, new URL(grant.url).origin)
    assert.equal(result.spaFallback, true)
    assert.equal(result.redirect.url, new URL('/account/verify?step=2', grant.url).href)
    assert.equal(result.redirect.body.path, '/account/verify')
    assert.equal(result.parentStorageBlocked, true)
    assert.equal(result.parentDomBlocked, true)
    assert.equal(result.appStorage, true)
    assert.equal(result.controlReadBlocked, true)
    assert.equal(result.loginStatus, 200)
    assert.equal(result.appIdentity.authorization, 'Bearer probe-app-' + nonce)
    assert.ok(
      result.appIdentity.cookie.includes('agena_probe=' + nonce),
      'partitioned app login cookie must survive in the iframe',
    )
    assert.ok(result.appIdentity.cookie.includes('domain=' + nonce), 'cookie named domain survives browser round-trip')
    assert.equal(
      await cdp.evaluate(pageSession, "localStorage.getItem('agena-probe-workbench')"),
      'workbench-secret-' + nonce,
    )
    check('Real Chrome iframe renders root assets, SPA fallback, and nested redirects')
    check('Real Chrome isolates parent DOM/storage while app storage and login cookie work')
    check('Real Chrome cannot read control API from preview')
    assert.equal(result.websocketProtocol, 'vite-hmr')
    assert.equal(result.websocketEcho.data, 'probe-echo')
    assert.equal(result.websocketEcho.origin, new URL(upstream).origin)
    assert.ok(result.websocketEcho.cookie.includes('agena_probe=' + nonce))
    assert.equal(result.hmrUpdate.updates[0].path, '/assets/app.js')
    check('Real Chrome WebSocket preserves subprotocol, app cookie, echo, and HMR update transport')

    // Running records cannot be retargeted/renamed. A rejected mutation must
    // leave the existing HTTP and WebSocket grant intact.
    await api('PUT', '/api/v1/workbench/preview/sessions/probe', { targetUrl: upstream }, 409)
    await api('POST', '/api/v1/workbench/preview/sessions/probe/rename', { id: 'probe-renamed' }, 409)
    assert.equal((await request(grant.url)).status, 200)
    assert.equal((await access()).url, grant.url)
    assert.equal(await cdp.evaluate(pageSession, 'window.probeState.closed'), false)
    check('Running update/rename returns 409 without revoking the live preview')

    // An authenticated human can attach an already running development server.
    // Updating that stopped/non-owned registry record revokes only its grant,
    // while the primary runtime-owned process and primary grant remain live.
    await api('POST', '/api/v1/workbench/preview/sessions', {
      id: 'attached',
      directory: workspace,
      runDirectory: workspace,
      agenaSessionId: 'preview-probe-owner',
      command: process.execPath,
      args: [__filename, '--fixture', configPath],
      logsPath: path.join(temporary, 'attached.log'),
      targetUrl: upstream,
    })
    const attachedGrant = await access('attached')
    result = await install(attachedGrant)
    assert.equal(result.websocketEcho.data, 'probe-echo')
    const eventStream = await openEventStream(new URL('/events', attachedGrant.url))
    const legacyEventStream = await openEventStream(
      origin + '/api/v1/workbench/preview/s/attached/events',
      controlHeaders,
    )
    streams.push(eventStream, legacyEventStream)
    assert.equal(eventStream.status, 200)
    assert.equal(legacyEventStream.status, 200)
    await until(
      () => Promise.resolve(eventStream.chunks > 0 && legacyEventStream.chunks > 0),
      Boolean,
      'active SSE streams',
    )
    const pendingHeaders = request(new URL('/wait-headers', attachedGrant.url), { timeout: 8_000 }).then(
      (response) => ({ response }),
      (error) => ({ error: String(error) }),
    )
    await until(
      async () => (await fs.readFile(requestsPath, 'utf8')).includes('/mount/wait-headers'),
      Boolean,
      'waiting upstream headers',
    )
    await api('PUT', '/api/v1/workbench/preview/sessions/attached', {
      args: [__filename, '--fixture', configPath, '--new-generation'],
    })
    await until(
      () => cdp.evaluate(pageSession, 'window.probeState.closed'),
      Boolean,
      'attached WebSocket grant revocation',
      6_000,
    )
    assert.equal((await request(upstream + '__ready')).status, 200)
    assert.equal((await request(grant.url)).status, 200)
    assert.equal((await request(attachedGrant.url)).status, 403)
    await until(
      () => Promise.resolve(eventStream.closed && legacyEventStream.closed),
      Boolean,
      'revoked HTTP and legacy SSE streams',
      4_000,
    )
    const headerResult = await pendingHeaders
    assert.equal(headerResult.response?.status, 403, 'revocation aborts an upstream response still waiting for headers')
    await api('DELETE', '/api/v1/workbench/preview/sessions/attached')
    result = await install(grant)
    assert.equal(result.websocketEcho.data, 'probe-echo')
    check('Revoked attached WebSocket closes while primary app and grant remain running')
    check(
      'Revocation closes isolated and legacy SSE streams and interrupts pending response headers without stopping the app',
    )

    if (probeAgent) {
      const operatorWorkspace = await api('POST', '/api/v1/workspaces/resolve', {
        path: workspace,
        create_if_missing: true,
      })
      const otherWorkspacePath = path.join(temporary, 'other-project')
      await fs.mkdir(otherWorkspacePath)
      const otherWorkspace = await api('POST', '/api/v1/workspaces/resolve', {
        path: otherWorkspacePath,
        create_if_missing: true,
      })
      const tools = await api('GET', '/api/v1/operator/tools')
      assert.ok(Array.isArray(tools), 'operator discovery returns the tool descriptor array')
      const browserTools = new Map(
        tools.filter((tool) => tool.plugin_id === 'agena.web').map((tool) => [tool.name.split('.').at(-1), tool.name]),
      )
      for (const name of [
        'browser_list',
        'browser_open',
        'browser_snapshot',
        'browser_click',
        'browser_wait',
        'browser_close',
      ]) {
        assert.ok(browserTools.has(name), 'enabled operator tool exists: ' + name)
      }
      invokeAgentTool = async (
        name,
        input = {},
        { rejected = false, workspaceId = operatorWorkspace.id, status } = {},
      ) => {
        const response = await request(origin + '/api/v1/operator/tools/invoke', {
          method: 'POST',
          headers: { authorization: 'Bearer ' + token },
          timeout: 90_000,
          body: { workspace_id: workspaceId, tool: browserTools.get(name), input },
        })
        if (rejected) {
          assert.ok(response.status >= 400, name + ' must reject this request: ' + response.text)
          if (status) assert.equal(response.status, status, response.text)
          assert.ok(!response.json?.payload?.session_id, 'rejected call must not expose an opened browser target')
          return { status: response.status, error: response.json }
        }
        assert.equal(response.status, 200, name + ': ' + response.text)
        assert.ok(
          response.json && response.json.payload,
          name + ' must return its structured tool payload: ' + response.text,
        )
        return response.json.payload
      }
      const foreignRecords = [
        { id: 'foreign-session', directory: workspace, agenaSessionId: '999999999' },
        { id: 'foreign-workspace', directory: otherWorkspacePath, agenaSessionId: '-1' },
      ]
      for (const record of foreignRecords) {
        await api('POST', '/api/v1/workbench/preview/sessions', {
          ...record,
          runDirectory: record.directory,
          command: process.execPath,
          args: [],
          logsPath: path.join(temporary, record.id + '.log'),
          targetUrl: upstream,
        })
      }
      const listed = await invokeAgentTool('browser_list')
      assert.ok(listed.previews.some((preview) => preview.preview_id === 'probe' && preview.state === 'running'))
      for (const record of foreignRecords) {
        assert.ok(
          !listed.previews.some((preview) => preview.preview_id === record.id),
          'foreign preview must not be discovered: ' + record.id,
        )
        await invokeAgentTool('browser_open', { preview_id: record.id, timeout_ms: 5_000 }, { rejected: true })
      }
      await invokeAgentTool('browser_list', {}, { rejected: true, workspaceId: otherWorkspace.id, status: 409 })
      const publicDenied = await invokeAgentTool(
        'browser_open',
        { url: upstream, timeout_ms: 5_000 },
        { rejected: true },
      )
      check('Real operator browser discovery is owner/workspace scoped; raw loopback URL stays denied', {
        rawLoopbackStatus: publicDenied.status,
      })

      const opened = await invokeAgentTool('browser_open', { preview_id: 'probe', timeout_ms: 20_000 })
      agentSession = opened.session_id
      assert.equal(typeof agentSession, 'string')
      assert.ok(agentSession.length > 0)
      assert.equal(opened.preview_id, 'probe')
      assert.ok(opened.preview_generation)
      assert.ok(opened.managed_preview_network_policy.includes('upstream'))
      assert.notEqual(new URL(opened.snapshot.url).origin, origin)
      assert.notEqual(
        new URL(opened.snapshot.url).origin,
        new URL(grant.url).origin,
        'agent and human preview grants use separate hosts',
      )
      const snapshot = await invokeAgentTool('browser_snapshot', { session_id: agentSession })
      assert.ok(snapshot.snapshot.text.includes('Preview fixture: root asset loaded'))
      const clicked = await invokeAgentTool('browser_click', {
        session_id: agentSession,
        selector: '#probe-click',
        timeout_ms: 10_000,
      })
      assert.ok(clicked.result.snapshot.text.includes('Managed browser click verified'))
      await invokeAgentTool('browser_click', {
        session_id: agentSession,
        selector: '#probe-connect',
        timeout_ms: 10_000,
      })
      const blocked = await invokeAgentTool('browser_wait', {
        session_id: agentSession,
        text: 'CSP blocked external connection',
        timeout_ms: 10_000,
      })
      assert.ok(blocked.result.snapshot.text.includes('CSP blocked external connection'))
      check('Real browser_open resolves provider grant, runs CDP snapshot/click, and enforces external-connect CSP')
      for (const record of foreignRecords) await api('DELETE', '/api/v1/workbench/preview/sessions/' + record.id)
    }

    if (process.env.PROBE_SCREENSHOT) {
      const screenshot = await cdp.send('Page.captureScreenshot', { format: 'png' }, pageSession)
      await fs.writeFile(path.resolve(process.env.PROBE_SCREENSHOT), Buffer.from(screenshot.data, 'base64'))
    }
    await api('POST', '/api/v1/workbench/preview/sessions/probe/stop')
    await until(
      () => cdp.evaluate(pageSession, 'window.probeState.closed'),
      Boolean,
      'WebSocket stop revocation',
      6_000,
    )
    assert.equal((await request(grant.url)).status, 403)
    await api(
      'POST',
      '/api/v1/workbench/preview/sessions/probe/access',
      { backendOrigin: origin, workbenchOrigin: origin },
      409,
    )
    await api('GET', '/api/v1/workbench/preview/s/probe/', undefined, 409)
    check('A stopped managed preview cannot be reopened as an unowned external target')
    if (agentSession) {
      const stale = await invokeAgentTool('browser_snapshot', { session_id: agentSession }, { rejected: true })
      const closed = await invokeAgentTool('browser_close', { session_id: agentSession })
      assert.equal(closed.closed, true)
      agentSession = undefined
      check('Real browser tool rejects the stopped run while still permitting owner cleanup', {
        stoppedActionStatus: stale.status,
      })
    }
    const restarted = await api('POST', '/api/v1/workbench/preview/sessions/probe/start')
    await waitFixture(restarted.pid)
    const nextGrant = await access()
    assert.notEqual(nextGrant.generation, grant.generation)
    assert.notEqual(nextGrant.url, grant.url)
    assert.equal((await request(nextGrant.url)).status, 200)
    assert.equal((await request(grant.url)).status, 403)
    check('Stop/restart retires old HTTP and WS access and issues a new run identity')
    const observations = await fs.readFile(requestsPath, 'utf8')
    assert.ok(observations.includes('Bearer probe-app-' + nonce), 'fixture captures actual application auth requests')
    assert.ok(!observations.includes(token), 'workbench Bearer must never reach upstream app')
    check('Workbench Bearer is absent from all upstream fixture requests')
    // Keep the preview and browser connections live: a deployment SIGTERM must
    // stop owned children itself, without a preceding preview Stop request.
    server.kill('SIGTERM')
    await until(
      () => Promise.resolve(server.exitCode !== null || server.signalCode !== null),
      Boolean,
      'SIGTERM graceful server shutdown',
      9_000,
    )
    assert.equal(
      server.signalCode,
      null,
      'SIGTERM must be handled gracefully instead of terminating the server directly',
    )
    assert.equal(server.exitCode, 0)
    await until(
      async () => {
        const argv = await fs.readFile('/proc/' + restarted.pid + '/cmdline', 'utf8').catch(() => '')
        return !argv.split('\0').includes(configPath)
      },
      Boolean,
      'SIGTERM cleans up the owned preview process',
      4_000,
    )
    check('Deployment SIGTERM gracefully closes the server and its running preview process')
  } catch (error) {
    failure = error
  } finally {
    failure ||= interrupted
    interrupted = undefined
    process.removeListener('SIGINT', signalHandler)
    process.removeListener('SIGTERM', signalHandler)
    const cleanupErrors = []
    for (const stream of streams) stream.close()
    if (agentSession && invokeAgentTool && server?.exitCode === null && server?.signalCode === null) {
      try {
        await invokeAgentTool('browser_close', { session_id: agentSession })
      } catch (error) {
        cleanupErrors.push('agent browser close: ' + error)
      }
    }
    if (cdp) {
      try {
        cdp.close()
      } catch (error) {
        cleanupErrors.push(String(error))
      }
    }
    try {
      await terminate(chrome)
    } catch (error) {
      cleanupErrors.push(String(error))
    }
    if (previewCreated && server?.exitCode === null && server?.signalCode === null) {
      try {
        await api('POST', '/api/v1/workbench/preview/sessions/probe/stop')
      } catch (error) {
        cleanupErrors.push('preview stop: ' + error)
      }
    }
    try {
      await terminate(server, 'SIGINT')
    } catch (error) {
      cleanupErrors.push(String(error))
    }
    // ManagedChild should already have stopped this fixture. Only clean up an
    // orphan if its live Linux argv still matches this exact private config.
    try {
      const ready = JSON.parse(await fs.readFile(readyPath, 'utf8'))
      if (ready.nonce === nonce && ready.configPath === configPath && Number.isInteger(ready.pid) && ready.pid > 1) {
        const argv = await fs.readFile('/proc/' + ready.pid + '/cmdline', 'utf8').catch(() => '')
        if (argv.split('\0').includes(configPath) && argv.split('\0').includes('--fixture')) {
          process.kill(ready.pid, 'SIGTERM')
          await until(
            async () => {
              const current = await fs.readFile('/proc/' + ready.pid + '/cmdline', 'utf8').catch(() => '')
              return !current.split('\0').includes(configPath)
            },
            Boolean,
            'owned fixture cleanup',
            3_000,
          )
        }
      }
    } catch (error) {
      if (error.code !== 'ENOENT' && error.code !== 'ESRCH') cleanupErrors.push(String(error))
    }
    if (cleanupErrors.length) failure ||= new Error('cleanup failed: ' + cleanupErrors.join('; '))
    const diagnostics = failure
      ? {
          serverLog: (await fs.readFile(serverLogPath, 'utf8').catch(() => '')).slice(-12_000),
          fixtureLog: (await fs.readFile(path.join(temporary, 'fixture.log'), 'utf8').catch(() => '')).slice(-12_000),
          chromeLog: (await fs.readFile(chromeLogPath, 'utf8').catch(() => '')).slice(-6_000),
        }
      : undefined
    await serverLog.close()
    await chromeLog.close()
    const report = {
      status: failure ? 'failed' : 'passed',
      agentToolProbe: probeAgent,
      checks,
      error: failure?.stack,
      cleanupErrors,
      diagnostics,
      node: process.version,
      binary,
      testedAt: new Date().toISOString(),
    }
    // Never retain a live credential in the report, including startup logs.
    let serialized = JSON.stringify(report, null, 2)
    for (const secret of [password, token].filter(Boolean)) serialized = serialized.split(secret).join('[redacted]')
    await fs.writeFile(reportPath, serialized + '\n')
    console.log('Report: ' + reportPath)
  }
  if (failure) throw failure
  console.log(`Workspace preview probe passed (${checks.length} checks).`)
}

if (process.argv[2] === '--fixture') {
  fixtureMain(process.argv[3]).catch((error) => {
    console.error(error)
    process.exitCode = 1
  })
} else {
  main().catch((error) => {
    console.error(error.stack || error)
    process.exitCode = 1
  })
}
