import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createMemoryHistory, createRouter } from 'vue-router'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  optimizeDeps: { noDiscovery: true, include: [] },
  logLevel: 'error',
})

after(() => vite.close())

/** The one row that carries `aria-current="page"` is the current destination. */
function activeRowLabel(html: string): string {
  const index = html.indexOf('aria-current="page"')
  if (index < 0) return ''
  const match = /typography-ui-label[^>]*>([^<]+)</.exec(html.slice(index))
  return match ? String(match[1]).trim() : ''
}

test('the settings sidebar highlights the page while scopes remain in the workbench', async () => {
  const { buildSettingsSidebarNodes } = await vite.ssrLoadModule(
    '/src/components/settings/settingsNavigationCatalog.ts',
  )
  const { buildSettingsSidebarTabs } = await vite.ssrLoadModule(
    '/src/components/settings/sidebar/settingsSidebarNavigation.ts',
  )
  const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
  const { settingsText } = await vite.ssrLoadModule('/src/i18n/settingsText.ts')
  const { default: SettingsSidebar } = await vite.ssrLoadModule('/src/components/settings/sidebar/SettingsSidebar.vue')

  // `bun test` preloads the English locale in its own module graph; the Vite SSR
  // graph loads a fresh i18n instance, so pin the copy this test asserts on.
  i18n.global.locale.value = 'en-US'

  const tabs = buildSettingsSidebarTabs(
    (_id: string, labelKey: string) => String(i18n.global.t(labelKey)),
    (section: string) => buildSettingsSidebarNodes(section),
  )

  async function render(props: { activeTab: string; activeView: string; activeScope: string }) {
    const app = createSSRApp(SettingsSidebar, { tabs, ...props })
    app.config.globalProperties.$st = settingsText
    app.use(i18n)
    return renderToString(app)
  }

  const workspace = await render({ activeTab: 'permissions', activeView: 'filesystem', activeScope: 'workspace' })
  for (const label of ['Filesystem', 'Network', 'Tools']) assert.ok(workspace.includes(label))
  assert.equal(workspace.includes('Workspace Permission'), false)
  assert.equal(workspace.match(/padding-left:44px/g), null)
  assert.equal(workspace.match(/aria-current="page"/g)?.length, 1)
  assert.equal(activeRowLabel(workspace), 'Filesystem')

  const session = await render({ activeTab: 'permissions', activeView: 'network', activeScope: 'session' })
  assert.equal(activeRowLabel(session), 'Network')
  assert.equal(session.includes('Current Session Permission'), false)

  // A page without a layer choice renders no third level at all.
  const plain = await render({ activeTab: 'models-providers', activeView: 'provider-studio', activeScope: '' })
  assert.equal(plain.includes('Effective Permission'), false)
  assert.equal(plain.match(/padding-left:44px/g), null)
  assert.equal(plain.match(/aria-current="page"/g)?.length, 1)
  assert.equal(activeRowLabel(plain), 'Provider Studio')
})

test('scope toolbar avoids duplicate navigation while sidebar layout remains available', async () => {
  const { buildSettingsSubpages } = await vite.ssrLoadModule('/src/components/settings/settingsNavigationCatalog.ts')
  const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
  const { settingsText } = await vite.ssrLoadModule('/src/i18n/settingsText.ts')
  const { default: SettingsSectionWorkbench } = await vite.ssrLoadModule(
    '/src/components/settings/workbench/SettingsSectionWorkbench.vue',
  )
  i18n.global.locale.value = 'en-US'

  async function render(section: string, activePage: string, activeScope: string, scopeLayout = 'sidebar') {
    const pages = buildSettingsSubpages(section)
    const app = createSSRApp({
      render: () =>
        h(
          SettingsSectionWorkbench,
          { section, pages, defaultPage: pages[0]?.id || '', activePage, activeScope, scopeLayout },
          { default: () => 'page body' },
        ),
    })
    app.config.globalProperties.$st = settingsText
    app.use(i18n)
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: '/settings/:section', component: { render: () => null } }],
    })
    app.use(router)
    await router.push({ path: `/settings/${section}`, query: { view: activePage, scope: activeScope } })
    await router.isReady()
    return renderToString(app)
  }

  const scoped = await render('models-providers', 'model-settings', 'workspace')
  assert.ok(scoped.includes('Model settings'))
  assert.ok(scoped.includes('· Workspace layer'))
  assert.equal(scoped.match(/<h1[^>]*>[\s\S]*?<\/h1>/)?.[0].match(/Workspace layer/g)?.length, 1)

  const toolbar = await render('models-providers', 'model-settings', 'workspace', 'toolbar')
  assert.equal(toolbar.includes('<aside'), false)
  assert.equal(toolbar.match(/aria-current="true"/g)?.length, 1)
  assert.equal(toolbar.match(/Workspace layer/g)?.length, 1)
  assert.equal(toolbar.includes('· Workspace layer'), false)
  for (const label of ['Effective settings', 'Global layer', 'Workspace layer']) assert.ok(toolbar.includes(label))

  const unscoped = await render('models-providers', 'provider-studio', '')
  assert.ok(unscoped.includes('Provider Studio'))
  assert.equal(unscoped.includes('Workspace layer'), false)
})
