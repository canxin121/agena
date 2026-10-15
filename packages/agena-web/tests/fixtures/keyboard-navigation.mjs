// Real UI components and keyboard routing with in-memory service actions.
// This fixture never creates a server session, runs a command, or changes settings.
import { createApp, h, onBeforeUnmount, onMounted, reactive } from 'vue'
import { createPinia } from 'pinia'
import { i18n } from '../../src/i18n'
import { router } from '../../src/router'
import { installImeCompositionGuard } from '../../src/lib/imeKeyboard'
import { installKeyboardShortcuts } from '../../src/app/runtime/installKeyboardShortcuts'
import { useUiStore } from '../../src/stores/ui'
import { useSettingsStore } from '../../src/stores/settings'
import { useChatStore } from '../../src/stores/chat'
import HelpDialog from '../../src/components/HelpDialog.vue'
import FormDialog from '../../src/components/ui/FormDialog.vue'
import OptionMenu from '../../src/components/ui/OptionMenu.vue'
import OptionPicker from '../../src/components/ui/OptionPicker.vue'
import InlineSearchAdd from '../../src/components/ui/InlineSearchAdd.vue'
import IconButton from '../../src/components/ui/IconButton.vue'
import SidebarIconButton from '../../src/components/ui/SidebarIconButton.vue'
import HorizontalSplitPane from '../../src/components/ui/HorizontalSplitPane.vue'
import WorkspaceEditorGroupPane from '../../src/layout/WorkspaceEditorGroupPane.vue'
import '../../src/style.css'
import '@fontsource/ibm-plex-sans/400.css'
import '@fontsource/ibm-plex-mono/400.css'

i18n.global.locale.value = 'zh-CN'
document.documentElement.classList.add('dark')
installImeCompositionGuard()

createApp({
  setup() {
    const ui = useUiStore()
    const settings = useSettingsStore()
    const chat = useChatStore()
    settings.data = { chatTranscriptVim: true }
    settings.save = async (patch) => Object.assign(settings.data, patch)
    const state = reactive({
      menu: false,
      query: '',
      form: false,
      selected: '',
      adds: [],
      choice: '',
      created: 0,
      rightWidth: 220,
      showRight: true,
    })
    chat.createSession = async () => {
      state.created++
      return { id: 'fixture-session' }
    }
    ui.closeAllWorkspaceWindows()
    const first = ui.openWorkspaceWindow('files', { title: 'Files one', reuseExisting: false })
    const second = ui.openWorkspaceWindow('preview', { title: 'Preview two', reuseExisting: false })
    ui.selectWorkspaceWindow(first)
    window.keyboardFixture = { state, ui, settings, first, second, router }
    let dispose
    onMounted(() => {
      dispose = installKeyboardShortcuts()
    })
    onBeforeUnmount(() => dispose?.())
    const options = [
      { value: 'disabled', label: 'Disabled option', disabled: true },
      ...Array.from({ length: 30 }, (_, index) => ({ value: `option-${index}`, label: `Option ${index}` })),
    ]
    return () =>
      h('main', { class: 'h-full overflow-auto p-4 space-y-4' }, [
        h('nav', { 'data-keyboard-region': '', tabindex: -1 }, [
          h(
            'button',
            {
              id: 'help-trigger',
              type: 'button',
              onClick: () => {
                ui.isHelpDialogOpen = true
              },
            },
            'Open help',
          ),
          h(IconButton, { tooltip: 'Named icon' }, () => '+'),
          h(SidebarIconButton, { title: 'Named sidebar icon' }, () => '+'),
          h(
            'button',
            {
              type: 'button',
              onClick: () => {
                state.form = true
              },
            },
            'Open form',
          ),
        ]),
        h('aside', { 'data-keyboard-region': '', 'data-keyboard-sidebar': '', tabindex: -1 }, [
          h('button', { id: 'sidebar-control', type: 'button' }, 'Sidebar control'),
          h(OptionPicker, {
            title: 'Fixture picker',
            modelValue: state.choice,
            options,
            includeEmpty: false,
            'onUpdate:modelValue': (value) => {
              state.choice = value
            },
          }),
          h(InlineSearchAdd, {
            options,
            allowCustom: false,
            panelTitle: 'Inline options',
            onAdd: (value) => state.adds.push(value),
          }),
        ]),
        h('section', { 'data-keyboard-region': '', tabindex: -1 }, [
          h('div', { class: 'xterm' }, [h('textarea', { id: 'terminal-input', 'aria-label': 'Terminal input' })]),
          h('div', { 'data-workspace-pane-window': first }, [
            h('textarea', { id: 'composer-first', 'data-chat-input': 'true', 'aria-label': 'First composer' }),
          ]),
          h('div', { 'data-workspace-pane-window': second }, [
            h('textarea', { id: 'composer-second', 'data-chat-input': 'true', 'aria-label': 'Second composer' }),
          ]),
        ]),
        ui.workspaceGroups[0]
          ? h('div', { style: 'height:160px' }, [h(WorkspaceEditorGroupPane, { groupId: ui.workspaceGroups[0].id })])
          : null,
        h('div', { class: 'relative' }, [
          h(
            'button',
            {
              id: 'menu-trigger',
              type: 'button',
              onClick: () => {
                state.menu = true
              },
            },
            'Open options',
          ),
          h(OptionMenu, {
            open: state.menu,
            query: state.query,
            title: 'Fixture menu',
            searchable: true,
            isCompactTouch: ui.isCompactTouch,
            groups: [
              {
                id: 'fixture',
                items: options.map((option) => ({ id: option.value, label: option.label, disabled: option.disabled })),
              },
            ],
            'onUpdate:open': (value) => {
              state.menu = value
            },
            'onUpdate:query': (value) => {
              state.query = value
            },
            onSelect: (item) => {
              state.selected = item.id
            },
          }),
        ]),
        h('div', { style: 'height:80px' }, [
          h(
            HorizontalSplitPane,
            {
              modelValue: state.rightWidth,
              minWidth: 120,
              maxWidth: 400,
              showRightPane: state.showRight,
              'onUpdate:modelValue': (value) => {
                state.rightWidth = value
              },
            },
            {
              left: () => h('button', { type: 'button' }, 'Left pane'),
              right: () => h('button', { id: 'hidden-pane-control', type: 'button' }, 'Right pane'),
            },
          ),
        ]),
        h(
          FormDialog,
          {
            open: state.form,
            title: 'Fixture form',
            'onUpdate:open': (value) => {
              state.form = value
            },
          },
          () => h('input', { 'aria-label': 'Form input' }),
        ),
        h(HelpDialog),
      ])
  },
})
  .use(createPinia())
  .use(i18n)
  .use(router)
  .mount('#app')
