import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4/5/hPwAH/QL+ppTFtAAAAABJRU5ErkJggg==',
  'base64',
)
// A short valid PCM clip and VP8 WebM keep playback checks independent of codecs
// installed on the server and avoid network access to external fixture hosts.
const wav = Buffer.alloc(44 + 1600)
wav.write('RIFF', 0)
wav.writeUInt32LE(wav.length - 8, 4)
wav.write('WAVEfmt ', 8)
wav.writeUInt32LE(16, 16)
wav.writeUInt16LE(1, 20)
wav.writeUInt16LE(1, 22)
wav.writeUInt32LE(8000, 24)
wav.writeUInt32LE(16000, 28)
wav.writeUInt16LE(2, 32)
wav.writeUInt16LE(16, 34)
wav.write('data', 36)
wav.writeUInt32LE(1600, 40)
const webm = Buffer.from(
  'GkXfo59ChoEBQveBAULygQRC84EIQoKEd2VibUKHgQJChYECGFOAZwH/////////EU2bdKtNu4tTq4QVSalmU6yBoU27i1OrhBZUrmtTrIHYTbuMU6uEElTDZ1OsggEe7AEAAAAAAABoAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAVSalmsirXsYMPQkBNgI1MYXZmNjAuMTYuMTAwV0GNTGF2ZjYwLjE2LjEwMESJiEBpAAAAAAAAFlSua8GuAQAAAAAAADjXgQFzxYh0UlRnQDmOwpyBACK1nIN1bmSIgQCGhVZfVlA4g4EBI+ODhAvrwgDgibCBELqBEJqBAhJUw2fYc3OgY8CAZ8iaRaOHRU5DT0RFUkSHjUxhdmY2MC4xNi4xMDBzc7JjwItjxYh0UlRnQDmOwmfIoUWjh0VOQ09ERVJEh5RMYXZjNjAuMzEuMTAyIGxpYnZweB9DtnWo54EAo6OBAACAEAIAnQEqEAAQAABHCIWFiIWEiAICAAwNYAD+/6tQgA==',
  'base64',
)
export default defineConfig({
  root,
  resolve: { alias: { '@': path.join(root, 'src') } },
  plugins: [
    vue(),
    {
      name: 'deterministic-attachment-server',
      configureServer(server) {
        const requests = []
        let unsafeExecutions = 0
        server.middlewares.use((request, response, next) => {
          const url = new URL(request.url || '/', 'http://localhost')
          if (url.pathname === '/probe/requests') {
            response.setHeader('content-type', 'application/json')
            response.end(JSON.stringify(requests))
            return
          }
          if (url.pathname === '/probe/executed') {
            unsafeExecutions += 1
            response.end()
            return
          }
          if (url.pathname === '/probe/executions') {
            response.setHeader('content-type', 'application/json')
            response.end(JSON.stringify(unsafeExecutions))
            return
          }
          if (!url.pathname.startsWith('/api/')) {
            next()
            return
          }
          if (!['/api/v1/sessions/43/media', '/api/v1/workbench/fs/raw'].includes(url.pathname)) {
            response.statusCode = 403
            response.end()
            return
          }
          const authenticated = request.headers.authorization === 'Bearer attachment-fixture-token'
          requests.push({
            path: url.searchParams.get('path'),
            download: url.searchParams.get('download'),
            authenticated,
            url: request.url,
          })
          if (!authenticated) {
            response.statusCode = 401
            response.setHeader('content-type', 'application/json')
            response.end(
              JSON.stringify({ problem: { code: 'auth.required', user: { fallback: 'Server password is required' } } }),
            )
            return
          }
          const requestedPath = url.searchParams.get('path') || ''
          const [body, mime] = requestedPath.endsWith('.wav')
            ? [wav, 'audio/wav']
            : requestedPath.endsWith('.webm')
              ? [webm, 'video/webm']
              : requestedPath.endsWith('.txt')
                ? [Buffer.from('fixture report\n'), 'text/plain']
                : requestedPath.endsWith('.html')
                  ? [Buffer.from('<script>fetch("/probe/executed")</script>'), 'application/octet-stream']
                  : [png, 'image/png']
          response.setHeader('content-type', mime)
          response.setHeader('content-length', body.length)
          response.setHeader(
            'content-disposition',
            requestedPath.endsWith('.html')
              ? 'attachment; filename="active.html"'
              : `${url.searchParams.get('download') === 'true' ? 'attachment' : 'inline'}; filename="flower.png"; filename*=UTF-8''${encodeURIComponent('向日葵.png')}`,
          )
          response.setHeader('cache-control', 'private, no-store')
          response.setHeader('x-content-type-options', 'nosniff')
          response.end(request.method === 'HEAD' ? undefined : body)
        })
      },
    },
  ],
})
