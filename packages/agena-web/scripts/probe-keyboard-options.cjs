const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { pathToFileURL } = require('node:url')

const web = path.resolve(__dirname, '..')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

const fixture = `
import { createApp, h, ref } from 'vue'
import { createPinia } from 'pinia'
import ImageViewerDialog from '@/components/ImageViewerDialog.vue'
import ChoiceDialog from '@/components/ui/ChoiceDialog.vue'
import DirectoryPathDialog from '@/components/ui/DirectoryPathDialog.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import GroupedPicker from '@/components/ui/GroupedPicker.vue'
import NameInputPrompt from '@/components/ui/NameInputPrompt.vue'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SegmentedButton from '@/components/ui/SegmentedButton.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import AttachmentsPanel from '@/components/chat/AttachmentsPanel.vue'
import SessionChip from '@/components/chat/SessionChip.vue'
import ProviderModelCatalogPicker from '@/components/settings/ProviderModelCatalogPicker.vue'
import SidebarSessionActionMenu from '@/layout/chatSidebar/components/SidebarSessionActionMenu.vue'
import AppDialogHost from '@/components/AppDialogHost.vue'
import GitRemoteTargetDialogs from '@/pages/git/components/GitRemoteTargetDialogs.vue'
import BrowserViewport from '@/features/browser/BrowserViewport.vue'
import WorkspacePreviewPageSidebar from '@/features/workspacePreview/components/WorkspacePreviewPageSidebar.vue'
import { useDirectoryStore } from '@/stores/directory'
import { i18n } from '@/i18n'
import { confirmAction } from '@/lib/appConfirm'
import { promptForText } from '@/lib/appTextPrompt'
import '@/style.css'

const options = [
  { value: 'alpha', label: 'Alpha' },
  { value: 'blocked', label: 'Blocked', disabled: true },
  { value: 'omega', label: 'Omega' },
]
const collapsedGroups = [
  { id: 'one', title: 'First group', defaultCollapsed: true, items: [{ id: 'leaf', label: 'First leaf' }] },
  { id: 'two', title: 'Second group', defaultCollapsed: true, items: [{ id: 'last', label: 'Last leaf' }] },
]
const actions = [
  { id: 'keep', label: 'Keep session' },
  { id: 'archive', label: 'Archive session', confirmTitle: 'Confirm archive', confirmDescription: 'Archive this session?' },
]
createApp({ setup() {
  const directoryStore = useDirectoryStore()
  const output = ref(''), visible = ref(''), directory = ref(''), tab = ref('Source'), sessionChoice = ref('')
  const actionAnchor = ref(null), pagerAnchor = ref(null), actionQuery = ref(''), pagerQuery = ref('')
  const attachmentsAnchor = ref(null)
  const name = ref('Existing name')
  const previewVisible = ref(false)
  const queueResults = ref('')
  const browserDialog = ref(null)
  const branch = ref(''), remote = ref('origin')
  const branches = Array.from({length: 18}, (_, index) => 'feature/' + String(index + 1).padStart(2, '0'))
  const imageUrl = 'data:image/gif;base64,R0lGODlhAQABAAAAACwAAAAAAQABAAA='
  const files = ref([
    {id: 'first-image', filename: 'first.gif', url: imageUrl, size: 12, mime: 'image/gif', delivery: 'reference'},
    {id: 'next-image', filename: 'next.gif', url: imageUrl, size: 12, mime: 'image/gif', delivery: 'reference'},
    {id: 'file', filename: 'notes.txt', size: 12, mime: 'text/plain', delivery: 'reference'},
  ])
  const paged = ref(['One', 'Two', 'Three', 'Four'])
  const setOpen = kind => open => { visible.value = open ? kind : '' }
  const choose = value => { output.value = value }
  const button = (label, callback, attrs = {}) => h('button', {type: 'button', onClick: callback, ...attrs}, label)
  const trigger = (kind, label, attrs) => button(label, () => setOpen(kind)(visible.value !== kind), attrs)
  const picker = (title, extra = {}) => h(OptionPicker, { title, options, modelValue: output.value,
    includeEmpty: false, 'onUpdate:modelValue': choose, ...extra })
  async function openQueue() {
    const results = await Promise.all([
      confirmAction({ title: 'Remove saved item', confirmText: 'Remove' }),
      promptForText({ title: 'Name saved item', initialValue: 'Draft', confirmText: 'Save' }),
      confirmAction({ title: 'Keep saved item', variant: 'default', confirmText: 'Keep' }),
    ])
    queueResults.value = JSON.stringify(results)
  }

  return () => h('main', { class: 'grid w-[32rem] gap-2 p-10' }, [
    picker('Desktop picker'),
    picker('Mobile picker', {isCompactTouch: true}),
    h(GroupedPicker, {title: 'Grouped picker', label: 'Groups', groups: collapsedGroups, page: 2, pageCount: 4, onSelect: choose}),
    trigger('form', 'Open parent form'),
    h(FormDialog, {open: visible.value === 'form', title: 'Parent form', description: 'Contains a picker.',
      'onUpdate:open': setOpen('form')}, {default: () => [
        picker('Nested picker'),
        picker('Nested mobile picker', {isCompactTouch: true}),
      ]}),
    trigger('name', 'Open name editor'),
    h(NameInputPrompt, {open: visible.value === 'name', title: 'Edit name', modelValue: name.value,
      'onUpdate:modelValue': value => { name.value = value }, 'onUpdate:open': setOpen('name'),
      onSubmit: () => { choose(name.value); setOpen('name')(false) }}),
    button('Open queued dialogs', openQueue),
    h(AppDialogHost),
    h('output', {id: 'queue-results'}, queueResults.value),
    trigger('git', 'Open Git push dialog'),
    h(GitRemoteTargetDialogs, {pushToOpen: visible.value === 'git', 'onUpdate:pushToOpen': setOpen('git'),
      pullFromOpen: false, fetchFromOpen: false, stashDialogOpen: false, stashMessage: '',
      stashIncludeUntracked: false, stashKeepIndex: false, stashStaged: false, remoteBranchLoading: false,
      remoteInfo: {remotes: [{name: 'origin'}]}, targetRemote: remote.value, targetRef: '', targetSetUpstream: false,
      targetBranch: branch.value, filteredRemoteBranchOptions: branches.filter(value => value.includes(branch.value)),
      'onUpdate:targetBranch': value => { branch.value = value },
      'onUpdate:targetRemote': value => { remote.value = value },
      pushToTarget: () => choose(branch.value), pullFromTarget: () => {}, fetchFromTarget: () => {}, stashPush: () => {},
    }),
    trigger('choice', 'Open choice dialog'),
    h(ChoiceDialog, {open: visible.value === 'choice', title: 'Which outcome?', description: 'Choose an outcome.',
      options: [{id: 'first', label: 'First outcome'}, {id: 'second', label: 'Second outcome'}],
      cancelText: 'Cancel', onSelect: choose, 'onUpdate:open': setOpen('choice')}),
    trigger('actions', 'Open session actions', {ref: actionAnchor}),
    visible.value === 'actions' && h(SidebarSessionActionMenu, {open: true, anchorEl: actionAnchor.value,
      query: actionQuery.value, items: actions, 'onUpdate:query': value => { actionQuery.value = value },
      onSelect: item => choose(item.id), onClose: () => setOpen('actions')(false)}),
    trigger('pager', 'Open paged menu', {ref: pagerAnchor}),
    h(OptionMenu, {open: visible.value === 'pager', title: 'Paged menu', desktopAnchorEl: pagerAnchor.value,
      groups: [{id: 'pages', items: paged.value.map(label => ({id: label, label}))}],
      query: pagerQuery.value, paginated: true, pageSize: 2, paginationMode: 'item', closeOnSelect: false,
      'onUpdate:open': setOpen('pager'), 'onUpdate:query': value => { pagerQuery.value = value },
      onSelect: item => { choose(item.id); paged.value = paged.value.filter(label => label !== item.id) }}),
    trigger('directory', 'Open directory picker'),
    h(DirectoryPathDialog, {open: visible.value === 'directory', title: 'Choose directory',
      description: 'Browse fixture directories.', basePath: '/project', path: directory.value,
      confirmLabel: 'Use directory', allowCreateDirectory: false,
      'onUpdate:open': setOpen('directory'), 'onUpdate:path': value => { directory.value = value },
      onConfirm: () => { choose(directory.value); setOpen('directory')(false) }}),
    trigger('catalog', 'Open model catalog'),
    visible.value === 'catalog' && h(ProviderModelCatalogPicker, {initialQuery: '',
      onClose: () => setOpen('catalog')(false),
      onSelect: model => { choose(model.model_id); setOpen('catalog')(false) }}),
    trigger('attachments', 'Open attachments', {ref: attachmentsAnchor}),
    h(AttachmentsPanel, {open: visible.value === 'attachments', title: 'Fixture attachments',
      desktopAnchorEl: attachmentsAnchor.value, attachedFiles: files.value, formatBytes: String,
      'onUpdate:open': setOpen('attachments'), onRemove: () => { files.value = [] }}),
    h(ImageViewerDialog),
    h(SessionChip, {open: visible.value === 'session', title: 'Session list', label: 'Sessions',
      'onUpdate:open': setOpen('session')}, {
      default: () => sessionChoice.value ? h('p', sessionChoice.value) :
        ['First session', 'Second session'].map(label =>
          button(label, () => { sessionChoice.value = label }, {'data-keyboard-option': ''})),
    }),
    h(SegmentedControl, {'aria-label': 'Viewer mode'}, {
      default: () => ['Source', 'Preview'].map(label =>
        h(SegmentedButton, {active: tab.value === label, onClick: () => { tab.value = label }}, {default: () => label})),
    }),
    button('Outside control', () => {}),
    h(BrowserViewport, {interactive: true,
      frame: {kind: 'frame', jpeg: '', origin: '', width: 640, height: 480, dialog: browserDialog.value},
      onInput: event => {
        if (event.kind === 'key' && event.down && event.key === 'F2') {
          browserDialog.value = {kind: 'prompt', message: 'Enter a viewport name.'}
        } else if (event.kind === 'dialog') {
          choose(event.accept ? event.text : 'dismissed')
          browserDialog.value = null
        }
      }}),
    button('Show preview sidebar', () => {
      directoryStore.setDirectory('/project')
      previewVisible.value = true
    }),
    previewVisible.value ? h('div', { style: 'height: 24rem' }, [h(WorkspacePreviewPageSidebar)]) : null,
    h('output', {id: 'selection'}, output.value),
  ])
} }).use(createPinia()).use(i18n).mount('#app')
`

