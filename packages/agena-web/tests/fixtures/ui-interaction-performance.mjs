// Production pane ownership, routing, transcript projection and cursor. The
// only fake is session data, so the fixture needs no running Agena backend.
import { computed, createApp, h, onBeforeUnmount, reactive, ref } from 'vue'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { i18n } from '../../src/i18n'
import { useUiStore } from '../../src/stores/ui'
import { useWorkspacePaneContext } from '../../src/app/workspace/workspacePaneContext'
import { provideWorkspacePaneHosts } from '../../src/layout/workspacePaneHosts'
import WorkspaceEditorGroupPane from '../../src/layout/WorkspaceEditorGroupPane.vue'
import WorkspacePaneHost from '../../src/layout/WorkspacePaneHost.vue'
import MessageItem from '../../src/components/chat/MessageItem.vue'
import AgenaDiffBlock from '../../src/components/chat/AgenaDiffBlock.vue'
import { useChatRenderBlocks } from '../../src/pages/chat/useChatRenderBlocks'
import { useChatTranscriptVim } from '../../src/pages/chat/useChatTranscriptVim'
import { transcriptTextProjection } from '../../src/pages/chat/transcriptDomCursor'
import { usePinnedScroll } from '../../src/composables/chat/usePinnedScroll'
import '../../src/style.css'

i18n.global.locale.value = 'en-US'
const params = new URLSearchParams(location.search)
const pairs = Math.max(1, Math.min(500, Number(params.get('messages') || 80)))
const legacy = params.has('legacy')
const fixture = (window.uiPerformanceFixture = { mounts: 0, unmounts: 0, pairs })
fixture.transcriptTextProjection = transcriptTextProjection
const nativeFetch = window.fetch.bind(window)
window.fetch = (input, options) =>
  new URL(String(input), location.href).pathname.startsWith('/api/')
    ? Promise.resolve(Response.json({}))
    : nativeFetch(input, options)
const chat = reactive({
  messages: Array.from({ length: pairs * 2 }, (_, index) => ({
    info: { id: String(index + 1), role: index % 2 ? 'assistant' : 'user' },
    parts:
      index % 2
        ? Array.from({ length: 5 }, (_, part) => ({
            id: `${index + 1}-${part}`,
            agenaKind: 'tool_call',
            partState: 'completed',
            agenaContent: { name: 'shell.exec', input: { command: `echo item-${index}-${part}` } },
            agenaPresentation: {
              title: `Operation ${index}-${part}`,
              blocks: [{ type: 'text', text: `Result ${index}-${part}` }],
            },
          }))
        : [{ id: `user-${index}`, agenaKind: 'text', text: `Review change ${index}. Keep the workspace responsive.` }],
  })),
})
fixture.lastPartId = `${pairs * 2}-4`
const TranscriptPage = {
  setup() {
    fixture.mounts++
    onBeforeUnmount(() => {
      fixture.unmounts++
    })
    const pane = useWorkspacePaneContext()
    const pageRef = ref(null)
    const scroll = usePinnedScroll({
      isVisible: () => pane.isVisible.value,
      sessionId: () => String(pane.route.value.query.sessionId),
    })
    const { scrollEl, contentEl } = scroll
    const render = useChatRenderBlocks({ chat, settings: {}, showThinking: computed(() => true), formatTime: () => '' })
    const vim = useChatTranscriptVim({
      enabled: computed(() => pane.isFocused.value),
      pageRef,
      scrollEl,
      composerRef: ref(null),
      searchInputRef: ref(null),
      selectedSessionId: computed(() => pane.route.value.query.sessionId),
      renderBlocks: render.renderBlocks,
      draft: ref(''),
      clearComposer() {},
      canAbort: computed(() => false),
      abortRun() {},
      toggleHelp() {},
      openPlan() {},
      isPartExpanded: render.transcriptPartExpanded,
      togglePart: (part, expanded) => render.setActivityExpanded(part.key, expanded),
      toasts: { push() {} },
    })
    return () =>
      h('div', { ref: pageRef, class: 'h-full min-h-0' }, [
        h(
          'div',
          {
            ref: scrollEl,
            class: 'h-full overflow-auto p-2',
            'data-fixture-transcript': pane.windowId.value,
            onScroll: scroll.handleScroll,
          },
          [
            h(
              'div',
              { ref: contentEl, 'data-transcript-root': 'true' },
              render.renderBlocks.value.map((block) =>
                h(MessageItem, {
                  key: block.key,
                  message: block.message,
                  displayParts: block.displayParts,
                  showTimestamps: false,
                  formatTime: () => '',
                  copiedMessageId: '',
                  revertBusyMessageId: '',
                  isStreaming: false,
                  collapseSignal: render.activityCollapseSignal.value,
                  activityPageSize: 5,
                  isCompactTouch: false,
                  isPartExpanded: render.transcriptPartExpanded,
                  isNodeSelected: vim.isNodeSelected,
                  isNodeSearchMatch: vim.isNodeSearchMatch,
                  onPartToggle: (part, expanded) => render.setActivityExpanded(part.key, expanded),
                  onNodeSelect: vim.selectNode,
                }),
              ),
            ),
          ],
        ),
      ])
  },
}
const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: '/chat', component: TranscriptPage },
    { path: '/:pathMatch(.*)*', component: TranscriptPage },
  ],
})
await router.push('/chat')
const app = createApp({
  setup() {
    const ui = useUiStore()
    fixture.ui = ui
    provideWorkspacePaneHosts()
    ui.closeAllWorkspaceWindows()
    let serial = 0
    const open = () => ui.openWorkspaceWindow('chat', { query: { sessionId: String(++serial) }, reuseExisting: false })
    fixture.firstId = open()
    const diff = ref('')
    fixture.setDiff = (value) => {
      diff.value = value
    }
    const button = (label, action) => h('button', { class: 'rounded border px-3 py-1', onClick: action }, label)
    return () =>
      h('main', { class: 'flex h-screen flex-col bg-background text-foreground' }, [
        h('nav', { class: 'flex gap-3 p-2' }, [
          button('Open tab', open),
          button('Close tab', () => ui.closeWorkspaceWindow(ui.activeWorkspaceWindowId)),
          button('First tab', () => ui.selectWorkspaceWindow(fixture.firstId)),
          button('Move to split', () => ui.ungroupWorkspaceWindow(ui.activeWorkspaceWindowId)),
          button('Split', () => ui.splitWorkspaceWindowToRight(ui.activeWorkspaceWindowId)),
          button('Merge splits', () => {
            const first = ui.workspaceGroups[0]
            for (const group of [...ui.workspaceGroups].slice(1))
              for (const id of [...group.tabIds]) {
                ui.moveWorkspaceWindowToGroup(id, first.id, { activateTargetGroup: true })
              }
          }),
        ]),
        diff.value ? h('div', { class: 'overflow-auto p-4' }, [h(AgenaDiffBlock, { diff: diff.value })]) : null,
        h(
          'div',
          { class: 'flex min-h-0 flex-1' },
          ui.workspaceGroups.map((group) =>
            h(WorkspaceEditorGroupPane, {
              key: group.id,
              groupId: group.id,
              class: 'min-w-0 flex-1',
            }),
          ),
        ),
        !legacy ? h(WorkspacePaneHost) : null,
      ])
  },
})
app.use(createPinia()).use(i18n).use(router).mount('#app')
