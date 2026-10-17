import { computed, createApp, h, nextTick, reactive, ref } from 'vue'
import { createPinia } from 'pinia'
import { i18n } from '../../src/i18n'
import { useChatStore } from '../../src/stores/chat'
import { useChatRenderBlocks } from '../../src/pages/chat/useChatRenderBlocks'
import { useChatTranscriptVim } from '../../src/pages/chat/useChatTranscriptVim'
import MessageItem from '../../src/components/chat/MessageItem.vue'
import PlanSection from '../../src/components/chat/PlanSection.vue'
import AgenaInteractionPart from '../../src/components/chat/AgenaInteractionPart.vue'
import AgenaDiffBlock from '../../src/components/chat/AgenaDiffBlock.vue'
import AgenaOperationBlock from '../../src/components/chat/AgenaOperationBlock.vue'
import AgenaAttachmentPreview from '../../src/components/chat/AgenaAttachmentPreview.vue'
import CodeBlock from '../../src/components/ui/CodeBlock.vue'
import { installImeCompositionGuard } from '../../src/lib/imeKeyboard'
import '../../src/style.css'

installImeCompositionGuard()
i18n.global.locale.value = 'zh-CN'
const pinia = createPinia()
const chat = useChatStore(pinia)
const state = reactive({ replies: [], failNext: false, clone: false, help: 0, viewKey: 0, clipboard: '', opened: [] })
Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: {
    async writeText(text) {
      state.clipboard = text
    },
  },
})
const code = Array.from({ length: 40 }, (_, i) => `console.log('Keyboard line ${i + 1}: ${'x'.repeat(160)}')`).join(
  '\n',
)
const diff =
  'diff --git a/keyboard.ts b/keyboard.ts\n--- a/keyboard.ts\n+++ b/keyboard.ts\n@@ -0,0 +1,240 @@\n' +
  Array.from({ length: 240 }, (_, i) => `+const line${i} = ${i}`).join('\n')
const option = (label, description = '', custom = false) => ({ label, description, custom })
const question = (text, options = [], multiple = false, allow_custom = true) => ({
  question: text,
  header: '',
  options,
  multiple,
  allow_custom,
})
const requests = [
  {
    request_id: 'ask',
    title: 'Ask fixture',
    input_kind: 'ask_user',
    questions: [
      question('选择部署环境', [option('Staging'), option('Production'), option('自定义回答', '', true)], false, false),
      question(
        '选择需要执行的检查',
        [option('Types', '检查 TypeScript 类型'), option('Build'), option('Lint'), option('自定义检查', '', true)],
        true,
        false,
      ),
      question('补充说明', [option('自定义回答', '请填写补充说明', true)], false, false),
    ],
  },
  {
    request_id: 'review',
    title: 'Approve New Plan',
    input_kind: 'review',
    body_markdown:
      '# Keyboard plan\n\n1. Inspect keyboard behavior.\n2. Fix focus and translations.\n\n```js\n' + code + '\n```',
    questions: [
      question('Choose whether this plan should move to active, or type feedback for the agent to revise the plan.', [
        option('Approve with autorun on', 'Approve the plan, move it to active, and keep autorun on.'),
        option('Approve with autorun off', 'Approve the plan, move it to active, and keep autorun off.'),
        option('Keep in planning', 'Return to planning so the plan can be edited further.'),
        option('Reject', 'Reject the current plan and mark the review as rejected.'),
        option('Cancel plan', 'Cancel the plan entirely and stop work on it.'),
      ]),
    ],
  },
]
const message = reactive({
  info: { id: 'reply', role: 'assistant', runState: 'in_progress' },
  parts: requests
    .map((request, index) => ({
      id: String(index + 1),
      agenaKind: 'tool_call',
      agenaRole: 'assistant',
      partState: 'in_progress',
      revision: 1,
      agenaContent: {
        name: index ? 'plan.review' : 'interaction.ask',
        user_input: { requests: [{ request, reply: null }] },
      },
      agenaPresentation: { title: request.title, blocks: [] },
    }))
    .concat([
      {
        id: '3',
        agenaKind: 'tool_call',
        agenaRole: 'assistant',
        partState: 'in_progress',
        revision: 1,
        agenaContent: {
          name: 'shell.exec',
          authorization: {
            permissions: [
              { request: { request_id: 'permission', action: { kind: 'tool', tool_name: 'shell.exec' } }, reply: null },
            ],
          },
        },
        agenaPresentation: { title: 'Permission fixture', blocks: [] },
      },
    ]),
})
const pending = reactive(new Set(['ask', 'review', 'permission']))
chat.getPendingInteraction = (_session, id) => (pending.has(id) ? { requestId: id } : null)
chat.hasInteractionSnapshot = () => true
async function reply(id, payload) {
  await new Promise((resolve) => setTimeout(resolve, 40))
  if (state.failNext) {
    state.failNext = false
    throw new Error('Fixture reply failed')
  }
  state.replies.push({ id, payload })
  const part = message.parts.find(
    (part) =>
      (part.agenaContent.user_input?.requests[0].request.request_id ||
        part.agenaContent.authorization?.permissions[0].request.request_id) === id,
  )
  const entry = part?.agenaContent.user_input?.requests[0] || part?.agenaContent.authorization?.permissions[0]
  if (entry) entry.reply = payload
  pending.delete(id)
  return true
}
chat.replyQuestion = (_session, id, answers) =>
  reply(id, { answers: Object.fromEntries(answers.map((answer, i) => [String(i), answer])) })
chat.rejectQuestion = (_session, id) => reply(id, { cancelled: true })
chat.replyPermission = (_session, id, decision) => reply(id, { decision })

