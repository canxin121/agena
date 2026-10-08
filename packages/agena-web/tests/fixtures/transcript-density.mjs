import { createApp, h, reactive } from 'vue'
import { createPinia } from 'pinia'
import { i18n } from '../../src/i18n'
import MessageItem from '../../src/components/chat/MessageItem.vue'
import MarkdownRenderer from '../../src/components/markdown/MarkdownRenderer.vue'
import { projectTranscriptBlocks } from '../../src/pages/chat/transcriptProjection'
import '../../src/style.css'
import '@fontsource/ibm-plex-sans/400.css'
import '@fontsource/ibm-plex-mono/400.css'

i18n.global.locale.value = 'zh-CN'
document.documentElement.classList.add('dark')
const expanded = reactive({})
const markdown =
  '已完成：每隔 1 秒依次输出 **1、2、3**，总耗时约 **3.28 秒**，退出码 **0**。\n\n```text\n1\n2\n3\n```\n\n命令输出可以直接在 part 中查看。'
const messages = [
  {
    info: { id: 'user', role: 'user' },
    parts: [{ id: 'u1', agenaKind: 'text', agenaContent: { text: '再来一次，一秒输出一个' } }],
  },
  {
    info: { id: 'reply', role: 'assistant' },
    parts: [
      {
        id: '1',
        agenaKind: 'tool_call',
        partState: 'completed',
        agenaContent: { name: 'shell.exec' },
        agenaPresentation: {
          title: 'Execute command',
          summary: 'sleep 1; printf 1; sleep 1; printf 2; sleep 1; printf 3',
          blocks: [{ type: 'text', text: '1\n2\n3' }],
        },
      },
      { id: '2', agenaKind: 'text', partState: 'completed', agenaContent: { text: markdown } },
    ],
  },
]
const richMarkdown =
  '# 标题\n\n第一段正文。\n\n第二段正文。\n\n## 次级标题\n\n- 第一个选项\n- 第二个选项\n\n> 引用说明\n>\n> 引用的第二段。\n\n| 名称 | 状态 |\n| --- | --- |\n| A | 完成 |\n| B | 运行 |\n\n```sh\necho first\n\necho third\n```\n\n---\n\n最后一段正文。'
createApp({
  setup() {
    return () =>
      h('main', { class: 'mx-auto max-w-4xl p-3' }, [
        h(
          'div',
          { 'data-transcript-root': 'true', 'data-density-example': 'exchange' },
          projectTranscriptBlocks(messages).map((block) =>
            h(MessageItem, {
              key: block.key,
              message: block.message,
              displayParts: block.displayParts,
              showTimestamps: true,
              formatTime: () => '15:29',
              copiedMessageId: '',
              revertBusyMessageId: '',
              isStreaming: false,
              collapseSignal: 0,
              activityPageSize: 5,
              isCompactTouch: false,
              isPartExpanded: (part) => expanded[part.key] ?? part.defaultExpanded,
              onPartToggle: (part, value) => {
                expanded[part.key] = value
              },
            }),
          ),
        ),
        h('div', { 'data-transcript-root': 'true', 'data-density-example': 'markdown', class: 'mt-3 border-t pt-2' }, [
          h(MarkdownRenderer, { content: richMarkdown }),
        ]),
      ])
  },
})
  .use(createPinia())
  .use(i18n)
  .mount('#app')
