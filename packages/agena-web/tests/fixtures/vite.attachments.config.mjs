import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4/5/hPwAH/QL+ppTFtAAAAABJRU5ErkJggg==',
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
        server.middlewares.use((request, response, next) => {
          const url = new URL(request.url || '/', 'http://localhost')
          if (url.pathname === '/probe/requests') {
            response.setHeader('content-type', 'application/json')
            response.end(JSON.stringify(requests))
            return
          }
          if (!url.pathname.startsWith('/api/')) {
            next()
            return
          }
          if (url.pathname !== '/api/v1/sessions/43/media') {
            response.statusCode = 403
            response.end()
            return
          }
          requests.push({ path: url.searchParams.get('path'), download: url.searchParams.get('download') })
          response.setHeader('content-type', 'image/png')
          response.setHeader('content-length', png.length)
          response.setHeader(
            'content-disposition',
            `${url.searchParams.get('download') === 'true' ? 'attachment' : 'inline'}; filename="flower.png"; filename*=UTF-8''${encodeURIComponent('向日葵.png')}`,
          )
          response.setHeader('cache-control', 'private, no-store')
          response.setHeader('x-content-type-options', 'nosniff')
          response.end(request.method === 'HEAD' ? undefined : png)
        })
      },
    },
  ],
})
