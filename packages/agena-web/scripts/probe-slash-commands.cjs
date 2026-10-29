// Use a surface exported by the real Rust plugin host, never a hand-written
// replacement catalog. Every browser request stays inside this fixture server.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function main() {
  assert(process.env.SLASH_COMMAND_SURFACE, 'Set SLASH_COMMAND_SURFACE to a JSON surface exported by the Rust host')
  const surface = JSON.parse(await fs.readFile(process.env.SLASH_COMMAND_SURFACE, 'utf8'))
  assert(Array.isArray(surface.catalog?.commands) && surface.catalog.commands.length > 0, 'Rust command catalog is empty')
  assert(surface.catalog.commands.some((command) =>
    command.slash === '/compact' && command.target?.kind === 'client' && command.target.action === 'compact'),
  'Rust surface must publish /compact as a client action')

  const { createServer } = await import('vite')
  const web = path.resolve(__dirname, '..')
  const repo = path.resolve(web, '../..')
  const temporary = await fs.mkdtemp(path.join(web, '.slash-commands-probe-'))
  const apiRequests = []
  const unexpectedRequests = []
  const externalRequests = []
  const errors = []
  let server, browser
  try {
    const entry = path.join(temporary, 'entry.ts')
    await fs.writeFile(entry, `
import { createApp, computed, h, ref } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import CommandPalette from '/@fs/${web}/src/components/chat/CommandPalette.vue'
import { useChatCommands } from '/@fs/${web}/src/pages/chat/useChatCommands.ts'
import { useChatSessionActions } from '/@fs/${web}/src/pages/chat/useChatSessionActions.ts'
import { compactSession } from '/@fs/${web}/src/stores/chat/api.ts'
import { i18n } from '/@fs/${web}/src/i18n/index.ts'
import '/@fs/${web}/src/style.css'
const app = createApp({ setup() {
  const draft = ref(''), sent = [], selected = [], toasts = []
  const unavailable = async () => { throw new Error('Unexpected session action in slash probe') }
  const sessionActions = useChatSessionActions({
    chat: {selectedSessionId:'7', selectedSession:{id:'7'}, messages:[], compactSession,
      renameSession:unavailable, forkSession:unavailable},
    toasts:{push:(kind,message)=>toasts.push({kind,message})},
    sessionTitle:computed(()=>'Slash command fixture'), showThinking:ref(false), copyToClipboard:unavailable,
  })
  const commands = useChatCommands({
    draft, composerRef:ref(null), composerPickerOpen:ref(null),
    onSend:async()=>{sent.push(draft.value)},
    onCommandSelected:async(command)=>{
      selected.push({kind:command.kind, action:command.kind==='builtin'?command.action:null})
      if (command.kind!=='builtin' || command.action!=='compact') throw new Error('Expected the catalog compact action')
      await sessionActions.handleCompactSession()
    },
  })
  window.__slashCommandsProbe = { snapshot:()=>({
    draft:draft.value, loading:commands.commandsLoading.value,
    names:commands.commands.value.map(command=>command.name), selected, sent, toasts,
  }) }
  return ()=>h('main', [
    h('div', {class:'composer-fixture'}, [
      h(CommandPalette, {
        open:commands.commandOpen.value, autoFocus:commands.commandFocusSearch.value,
        loading:commands.commandsLoading.value, query:commands.commandQuery.value,
        commands:commands.filteredCommands.value, activeIndex:commands.commandIndex.value,
        commandIcon:commands.commandIcon,
        'onUpdate:query':commands.setCommandQuery,
        'onUpdate:activeIndex':(index)=>{commands.commandIndex.value=index},
        onKeydown:commands.handleCommandPaletteKeydown, onSelect:commands.selectCommand,
        onDismiss:commands.closeCommandPalette,
      }),
      h('textarea', {'data-slash-composer':'', 'aria-label':'Message', value:draft.value,
        onInput:(event)=>{draft.value=event.target.value;commands.handleDraftInput()},
        onKeydown:commands.handleDraftKeydown}),
    ]),
  ])
} })
app.use(createPinia()).use(createRouter({history:createMemoryHistory(),routes:[{path:'/',component:{render:()=>null}}]})).use(i18n).mount('#app')
`)
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, fs: { allow: [repo, temporary] } },
      plugins: [{
        name: 'slash-commands-probe-isolation',
        configureServer(dev) {
          dev.middlewares.use(async (req, res, next) => {
            const url = new URL(req.url, 'http://fixture')
            const json = (value) => {
              res.setHeader('Content-Type', 'application/json')
              res.setHeader('Cache-Control', 'no-store')
              res.end(JSON.stringify(value))
            }
            if (/^\/(api|auth|health)(\/|$)/.test(url.pathname)) {
              apiRequests.push({ method: req.method, path: url.pathname })
              if (url.pathname === '/api/v1/plugins/surface' && req.method === 'GET') return json(surface)
              if (url.pathname === '/api/v1/sessions/7/compact' && req.method === 'POST') {
                return json({
                  session: {id:7, title:'Slash command fixture'},
                  parts:[], part_page:null,
                })
              }
              unexpectedRequests.push({ method: req.method, path: url.pathname })
              res.statusCode = 404
              return json({ error: 'Unavailable in the isolated slash-command fixture' })
            }
            if (url.pathname !== '/__slash-commands-probe') return next()
            const html = `<!doctype html><html><head><meta charset="utf-8"><title>Slash command regression</title></head><body><div id="app"></div><style>html,body,#app{height:auto!important;overflow:visible!important}main{max-width:700px;margin:360px auto 40px}.composer-fixture{position:relative}textarea{width:100%;height:100px;border:1px solid;padding:12px}</style><script type="module" src="/@fs/${entry}"></script></body></html>`
            try {
              res.setHeader('Content-Type', 'text/html')
              res.end(await dev.transformIndexHtml('/__slash-commands-probe', html))
            } catch (error) {
              next(error)
            }
          })
        },
      }],
    })
    await server.listen()
    const origin = `http://127.0.0.1:${server.httpServer.address().port}`
    browser = await chromium.launch({ executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome', headless: true })
    const context = await browser.newContext({ viewport: { width: 1100, height: 900 }, serviceWorkers: 'block' })
    const page = await context.newPage()
    page.on('pageerror', (error) => errors.push(error.message))
    await page.route('**/*', (route) => {
      if (new URL(route.request().url()).origin === origin) return route.continue()
      externalRequests.push(route.request().url())
      return route.abort()
    })
    await page.goto(origin + '/__slash-commands-probe', { waitUntil: 'domcontentloaded' })
    const composer = page.locator('[data-slash-composer]')
    await composer.waitFor({ timeout: 60000 })
    await composer.fill('/')
    await page.waitForFunction(() => {
      const snapshot = window.__slashCommandsProbe.snapshot()
      return !snapshot.loading && snapshot.names.includes('compact')
    })
    const options = page.getByRole('option')
    assert(await options.count() > 0, 'typing / must display actual catalog options')
    assert.equal(await options.filter({ hasText: '/compact' }).count(), 1, '/compact must appear in the palette')

    await composer.fill('/compact')
    await page.waitForFunction(() =>
      document.querySelector('[data-command-active="true"]')?.textContent.includes('/compact'))
    const compactResponse = page.waitForResponse((response) =>
      new URL(response.url()).pathname === '/api/v1/sessions/7/compact')
    await composer.press('Enter')
    assert.equal((await compactResponse).status(), 200)
    await page.waitForFunction(() =>
      window.__slashCommandsProbe.snapshot().toasts.some(toast=>toast.kind==='success'))
    const result = await page.evaluate(() => window.__slashCommandsProbe.snapshot())
    assert.deepEqual(result.selected, [{ kind: 'builtin', action: 'compact' }])
    assert.deepEqual(result.sent, [], '/compact must not be sent as a chat message')
    assert.equal(result.draft, '')
    assert.equal(apiRequests.filter((request) => request.path.endsWith('/compact')).length, 1)
    assert.deepEqual(unexpectedRequests, [], 'slash commands must not invoke plugins or message submission')
    assert.deepEqual(externalRequests, [], 'the probe must not contact a live user service')
    assert.deepEqual(errors, [])
    console.log(`Slash command browser probe passed: ${surface.catalog.commands.length} real catalog entries; / lists commands; /compact Enter invokes only compaction.`)
  } finally {
    await Promise.allSettled([browser?.close(), server?.close()])
    await fs.rm(temporary, { recursive: true, force: true })
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