createApp({
  setup() {
    const pageRef = ref(null),
      scrollEl = ref(null),
      composer = ref(null),
      draft = ref('')
    const planOpen = ref(false)
    const plan = {
      loading: ref(false),
      toggling: ref(false),
      error: ref(''),
      autorun: ref(true),
      snapshot: ref({ title: 'Plan fixture' }),
      markdown: ref(
        '# Plan fixture\n\n## Tasks\n\n' +
          Array.from({ length: 80 }, (_, i) => `${i + 1}. Keyboard task ${i + 1}\n`).join('\n') +
          '\n\n## Code sample\n\n```js\n' +
          code +
          '\n```',
      ),
      async refresh() {
        plan.loading.value = true
        await new Promise((resolve) => setTimeout(resolve, 40))
        plan.loading.value = false
      },
      async toggleAutorun() {
        if (plan.loading.value || plan.toggling.value) return
        plan.toggling.value = true
        await new Promise((resolve) => setTimeout(resolve, 40))
        plan.autorun.value = !plan.autorun.value
        plan.toggling.value = false
      },
    }
    const render = useChatRenderBlocks({
      chat: { messages: [message] },
      settings: {},
      showThinking: computed(() => true),
      formatTime: () => '',
    })
    const vim = useChatTranscriptVim({
      enabled: ref(true),
      pageRef,
      scrollEl,
      composerRef: computed(() => ({ textareaEl: { element: composer.value, focus: () => composer.value?.focus() } })),
      searchInputRef: ref(null),
      selectedSessionId: ref('1'),
      renderBlocks: render.renderBlocks,
      draft,
      clearComposer() {
        draft.value = ''
      },
      canAbort: ref(false),
      abortRun() {},
      toggleHelp() {
        state.help++
      },
      openPlan() {
        planOpen.value = true
      },
      isPartExpanded: render.transcriptPartExpanded,
      togglePart: (part, expanded) => render.setActivityExpanded(part.key, expanded),
      toasts: { push() {} },
    })
    window.interactionKeyboardFixture = {
      state,
      pending,
      message,
      render,
      plan,
      planOpen,
      locale(locale) {
        i18n.global.locale.value = locale
      },
      async revise() {
        message.parts.forEach((part) => part.revision++)
        await nextTick()
      },
    }
    return () =>
      h('main', { ref: pageRef, class: 'mx-auto max-w-4xl space-y-4 p-4' }, [
        h('button', { id: 'before' }, 'Before interactions'),
        h(
          'div',
          { ref: scrollEl, 'data-transcript-root': 'true', class: 'max-h-[65vh] overflow-auto' },
          render.renderBlocks.value.map((block) =>
            h(MessageItem, {
              key: `${block.key}:${state.viewKey}`,
              message: block.message,
              displayParts: block.displayParts,
              showTimestamps: false,
              formatTime: () => '',
              copiedMessageId: '',
              revertBusyMessageId: '',
              isStreaming: true,
              collapseSignal: 0,
              activityPageSize: 12,
              isCompactTouch: false,
              sessionId: '1',
              isPartExpanded: render.transcriptPartExpanded,
              isNodeSelected: vim.isNodeSelected,
              isNodeSearchMatch: vim.isNodeSearchMatch,
              onPartToggle: (part, expanded) => render.setActivityExpanded(part.key, expanded),
              onNodeSelect: vim.selectNode,
            }),
          ),
        ),
        h('button', { id: 'after' }, 'After interactions'),
        h('textarea', { ref: composer, id: 'composer', 'data-chat-input': 'true', 'aria-label': 'Composer' }),
        h(PlanSection, {
          state: plan,
          label: 'Plan',
          open: planOpen.value,
          'onUpdate:open': (open) => {
            planOpen.value = open
          },
        }),
        h('section', { 'data-fixture-code': 'true' }, h(CodeBlock, { code, lang: 'js' })),
        h('section', { 'data-fixture-diff': 'true' }, h(AgenaDiffBlock, { diff })),
        h('section', { 'data-fixture-resources': 'true', class: 'space-y-1.5' }, [
          h(AgenaOperationBlock, {
            block: {
              type: 'search_results',
              results: [
                {
                  title: '交互部件导航参考与设计说明 Keyboard navigation reference'.repeat(4),
                  uri: 'https://example.test/reference/keyboard-navigation',
                  snippet: '可操作的资源具有独立边界，说明文字继续保持可读。',
                },
                { title: '无链接的上下文', snippet: '没有目标地址时，不显示假的操作入口。' },
              ],
            },
          }),
          h(AgenaAttachmentPreview, {
            attachment: {
              key: 'keyboard-reference',
              label: '交互部件键盘导航设计参考 keyboard-navigation-reference.md',
              kind: 'file',
              mime: 'text/markdown',
              path: 'docs/keyboard-navigation-reference.md',
              url: '',
              sizeBytes: 4096,
              width: null,
              height: null,
              durationMs: null,
              pageCount: null,
            },
            onOpen: (path, url) => state.opened.push({ path, url }),
          }),
        ]),
        state.clone
          ? h(AgenaInteractionPart, {
              interaction: render.renderBlocks.value[0].displayParts[1] && {
                requestId: 'review',
                title: 'Approve New Plan',
                kind: 'review',
                bodyMarkdown: '',
                pending: true,
                reply: null,
                questions: requests[1].questions.map((question, index) => ({
                  ...question,
                  questionId: String(index),
                  allowCustom: true,
                })),
              },
              sessionId: '1',
            })
          : null,
      ])
  },
})
  .use(pinia)
  .use(i18n)
  .mount('#app')
