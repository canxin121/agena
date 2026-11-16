const fs = require('node:fs/promises')
const path = require('node:path')
const { Readable, Writable } = require('node:stream')

// Chrome's pipe transports CDP over inherited descriptors 3/4. It opens no
// listening port and uses the same Cdp class as the socket-based probe.
class PipeSocket {
  constructor(chrome) {
    this.chrome = chrome
    this.listeners = new Map()
    this.sessionId = null
    let buffer = ''
    chrome.stdio[4].setEncoding('utf8')
    chrome.stdio[4].on('data', (chunk) => {
      buffer += chunk
      let end
      while ((end = buffer.indexOf('\0')) >= 0) {
        const data = buffer.slice(0, end)
        buffer = buffer.slice(end + 1)
        for (const listener of this.listeners.get('message') || []) listener({ data })
      }
    })
  }
  addEventListener(name, listener) {
    const listeners = this.listeners.get(name) || []
    listeners.push(listener)
    this.listeners.set(name, listeners)
  }
  send(data) {
    const message = JSON.parse(data)
    if (this.sessionId) message.sessionId = this.sessionId
    this.chrome.stdio[3].write(JSON.stringify(message) + '\0')
  }
  close() {
    this.chrome.stdio[3].end()
  }
}

async function prepareOfflineConfig(web, temporary) {
  // Keep the project's real plugins/settings while running its config from
  // /tmp without Vite's usual bundled-config cache in node_modules.
  let source = await fs.readFile(path.join(web, 'vite.config.ts'), 'utf8')
  for (const [specifier, target] of [
    ['vite', require.resolve('vite', { paths: [web] })],
    ['@vitejs/plugin-vue', require.resolve('@vitejs/plugin-vue', { paths: [web] })],
    ['./scripts/remixiconTreeShaking', path.join(web, 'scripts/remixiconTreeShaking.ts')],
  ])
    source = source.replace(`from '${specifier}'`, `from ${JSON.stringify(target)}`)
  const config = path.join(temporary, 'vite.config.ts')
  await fs.writeFile(config, `const __dirname = ${JSON.stringify(web)}\n` + source)
  return config
}

async function buildOfflineApp(web, temporary) {
  const { build } = await import('vite')
  const entry = path.join(temporary, 'entry.ts')
  const source = (await fs.readFile(entry, 'utf8')).replaceAll("'/src/", "'" + web + '/src/')
  await fs.writeFile(entry, source)
  await fs.writeFile(
    path.join(temporary, 'index.html'),
    '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="app"></div><script type="module" src="./entry.ts"></script></body></html>',
  )
  const directory = path.join(temporary, 'dist')
  await build({
    root: temporary,
    configFile: path.join(temporary, 'vite.config.ts'),
    configLoader: 'native',
    cacheDir: path.join(temporary, 'vite-cache'),
    publicDir: path.join(web, 'public'),
    css: { postcss: web },
    logLevel: 'warn',
    define: { __VUE_PROD_DEVTOOLS__: 'true' },
    resolve: {
      alias: {
        vue: path.join(web, 'node_modules/vue/dist/vue.runtime.esm-bundler.js'),
        pinia: path.join(web, 'node_modules/pinia/dist/pinia.mjs'),
      },
    },
    build: { outDir: directory, emptyOutDir: true, minify: false, cssMinify: false, sourcemap: false },
  })
  return directory
}

async function fixtureResponse(middleware, request) {
  const req = Readable.from(request.postData ? [request.postData] : [])
  req.url = new URL(request.url).pathname + new URL(request.url).search
  req.method = request.method
  req.headers = Object.fromEntries(
    Object.entries(request.headers || {}).map(([name, value]) => [name.toLowerCase(), value]),
  )
  const chunks = []
  const headers = new Map()
  const response = new Writable({
    write(chunk, _encoding, callback) {
      chunks.push(Buffer.from(chunk))
      callback()
    },
  })
  response.statusCode = 200
  response.setHeader = (name, value) => {
    headers.set(name.toLowerCase(), String(value))
  }
  response.getHeader = (name) => headers.get(name.toLowerCase())
  response.writeHead = (code, values) => {
    response.statusCode = code
    for (const [name, value] of Object.entries(values || {})) response.setHeader(name, value)
    return response
  }
  await middleware(req, response, () => {
    response.statusCode = 404
    response.end('Unknown fixture endpoint')
  })
  // Fixture event streams contain only a keepalive. Complete that in-memory
  // response; all reconnects are intercepted in the same way.
  if (!response.writableEnded) response.end()
  return {
    responseCode: response.statusCode,
    responseHeaders: [...headers].map(([name, value]) => ({ name, value })),
    body: Buffer.concat(chunks).toString('base64'),
  }
}

function offlineRequestHandler({ cdp, base, directory, middleware, errors }) {
  const mime = {
    '.html': 'text/html',
    '.js': 'text/javascript',
    '.css': 'text/css',
    '.json': 'application/json',
    '.svg': 'image/svg+xml',
    '.png': 'image/png',
    '.ico': 'image/x-icon',
    '.woff2': 'font/woff2',
    '.woff': 'font/woff',
    '.ttf': 'font/ttf',
    '.wasm': 'application/wasm',
  }
  return async (event) => {
    try {
      const url = new URL(event.request.url)
      if (['data:', 'blob:'].includes(url.protocol)) {
        await cdp.send('Fetch.continueRequest', { requestId: event.requestId })
        return
      }
      if (url.origin !== base) {
        await cdp.send('Fetch.failRequest', { requestId: event.requestId, errorReason: 'BlockedByClient' })
        return
      }
      let result
      if (/^\/(api|auth|health)(\/|$)/.test(url.pathname)) {
        result = await fixtureResponse(middleware, event.request)
      } else {
        const spa = ['/', '/bot', '/chat', '/assistant', '/integrations', '/tasks'].includes(url.pathname)
        const file = path.resolve(directory, spa ? 'index.html' : '.' + decodeURIComponent(url.pathname))
        if (!file.startsWith(directory + path.sep)) throw new Error('Invalid fixture asset path')
        let body
        try {
          body = await fs.readFile(file)
        } catch (error) {
          if (error.code !== 'ENOENT') throw error
          result = { responseCode: 404, body: '' }
        }
        if (body)
          result = {
            responseCode: 200,
            responseHeaders: [{ name: 'Content-Type', value: mime[path.extname(file)] || 'application/octet-stream' }],
            body: body.toString('base64'),
          }
      }
      await cdp.send('Fetch.fulfillRequest', { requestId: event.requestId, ...result })
    } catch (error) {
      // Aborted navigation/file reads can retire a paused request before its
      // fixture completes; those are intentional cancellation test cases.
      if (/Invalid InterceptionId|Invalid interceptionId|Invalid RequestId|Target closed/.test(String(error))) return
      errors.push('Offline fixture: ' + String(error))
      await cdp
        .send('Fetch.failRequest', { requestId: event.requestId, errorReason: 'BlockedByClient' })
        .catch(() => {})
    }
  }
}

module.exports = { PipeSocket, prepareOfflineConfig, buildOfflineApp, offlineRequestHandler }
