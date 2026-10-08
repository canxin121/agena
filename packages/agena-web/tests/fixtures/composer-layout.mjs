import { createApp, h, reactive } from 'vue'
import { createPinia } from 'pinia'
import { i18n } from '../../src/i18n'
import Composer from '../../src/components/chat/Composer.vue'
import '../../src/style.css'
import '@fontsource/ibm-plex-sans/400.css'
import '@fontsource/ibm-plex-mono/400.css'

i18n.global.locale.value = 'zh-CN'
document.documentElement.classList.add('dark')
const state = reactive({ draft: '', height: 128, headerHeight: 28, header: true, fullscreen: false })
window.composerLayoutFixture = state

createApp({
  setup() {
    return () =>
      h('main', { class: 'mx-auto max-w-4xl px-2 pt-10' }, [
        h(
          'div',
          {
            'data-composer-pane': 'true',
            class: 'flex min-h-0 flex-col px-2 py-1 sm:py-1.5',
            style: { height: `${state.fullscreen ? 360 : state.height}px` },
          },
          [
            h(
              Composer,
              {
                class: 'min-h-0 flex-1',
                draft: state.draft,
                fullscreen: state.fullscreen,
                attachedFiles: [],
                pendingAttachments: [],
                'onUpdate:draft': (value) => {
                  state.draft = value
                },
                onToggleFullscreen: () => {
                  state.fullscreen = !state.fullscreen
                },
              },
              {
                ...(state.header
                  ? {
                      status: () =>
                        h('span', { class: 'flex w-max items-center gap-1 whitespace-nowrap' }, [
                          h(
                            'button',
                            { type: 'button', class: 'px-1', style: { height: `${state.headerHeight}px` } },
                            'model/example | max | 0%',
                          ),
                        ]),
                    }
                  : {}),
                controls: () =>
                  h(
                    'div',
                    {
                      'data-composer-controls': 'true',
                      class: 'flex shrink-0 items-center justify-between border-t border-border/60 p-2',
                    },
                    [
                      h('button', { type: 'button', class: 'h-8 px-2' }, '附件'),
                      h('button', { type: 'button', class: 'h-8 px-2' }, '发送'),
                    ],
                  ),
              },
            ),
          ],
        ),
      ])
  },
})
  .use(createPinia())
  .use(i18n)
  .mount('#app')
