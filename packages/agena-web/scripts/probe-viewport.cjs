// Real FormDialog, OptionMenu, ConfirmPopover and AttachmentsPanel in Chromium.
// Uses an isolated localhost Vite fixture; never contacts an Agena service.
// Optional: PLAYWRIGHT_MODULE, CHROME_BIN, PROBE_REPORT (JSON), PROBE_SCREENSHOT (PNG).
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { pathToFileURL } = require('node:url')
const web = path.resolve(__dirname, '..')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || require.resolve('playwright', { paths: [web] }))

async function main() {
  const stage = (message) => console.error(`[viewport-probe ${new Date().toISOString()}] ${message}`)
  stage('loading existing Vite installation')
  const { createServer } = await import(pathToFileURL(require.resolve('vite', { paths: [web] })).href)
  const repo = path.resolve(web, '../..')
  const temporary = await fs.mkdtemp(path.join(web, '.viewport-probe-'))
  const reportPath = process.env.PROBE_REPORT
  const checks = [],
    geometry = [],
    errors = [],
    externalRequests = [],
    apiRequests = []
  let server, browser, page
  const check = (condition, message) => {
    assert.ok(condition, message)
    checks.push(message)
  }
  try {
    stage('writing isolated component entry')
    const entry = path.join(temporary, 'entry.ts')
    await fs.writeFile(
      entry,
      `
import { createApp, h, ref, nextTick } from 'vue'
import { createPinia } from 'pinia'
import FormDialog from '/@fs/${web}/src/components/ui/FormDialog.vue'
import OptionMenu from '/@fs/${web}/src/components/ui/OptionMenu.vue'
import ConfirmPopover from '/@fs/${web}/src/components/ui/ConfirmPopover.vue'
import AttachmentsPanel from '/@fs/${web}/src/components/chat/AttachmentsPanel.vue'
import { useUiStore } from '/@fs/${web}/src/stores/ui.ts'
import { i18n } from '/@fs/${web}/src/i18n/index.ts'
import '/@fs/${web}/src/style.css'

const app = createApp({ setup() {
  const ui = useUiStore()
  const current = ref('')
  const compact = ref(true)
  // App's useDeviceRuntime applies these signals together. In particular,
  // DialogHeader supplies isTouchPointer explicitly to its close IconButton.
  const applyDevice = value => {
    compact.value = value
    ui.setIsCompactLayout(value)
    ui.setIsMobileDevice(value)
    ui.setIsTouchPointer(value)
    ui.setIsCompactTouch(value)
  }
  applyDevice(true)
  const setOpen = (kind, open) => { if (open) current.value = kind; else if (current.value === kind) current.value = '' }
  const groups = [{ id:'viewport-options', items:Array.from({length:18}, (_, index) => ({id:String(index),label:'Option '+(index+1)})) }]
  const attachedFiles = Array.from({length:8}, (_, index) => ({id:String(index),filename:'viewport-'+(index+1)+'.txt',size:32,mime:'text/plain',state:'ready',delivery:'reference'}))
  window.__viewportProbe = {
    open: async kind => { current.value = kind; await nextTick() },
    close: async () => { current.value = ''; await nextTick() },
    compact: async value => { applyDevice(value); await nextTick() },
    state: () => current.value,
    visual: value => {
      const viewport = window.visualViewport
      for (const key of ['width','height','offsetLeft','offsetTop']) {
        if (value) Object.defineProperty(viewport, key, { configurable:true, get:()=>value[key] })
        else delete viewport[key]
      }
      viewport.dispatchEvent(new Event('resize'))
      viewport.dispatchEvent(new Event('scroll'))
    },
  }
  const trigger = kind => h('button', {id:'open-'+kind, type:'button', onClick:()=>setOpen(kind,true)}, 'Open '+kind)
  return () => h('main', [
    h('nav', [trigger('form'), trigger('menu'), trigger('attachments')]),
    h(FormDialog, {open:current.value==='form',title:'Viewport form',mobileFillViewport:true,'onUpdate:open':value=>setOpen('form',value)}, {
      default:()=>h('div', [h('input',{id:'form-input','aria-label':'Form input'}), ...Array.from({length:30},(_,index)=>h('p','Form row '+index))]),
    }),
    h(OptionMenu, {open:current.value==='menu',groups,title:'Viewport menu',isCompactTouch:compact.value,
      'onUpdate:open':value=>setOpen('menu',value),returnFocusTarget:()=>document.querySelector('#open-menu')}),
    h(AttachmentsPanel, {open:current.value==='attachments',attachedFiles,formatBytes:bytes=>String(bytes),
      title:'Viewport attachments',isCompactTouch:compact.value,'onUpdate:open':value=>setOpen('attachments',value)}),
    h(ConfirmPopover, {open:current.value==='confirm',title:'Viewport confirmation',
      description:compact.value?'Confirm the viewport fixture.':'','onUpdate:open':value=>setOpen('confirm',value)}, {
      default:()=>h('button',{id:'open-confirm',type:'button',style:compact.value?{}:{position:'fixed',top:'300px',left:'160px',height:'36px',width:'160px'}},'Open confirm'),
    }),
  ])
}})
app.use(createPinia()).use(i18n).mount('#app')
`,
    )
    stage('creating isolated localhost Vite server')
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, fs: { allow: [repo, temporary] } },
      plugins: [
        {
          name: 'viewport-probe-isolation',
          configureServer(dev) {
            dev.middlewares.use(async (req, res, next) => {
              const url = new URL(req.url, 'http://fixture')
              if (/^\/(api|auth|health)(\/|$)/.test(url.pathname)) {
                apiRequests.push({ method: req.method, path: url.pathname })
                res.statusCode = 404
                res.setHeader('Content-Type', 'application/json')
                return res.end(
                  JSON.stringify({ error: 'No application service is used by this isolated viewport fixture' }),
                )
              }
              if (url.pathname !== '/__viewport-probe') return next()
              try {
                const html =
                  '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Viewport regression</title></head><body><div id="app"></div><style>html,body,#app{height:100%;margin:0}main{padding:16px}nav{display:flex;flex-wrap:wrap;gap:8px}nav button,#open-confirm{border:1px solid #888;padding:8px}input{border:1px solid #888}p{margin:8px 0}:root{--oc-safe-area-top:0px;--oc-safe-area-bottom:0px;--oc-safe-area-left:0px;--oc-safe-area-right:0px}</style><script type="module" src="/@fs/' +
                  entry +
                  '"></script></body></html>'
                res.setHeader('Content-Type', 'text/html')
                res.end(await dev.transformIndexHtml('/__viewport-probe', html))
              } catch (error) {
                next(error)
              }
            })
          },
        },
      ],
    })
    stage('starting Vite listener')
    await server.listen()
    const origin = `http://127.0.0.1:${server.httpServer.address().port}`
    stage(`Vite listening at ${origin}; launching Chromium`)
    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      headless: true,
      args: ['--no-sandbox'],
    })
    const context = await browser.newContext({
      viewport: { width: 390, height: 760 },
      hasTouch: true,
      isMobile: true,
      serviceWorkers: 'block',
    })
    page = await context.newPage()
    page.setDefaultTimeout(10000)
    page.setDefaultNavigationTimeout(60000)
    page.on('pageerror', (error) => errors.push(error.message))
    await page.route('**/*', (route) => {
      if (new URL(route.request().url()).origin === origin) return route.continue()
      externalRequests.push(route.request().url())
      return route.abort()
    })
    stage('loading component page (60 second cold-start budget)')
    await page.goto(`${origin}/__viewport-probe`, { waitUntil: 'domcontentloaded' })
    await page.waitForFunction(() => Boolean(window.__viewportProbe), null, { timeout: 60000 })
    stage('Vue mounted; starting component assertions')

    const dialog = () => page.locator('[role="dialog"]:visible').last()
    async function bounded(label, viewport) {
      await dialog().waitFor()
      await page.waitForFunction((viewport) => {
        const elements = [...document.querySelectorAll('[role="dialog"]')].filter(
          (el) => el.getClientRects().length && getComputedStyle(el).visibility !== 'hidden',
        )
        const panel = elements.at(-1)
        if (!panel) return false
        const r = panel.getBoundingClientRect()
        const v = viewport || { left: 0, top: 0, width: window.innerWidth, height: window.innerHeight }
        return (
          r.width > 0 &&
          r.height > 0 &&
          r.left >= v.left - 2 &&
          r.top >= v.top - 2 &&
          r.right <= v.left + v.width + 2 &&
          r.bottom <= v.top + v.height + 2
        )
      }, viewport || null)
      const rect = await dialog().boundingBox()
      check(Boolean(rect), `${label}: real dialog has measurable bounds`)
      geometry.push({ label, ...rect, viewport: viewport || (await page.viewportSize()) })
      return rect
    }
    async function open(kind) {
      await page.locator(`#open-${kind}`).focus()
      if (kind === 'confirm') await page.keyboard.press('Enter')
      else await page.locator(`#open-${kind}`).click()
      await page.waitForFunction((kind) => window.__viewportProbe.state() === kind, kind)
      await dialog().waitFor()
    }
    async function close(kind) {
      const tooltipOpen = await page.evaluate(() =>
        [...document.querySelectorAll('[role="tooltip"]')].some((el) => el.getClientRects().length),
      )
      await page.keyboard.press('Escape')
      // A focused close button can open its Tooltip, which owns the first
      // Escape. The next Escape belongs to the containing dialog.
      if (tooltipOpen) {
        await page.waitForFunction(
          () => ![...document.querySelectorAll('[role="tooltip"]')].some((el) => el.getClientRects().length),
        )
        check(
          await page.evaluate((kind) => window.__viewportProbe.state() === kind, kind),
          `${kind}: first Escape dismisses the focused tooltip and retains its parent dialog`,
        )
        check(await dialog().isVisible(), `${kind}: dialog remains visible after dismissing only its tooltip`)
        await page.keyboard.press('Escape')
      }
      await page.waitForFunction(() => window.__viewportProbe.state() === '')
      await page.waitForFunction(
        () =>
          ![...document.querySelectorAll('[role="dialog"]')].some(
            (el) => el.getClientRects().length && getComputedStyle(el).visibility !== 'hidden',
          ),
      )
      await page.waitForFunction((kind) => document.activeElement?.id === `open-${kind}`, kind)
      check(
        await page.evaluate((kind) => document.activeElement?.id === `open-${kind}`, kind),
        `${kind}: Escape returns focus to trigger`,
      )
    }

    for (const kind of ['form', 'menu', 'attachments', 'confirm']) {
      stage(`${kind}: native viewport, close/reopen, focus and Escape`)
      await page.setViewportSize({ width: 390, height: 760 })
      await open(kind)
      const initial = await bounded(`${kind}: mobile open`)
      check(
        await dialog().evaluate((el) => el === document.activeElement || el.contains(document.activeElement)),
        `${kind}: focus enters dialog`,
      )
      await page.setViewportSize({ width: 360, height: 480 })
      const resized = await bounded(`${kind}: native mobile viewport resize`)
      check(
        Math.abs(initial.y - resized.y) > 2 ||
          Math.abs(initial.height - resized.height) > 2 ||
          Math.abs(initial.width - resized.width) > 2,
        `${kind}: layout responds to viewport resize`,
      )
      await close(kind)
      await page.setViewportSize({ width: 412, height: 680 })
      await open(kind)
      await bounded(`${kind}: resize while closed then reopen`)
      if (kind === 'menu') {
        await page.keyboard.press('End')
        await page.waitForFunction(() => document.activeElement?.textContent?.trim() === 'Option 18')
        check(
          await dialog().evaluate((el) => {
            const panel = el.getBoundingClientRect(),
              item = document.activeElement.getBoundingClientRect()
            return item.top >= panel.top && item.bottom <= panel.bottom
          }),
          'menu: End focuses the last option and scrolls it into the resized panel',
        )
      }
      await close(kind)
    }

    // VisualViewport offset/size events exercise the actual mounted components
    // in Chromium while emulating an on-screen keyboard's visible area.
    for (const kind of ['form', 'menu', 'attachments']) {
      stage(`${kind}: VisualViewport resize and offset events`)
      await page.setViewportSize({ width: 390, height: 760 })
      await open(kind)
      await bounded(`${kind}: before keyboard viewport`)
      const visible = { width: 360, height: 330, offsetLeft: 10, offsetTop: 90 }
      await page.evaluate((visible) => window.__viewportProbe.visual(visible), visible)
      await bounded(`${kind}: visualViewport resize and scroll`, { left: 10, top: 90, width: 360, height: 330 })
      await close(kind)
      await page.evaluate(() => window.__viewportProbe.visual(null))
    }

    // Keep the same anchor through close/reopen. At 480px there is enough
    // room for the compact card below, but less than the component's 140px
    // preferred space; this distinguishes its computed side from Radix's
    // collision fallback and catches a stale computed cache after reopening.
    stage('ConfirmPopover: cached anchor close/reopen and preferred side updates')
    await page.evaluate(() => window.__viewportProbe.compact(false))
    await page.setViewportSize({ width: 700, height: 720 })
    await open('confirm')
    await bounded('confirm: desktop initial anchor')
    check((await dialog().getAttribute('data-side')) === 'bottom', 'confirm: initial preferred side is bottom')
    await close('confirm')
    await page.setViewportSize({ width: 700, height: 480 })
    await page.evaluate(() => window.__viewportProbe.open('confirm'))
    await bounded('confirm: programmatic reopen after closed viewport resize')
    await page.waitForFunction(
      () => document.querySelector('[role="dialog"][data-state="open"]')?.getAttribute('data-side') === 'top',
    )
    check(
      (await dialog().getAttribute('data-side')) === 'top',
      'confirm: reopen recomputes top side without new anchor input',
    )
    await page.setViewportSize({ width: 700, height: 720 })
    await bounded('confirm: open viewport resize')
    await page.waitForFunction(
      () => document.querySelector('[role="dialog"][data-state="open"]')?.getAttribute('data-side') === 'bottom',
    )
    check((await dialog().getAttribute('data-side')) === 'bottom', 'confirm: viewport observer updates side while open')
    await close('confirm')

    check(errors.length === 0, 'no browser page errors')
    check(apiRequests.length === 0, 'no application API requests')
    check(externalRequests.length === 0, 'no non-localhost requests')
    const report = { passed: true, assertions: checks.length, checks, geometry, errors, apiRequests, externalRequests }
    if (reportPath) await fs.writeFile(reportPath, JSON.stringify(report, null, 2))
    stage(`passed ${checks.length} assertions`)
    console.log(JSON.stringify(report, null, 2))
  } catch (error) {
    stage(`failed: ${error.message}`)
    let diagnostics = null
    if (page) {
      if (process.env.PROBE_SCREENSHOT)
        await page.screenshot({ path: process.env.PROBE_SCREENSHOT, fullPage: true }).catch(() => {})
      diagnostics = await page
        .evaluate(() => ({
          state: window.__viewportProbe?.state(),
          active: {
            id: document.activeElement?.id,
            tag: document.activeElement?.tagName,
            label: document.activeElement?.getAttribute('aria-label'),
          },
          viewport: {
            width: window.innerWidth,
            height: window.innerHeight,
            visual: {
              width: window.visualViewport?.width,
              height: window.visualViewport?.height,
              left: window.visualViewport?.offsetLeft,
              top: window.visualViewport?.offsetTop,
            },
          },
          dialogs: [...document.querySelectorAll('[role="dialog"]')].map((el) => ({
            state: el.getAttribute('data-state'),
            rect: el.getBoundingClientRect().toJSON(),
            scrollHeight: el.scrollHeight,
            clientHeight: el.clientHeight,
            style: el.getAttribute('style'),
          })),
          tooltips: [...document.querySelectorAll('[role="tooltip"]')].map((el) => el.textContent),
        }))
        .catch(() => null)
      stage(`failure DOM: ${JSON.stringify(diagnostics)}`)
    }
    if (reportPath)
      await fs.writeFile(
        reportPath,
        JSON.stringify(
          { passed: false, error: error.stack, diagnostics, checks, geometry, errors, apiRequests, externalRequests },
          null,
          2,
        ),
      )
    throw error
  } finally {
    stage('closing browser/Vite and removing this probe temporary directory')
    await Promise.allSettled([browser?.close(), server?.close()])
    await fs.rm(temporary, { recursive: true, force: true })
    stage('cleanup complete')
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
