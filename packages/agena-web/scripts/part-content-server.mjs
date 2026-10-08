import { createServer } from 'vite'

// Isolated optimizer state keeps concurrent unit/build gates from replacing
// dependency URLs in an active visual verification browser.
const server = await createServer({
  cacheDir: 'node_modules/.vite-part-content',
  optimizeDeps: { entries: ['tests/fixtures/part-content.html'] },
  server: { host: '127.0.0.1', port: 5175, strictPort: true },
})
await server.listen()
server.printUrls()
