import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'

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

test('the settings sidebar renders one row per layer and highlights the selected one', async () => {
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
  for (const label of [
    'Effective Permission',
    'Global Permission',
    'Workspace Permission',
    'Current Session Permission',
  ]) {
    assert.ok(workspace.includes(label), `missing layer row: ${label}`)
  }
  // Only the open page shows its layers, one indent step below the page row.
  assert.equal(workspace.match(/padding-left:44px/g)?.length, 4)
  assert.equal(workspace.match(/aria-current="page"/g)?.length, 1)
  assert.equal(activeRowLabel(workspace), 'Workspace Permission')

  const session = await render({ activeTab: 'permissions', activeView: 'network', activeScope: 'session' })
  assert.equal(activeRowLabel(session), 'Current Session Permission')
  assert.ok(session.indexOf('Current Session Permission') > session.indexOf('Network'))

  // A page without a layer choice renders no third level at all.
  const plain = await render({ activeTab: 'models-providers', activeView: 'provider-studio', activeScope: '' })
  assert.equal(plain.includes('Effective Permission'), false)
  assert.equal(plain.match(/padding-left:44px/g), null)
  assert.equal(plain.match(/aria-current="page"/g)?.length, 1)
  assert.equal(activeRowLabel(plain), 'Provider Studio')
})

test('the section header names the active layer once for the open page', async () => {
  const { buildSettingsSubpages } = await vite.ssrLoadModule('/src/components/settings/settingsNavigationCatalog.ts')
  const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
  const { settingsText } = await vite.ssrLoadModule('/src/i18n/settingsText.ts')
  const { default: SettingsSectionWorkbench } = await vite.ssrLoadModule(
    '/src/components/settings/workbench/SettingsSectionWorkbench.vue',
  )
  i18n.global.locale.value = 'en-US'

  async function render(section: string, activePage: string, activeScope: string) {
    const pages = buildSettingsSubpages(section)
    const app = createSSRApp({
      render: () =>
        h(
          SettingsSectionWorkbench,
          { section, pages, defaultPage: pages[0]?.id || '', activePage, activeScope },
          { default: () => 'page body' },
        ),
    })
    app.config.globalProperties.$st = settingsText
    app.use(i18n)
    return renderToString(app)
  }

  const scoped = await render('models-providers', 'defaults', 'workspace')
  assert.ok(scoped.includes('Model defaults'))
  assert.ok(scoped.includes('· Workspace default'))
  // The layer is named by the header, not by a second block of page text.
  assert.equal(scoped.match(/Workspace default/g)?.length, 1)

  const unscoped = await render('models-providers', 'provider-studio', '')
  assert.ok(unscoped.includes('Provider Studio'))
  assert.equal(unscoped.includes('Workspace default'), false)
})
