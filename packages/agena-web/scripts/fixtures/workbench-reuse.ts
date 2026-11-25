import { createApp, h, type Component } from 'vue'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter, RouterView } from 'vue-router'
import { useDeviceRuntime } from '@/app/runtime/useDeviceRuntime'
import AppDialogHost from '@/components/AppDialogHost.vue'
import { i18n } from '@/i18n'
import { settingsText } from '@/i18n/settingsText'
import { useDirectoryStore } from '@/stores/directory'
import { useUiStore } from '@/stores/ui'
import '@/style.css'

const cases: Record<string, { path: string; load: () => Promise<{ default: Component }> }> = {
  'terminal-dock': { path: '/chat', load: () => import('@/features/terminal/components/TerminalDockPanel.vue') },
  'preview-dock': {
    path: '/chat',
    load: () => import('@/features/workspacePreview/components/WorkspacePreviewDockPanel.vue'),
  },
  assistant: { path: '/assistant', load: () => import('@/features/assistant/AssistantPanel.vue') },
  files: { path: '/files', load: () => import('@/pages/FilesPage.vue') },
  git: { path: '/git', load: () => import('@/pages/GitPage.vue') },
  settings: { path: '/settings/:section', load: () => import('@/pages/SettingsPage.vue') },
}

async function mount() {
  const params = new URLSearchParams(location.search)
  const view = params.get('case') || ''
  const fixture = cases[view]
  if (!fixture) throw new Error(`Unknown workbench fixture: ${view}`)
  const embedded = params.has('embedded')
  const dock = view.endsWith('-dock')
  const { default: component } = await fixture.load()
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: fixture.path,
        component,
        props: view === 'assistant' ? { source: 42 } : { ...(embedded ? { embedded: true } : {}) },
      },
    ],
  })
  const app = createApp({
    setup() {
      useDeviceRuntime()
      const ui = useUiStore()
      // A local Git or Settings sidebar is not controlled by the shell toggle.
      ui.setSidebarOpen(view !== 'git' && view !== 'settings', { preserveWidth: true })
      useDirectoryStore().setDirectory('/fixture')
      return () =>
        h('main', { class: dock ? 'h-full w-full max-w-[430px] sm:ms-16' : 'h-full w-full' }, [
          h(RouterView),
          h(AppDialogHost),
        ])
    },
  })
  app.config.globalProperties.$st = settingsText
  app.use(createPinia()).use(i18n).use(router)
  await router.push(view === 'settings' ? '/settings/interface?view=web-appearance' : fixture.path)
  app.mount('#app')
}

void mount()
