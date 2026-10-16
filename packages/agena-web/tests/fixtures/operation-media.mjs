import { createApp, h, reactive } from 'vue'
import { createPinia } from 'pinia'
import { i18n } from '../../src/i18n'
import OperationPart from '../../src/components/chat/AgenaOperationPart.vue'
import OperationBlock from '../../src/components/chat/AgenaOperationBlock.vue'
import ImageViewerDialog from '../../src/components/ImageViewerDialog.vue'
import { useChatStore } from '../../src/stores/chat'
import { useDirectoryStore } from '../../src/stores/directory'
import { useUiStore } from '../../src/stores/ui'
import '../../src/style.css'

const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360"><rect width="640" height="360" fill="#f5f2ed"/><rect x="32" y="32" width="576" height="64" rx="12" fill="#164e63"/><rect x="32" y="128" width="272" height="200" rx="12" fill="#5eead4"/><rect x="336" y="128" width="272" height="200" rx="12" fill="#fda4af"/></svg>'
const image = {
  type: 'media',
  id: 'media',
  artifact: { uri: `data:image/svg+xml;base64,${btoa(svg)}`, mime: 'image/svg+xml', name: 'generated-dashboard.svg' },
}
function part(id, block) {
  return reactive({
    key: `part:${id}`,
    id: String(id),
    kind: 'operation',
    status: 'completed',
    role: 'assistant',
    title: 'Image generation',
    summary: '1 image',
    copyText: '',
    toggleable: true,
    defaultExpanded: true,
    source: {
      id: String(id),
      agenaKind: 'tool_call',
      partState: 'completed',
      revision: 1,
      agenaSections: [{ section: 'presentation', revision: 1 }],
      agenaContent: { name: 'image_generation', state: 'completed', resources: [] },
      agenaPresentation: { title: 'Image generation', summary: '1 image', blocks: block ? [block] : [] },
    },
  })
}
const inline = part(1, image)
const local = part(2, {
  ...image,
  artifact: { ...image.artifact, uri: 'file:///workspace/session-781/UI draft.svg', name: 'UI draft.svg' },
})
const running = part(3)
running.status = running.source.partState = running.source.agenaContent.state = 'in_progress'
const expanded = reactive({ inline: true, local: true, running: true })
const nativeFetch = window.fetch.bind(window)
window.fetch = async (url, options) => (String(url).startsWith('/api/') ? Response.json({}) : nativeFetch(url, options))

createApp({
  setup() {
    const chat = useChatStore()
    const directory = useDirectoryStore()
    const ui = useUiStore()
    directory.currentDirectory = '/workspace/another-session'
    chat.getSessionDirectory = (id) => (id === '781' ? '/workspace/session-781' : null)
    window.mediaFixture = {
      svg,
      inline,
      local,
      running,
      ui,
      expanded,
      replaceImage(artifact) {
        inline.source.agenaPresentation.blocks = [{ type: 'media', id: 'media', artifact }]
      },
      complete() {
        running.status = running.source.partState = running.source.agenaContent.state = 'completed'
        running.source.agenaPresentation.blocks = [image]
        running.source.revision++
      },
    }
    return () =>
      h('main', { class: 'mx-auto max-w-4xl p-6 space-y-6' }, [
        ...Object.entries({ inline, local, running }).map(([key, value]) =>
          h('section', { 'data-fixture': key }, [
            h('h2', { class: 'mb-2 font-semibold' }, key),
            h(OperationPart, {
              part: value,
              expanded: expanded[key],
              collapseSignal: 0,
              sessionId: '781',
              onToggle: () => {
                expanded[key] = !expanded[key]
              },
            }),
          ]),
        ),
        h('section', { 'data-fixture': 'video' }, [
          h(OperationBlock, {
            block: {
              type: 'media',
              artifact: { uri: 'https://example.test/clip.webm', mime: 'video/webm', name: 'clip.webm' },
            },
            sessionId: '781',
          }),
        ]),
        h('section', { 'data-fixture': 'audio' }, [
          h(OperationBlock, {
            block: {
              type: 'media',
              artifact: { uri: 'https://example.test/sound.mp3', mime: 'audio/mpeg', name: 'sound.mp3' },
            },
            sessionId: '781',
          }),
        ]),
        h(ImageViewerDialog),
      ])
  },
})
  .use(createPinia())
  .use(i18n)
  .mount('#app')
