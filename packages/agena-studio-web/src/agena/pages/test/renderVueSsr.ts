import path from 'node:path'

import { afterAll } from 'bun:test'
import { createSSRApp, h, type Component } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createServer, type InlineConfig, type ViteDevServer } from 'vite'

const rootDir = path.resolve(import.meta.dir, '../../../..')
let sharedServerPromise: Promise<ViteDevServer> | null = null

function createTestServer(config: InlineConfig = {}) {
  return createServer({
    root: rootDir,
    server: { middlewareMode: true },
    appType: 'custom',
    optimizeDeps: {
      noDiscovery: true,
    },
    ssr: {
      optimizeDeps: {
        noDiscovery: true,
      },
    },
    ...config,
  })
}

async function getSharedServer() {
  if (!sharedServerPromise) {
    sharedServerPromise = createTestServer()
  }
  return await sharedServerPromise
}

export async function renderVueSsr(
  modulePath: string,
  props: Record<string, unknown>,
  config: InlineConfig = {},
): Promise<string> {
  const server = Object.keys(config).length ? await createTestServer(config) : await getSharedServer()

  try {
    const mod = await server.ssrLoadModule(modulePath)
    const LoadedComponent = mod.default as Component
    const app = createSSRApp({
      render: () => h(LoadedComponent, props),
    })
    return await renderToString(app)
  } finally {
    if (Object.keys(config).length) {
      await server.close()
    }
  }
}

afterAll(async () => {
  if (!sharedServerPromise) return
  const server = await sharedServerPromise
  await server.close()
  sharedServerPromise = null
})