async function main() {
  const { createServer } = await import(pathToFileURL(require.resolve('vite')).href)
  const temporary = await fs.mkdtemp(path.join(web, '.keyboard-options-probe-'))
  let server, browser
  try {
    await fs.writeFile(path.join(temporary, 'entry.ts'), fixture)
    await fs.writeFile(
      path.join(temporary, 'index.html'),
      '<!doctype html><meta charset="utf-8"><title>Keyboard options</title><div id="app"></div><script type="module" src="./entry.ts"></script>',
    )
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'error',
      server: { host: '127.0.0.1', port: 0, proxy: {} },
    })
    await server.listen()
    const origin = 'http://127.0.0.1:' + server.httpServer.address().port
    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      headless: true,
      args: ['--no-sandbox'],
    })
    const page = await browser.newPage({ viewport: { width: 1100, height: 900 }, serviceWorkers: 'block' })
    page.setDefaultTimeout(8000)
    const errors = []
    let renamedPreview = ''
    const previewApi = '/api/v1/workbench/preview/sessions'
    const previewSessions = ['preview-alpha', 'preview-beta'].map((id) => ({
      id,
      state: 'stopped',
      directory: '/project',
      runDirectory: '/project',
      targetUrl: 'http://127.0.0.1:8000',
      command: 'fixture',
      args: [],
      logsPath: '.agena/preview.log',
    }))
    page.on('pageerror', (error) => errors.push(error.message))
    await page.route('**/*', (route) => {
      const url = new URL(route.request().url())
      if (url.origin !== origin) return route.abort()
      if (url.pathname === '/api/v1/workbench/fs/list') {
        const directory = url.searchParams.get('path') || '/project'
        const name = directory === '/project' ? 'child' : 'grandchild'
        const entries = [{ name, path: directory + '/' + name, isDirectory: true, isFile: false }]
        return route.fulfill({ json: { path: directory, entries } })
      }
      if (url.pathname === '/api/v1/model-catalog') {
        const items = [{ model_id: 'model-one' }, { model_id: 'model-two' }]
        return route.fulfill({ json: { items, total: items.length } })
      }
      if (url.pathname === previewApi) return route.fulfill({ json: { sessions: previewSessions } })
      if (url.pathname === previewApi + '/events') {
        return route.fulfill({ contentType: 'text/event-stream', body: ': ready\n\n' })
      }
      if (url.pathname === previewApi + '/preview-beta/rename') {
        renamedPreview = route.request().postDataJSON().id
        previewSessions[1].id = renamedPreview
        return route.fulfill({ json: previewSessions[1] })
      }
      if (url.pathname.startsWith('/api/')) return route.abort()
      return route.continue()
    })
    await page.goto(origin + '/' + path.basename(temporary) + '/')
    const button = (name) => page.getByRole('button', { name, exact: typeof name === 'string' })
    const dialog = (name) => page.getByRole('dialog', { name, exact: true })
    const press = (key) => page.keyboard.press(key)
    const focused = (locator) => locator.and(page.locator(':focus')).waitFor({ state: 'attached' })
    const selection = async (value) => assert.equal(await page.locator('#selection').textContent(), value)

    const desktop = button(/^Desktop picker/)
    await desktop.press('Enter')
    const search = dialog('Desktop picker').getByRole('textbox')
    await focused(search)
    await search.fill('a')
    await press('Home')
    assert.equal(await search.evaluate((input) => input.selectionStart), 0)
    await search.dispatchEvent('keydown', { key: 'ArrowDown', isComposing: true })
    await search.dispatchEvent('keydown', { key: 'Enter', isComposing: true })
    await focused(search)
    await press('ArrowDown')
    await focused(button('Alpha'))
    await press('Shift+Tab')
    await focused(search)
    await press('Tab')
    await focused(button('Alpha'))
    await press('ArrowDown')
    await focused(button('Omega'))
    await press('Space')
    await focused(desktop)
    await selection('omega')
    await press('ArrowUp')
    await focused(button('Omega'))
    await press('ArrowUp')
    await focused(button('Alpha'))
    await button('Outside control').click()
    await dialog('Desktop picker').waitFor({ state: 'detached' })
    await focused(button('Outside control'))

    const mobile = button(/^Mobile picker/)
    await mobile.press('Enter')
    await focused(dialog('Mobile picker'))
    await press('ArrowDown')
    await focused(button('Alpha'))
    await press('End')
    await focused(button('Omega'))
    await press('Escape')
    await focused(mobile)

    const grouped = button('Grouped picker')
    await grouped.press('ArrowDown')
    await focused(button(/^First group/))
    await press('Enter')
    await press('ArrowDown')
    await focused(button('First leaf'))
    await press('ArrowDown')
    await focused(button(/^Second group/))
    await press('Space')
    await press('ArrowDown')
    await focused(button('Last leaf'))
    await press('Enter')
    await focused(grouped)
    await selection('last')
    await press('Enter')
    const groupedPanel = dialog('Grouped picker')
    await groupedPanel.getByRole('textbox', { name: 'Search options' }).fill('last leaf')
    await button(/^First group/).waitFor({ state: 'detached' })
    await button('Last leaf').waitFor()
    assert.equal(await groupedPanel.getByRole('textbox', { name: 'Current page' }).inputValue(), '2')
    await press('Escape')
    await focused(grouped)

    const parent = button('Open parent form')
    await parent.press('Enter')
    const nested = button(/^Nested picker/)
    await nested.press('ArrowDown')
    await focused(button('Alpha'))
    await press('Escape')
    await focused(nested)
    const nestedMobile = button(/^Nested mobile picker/)
    await nestedMobile.press('ArrowDown')
    const nestedMobilePanel = dialog('Nested mobile picker')
    await focused(nestedMobilePanel.getByRole('button', { name: 'Alpha' }))
    await press('End')
    await focused(nestedMobilePanel.getByRole('button', { name: 'Omega' }))
    await press('Escape')
    await focused(nestedMobile)
    await press('Escape')
    await focused(parent)

    const nameTrigger = button('Open name editor')
    await nameTrigger.press('Enter')
    const nameInput = page.getByRole('textbox', { name: 'Edit name' })
    await focused(nameInput)
    assert.equal(await nameInput.evaluate((input) => input.selectionEnd - input.selectionStart), 'Existing name'.length)
    await press('Escape')
    await focused(nameTrigger)

    const queueTrigger = button('Open queued dialogs')
    await queueTrigger.press('Enter')
    const remove = dialog('Remove saved item')
    const nameDialog = dialog('Name saved item')
    const keep = dialog('Keep saved item')
    await focused(remove.getByRole('button', { name: 'Cancel', exact: true }))
    assert.equal(await nameDialog.count(), 0)
    assert.equal(await keep.count(), 0)
    await press('Enter')
    const queuedInput = nameDialog.getByRole('textbox', { name: 'Name saved item' })
    await focused(queuedInput)
    assert.equal(await queuedInput.inputValue(), 'Draft')
    await queuedInput.dispatchEvent('keydown', { key: 'Escape', isComposing: true })
    await focused(queuedInput)
    await queuedInput.fill('Saved name')
    await press('Enter')
    await focused(keep.getByRole('button', { name: 'Keep', exact: true }))
    await press('Enter')
    await focused(queueTrigger)
    assert.equal(await page.locator('#queue-results').textContent(), '[false,"Saved name",true]')

    const gitTrigger = button('Open Git push dialog')
    await gitTrigger.press('Enter')
    const gitBranch = page.getByRole('combobox', { name: 'Branch', exact: true })
    await gitBranch.focus()
    const gitOptions = page.getByRole('listbox', { name: 'Branch' })
    await gitOptions.waitFor()
    await press('ArrowUp')
    const lastBranch = gitOptions.getByRole('option', { name: 'feature/18', selected: true })
    await lastBranch.waitFor()
    await page.waitForFunction(() => {
      const list = document.querySelector('[role="listbox"]')
      return list?.scrollTop > 0
    })
    await press('Enter')
    await focused(gitBranch)
    assert.equal(await gitBranch.inputValue(), 'feature/18')
    await gitOptions.waitFor({ state: 'detached' })
    await press('ArrowDown')
    await gitOptions.waitFor()
    await press('Escape')
    await gitOptions.waitFor({ state: 'detached' })
    await focused(gitBranch)
    await press('Escape')
    await focused(gitTrigger)

    const choiceTrigger = button('Open choice dialog')
    await choiceTrigger.press('Enter')
    await focused(button('First outcome'))
    await press('ArrowDown')
    await focused(button('Second outcome'))
    await press('Enter')
    await focused(choiceTrigger)
    await selection('second')

    const actionsTrigger = button('Open session actions')
    await actionsTrigger.press('Enter')
    await focused(page.getByRole('textbox', { name: 'Search actions' }))
    await press('ArrowUp')
    const archive = button('Archive session')
    await focused(archive)
    await press('Enter')
    const confirm = dialog('Confirm archive').getByRole('button', { name: 'Confirm', exact: true })
    await focused(confirm)
    await press('ArrowDown')
    await focused(dialog('Confirm archive').getByRole('button', { name: 'Cancel', exact: true }))
    await press('Escape')
    await focused(archive)
    await selection('second')
    await press('Enter')
    await focused(confirm)
    await press('Enter')
    await focused(actionsTrigger)
    await selection('archive')
    await press('Enter')
    await focused(page.getByRole('textbox', { name: 'Search actions' }))
    await press('Escape')
    await focused(actionsTrigger)

    const pagerTrigger = button('Open paged menu')
    await pagerTrigger.press('Enter')
    const pager = dialog('Paged menu')
    await focused(pager.getByRole('textbox').first())
    await press('ArrowDown')
    await press('End')
    await focused(button('Two'))
    await press('Enter')
    await focused(button('One'))
    await selection('Two')
    const pageInput = pager.getByRole('textbox').last()
    await pageInput.fill('2')
    await press('ArrowUp')
    await focused(pageInput)
    await press('Enter')
    await button('Four').waitFor()
    await selection('Two')
    await press('Escape')
    await focused(pagerTrigger)

    const directoryTrigger = button('Open directory picker')
    await directoryTrigger.press('Enter')
    const pathBrowser = dialog('Choose directory').locator('[data-path-picker-browser]')
    await focused(pathBrowser)
    await button(/^child/).waitFor()
    await press('ArrowDown')
    await focused(button(/^child/))
    await press('Enter')
    await focused(pathBrowser)
    await button(/^grandchild/).waitFor()
    await press('ArrowDown')
    await focused(button(/^grandchild/))
    await press('Escape')
    await focused(directoryTrigger)

    const catalogTrigger = button('Open model catalog')
    await catalogTrigger.press('Enter')
    const secondModel = button(/^model-two/)
    await secondModel.waitFor()
    await press('ArrowUp')
    await focused(secondModel)
    await press('Enter')
    await focused(catalogTrigger)
    await selection('model-two')

    const attachmentsTrigger = button('Open attachments')
    const attachments = dialog('Fixture attachments')
    await attachmentsTrigger.press('Enter')
    await focused(attachments.locator('[data-attachment-first]'))
    await press('ArrowDown')
    await focused(attachments.getByRole('button', { name: /from project/i }))
    const firstImage = attachments.getByRole('button', { name: /open.*first\.gif/i })
    await firstImage.press('Enter')
    const viewer = page.locator('[data-image-viewer-viewport]')
    await focused(viewer)
    await press('ArrowRight')
    await viewer.getByRole('img', { name: 'next.gif' }).waitFor()
    await press('Escape')
    await focused(firstImage)
    await press('End')
    await focused(attachments.getByRole('button', { name: /remove attachment/i }).last())
    await press('Enter')
    await focused(attachments.locator('[data-attachment-first]'))
    await press('Escape')
    await focused(attachmentsTrigger)

    const sessions = button('Session list')
    const sessionBody = page.getByRole('region', { name: 'Session list' })
    await sessions.press('Enter')
    await focused(sessionBody)
    await press('ArrowDown')
    await focused(button('First session'))
    await press('ArrowDown')
    await focused(button('Second session'))
    await press('Enter')
    await focused(sessionBody)
    await press('Escape')
    await focused(sessions)

    const source = page.getByRole('tab', { name: 'Source' })
    const preview = page.getByRole('tab', { name: 'Preview' })
    await source.press('ArrowRight')
    await focused(preview)
    assert.equal(await preview.getAttribute('aria-selected'), 'true')
    await press('Tab')
    await focused(button('Outside control'))

    const browserInput = page.getByRole('textbox', { name: 'Keyboard input', exact: true })
    await browserInput.press('F2')
    const pageDialog = dialog('Page dialog')
    const promptInput = pageDialog.getByRole('textbox', { name: 'Page dialog', exact: true })
    await focused(promptInput)
    await promptInput.fill('Viewport name')
    await press('ArrowDown')
    await focused(pageDialog.getByRole('button', { name: 'Cancel', exact: true }))
    await press('ArrowDown')
    await focused(pageDialog.getByRole('button', { name: 'OK', exact: true }))
    await press('Enter')
    await focused(browserInput)
    await selection('Viewport name')

    await button('Show preview sidebar').press('Enter')
    const previewSection = page.getByRole('group', { name: 'Current directory', exact: true })
    const previewRow = (name) => previewSection.getByRole('button', { name: new RegExp(`^${name}\\b`) })
    const secondPreview = previewRow('preview-beta')
    await previewRow('preview-alpha').press('ArrowDown')
    await focused(secondPreview)
    await press('Shift+F10')
    await focused(page.locator('[data-option-menu-item]').filter({ hasText: 'Start' }))
    await press('ArrowDown')
    await focused(page.locator('[data-option-menu-item]').filter({ hasText: 'Rename' }))
    await press('Enter')
    const renameInput = previewSection.getByRole('textbox', { name: 'Rename', exact: true })
    await focused(renameInput)
    assert.deepEqual(await renameInput.evaluate((input) => [input.selectionStart, input.selectionEnd]), [0, 12])
    await renameInput.fill('invalid name')
    await press('Enter')
    await previewSection.getByRole('alert').waitFor()
    assert.equal(renamedPreview, '')
    await renameInput.fill('renamed-preview')
    await press('Enter')
    await focused(previewRow('renamed-preview'))
    assert.equal(renamedPreview, 'renamed-preview')

    await press('F2')
    await focused(renameInput)
    await renameInput.dispatchEvent('keydown', { key: 'Escape', isComposing: true })
    await focused(renameInput)
    await press('Escape')
    await focused(previewRow('renamed-preview'))

    await button('Multi-select').press('Enter')
    await previewRow('preview-alpha').press('Enter')
    await page.getByRole('textbox', { name: 'Search preview sessions' }).fill('renamed-preview')
    await previewRow('preview-alpha').waitFor({ state: 'hidden' })
    assert.equal(await previewRow('renamed-preview').getAttribute('aria-pressed'), 'false')
    assert.equal(await button('Select all').isEnabled(), true)
    await button('Select all').press('Enter')
    assert.equal(await previewRow('renamed-preview').getAttribute('aria-pressed'), 'true')
    await focused(button('Invert selection'))
    await page.locator('[data-selection-toolbar]').getByRole('button', { name: 'Done' }).press('Enter')
    await focused(button('Multi-select'))

    assert.deepEqual(errors, [])
    console.log('Keyboard option navigation passed in Chromium.')
  } finally {
    await browser?.close()
    await server?.close()
    await fs.rm(temporary, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
