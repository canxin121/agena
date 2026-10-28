// Style regression for every controlled UI kind, real Vue components and host
// theme tokens. Isolated Vite page: no model, user project or backend is touched.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

function galleryDocument() {
  const nodes = []
  const add = (id, kind, props = {}, children, binding) => {
    nodes.push({ id, kind, props, ...(children ? { children } : {}), ...binding })
    return id
  }
  const metric = (id, label = '指标', value = '1,240') => add(id, 'metric', { label, value, detail: '明确的数据说明' })
  const controls = [
    add('select', 'select', { label: '选择条件', description: '选项与其他控件保持一致。' }, null, { bind: 'choice' }),
    add('input', 'input', { label: '搜索文字', placeholder: '输入关键词' }, null, { bind: 'query' }),
    add('number', 'number', { label: '数值', suffix: '%' }, null, { bind: 'amount' }),
    add('slider', 'slider', { label: '参数', description: '键盘和拖动均可调整。' }, null, { bind: 'amount' }),
    add('checkbox', 'checkbox', { label: '显示补充内容' }, null, { bind: 'enabled' }),
    add('switch', 'switch', { label: '启用模拟' }, null, { bind: 'enabled' }),
  ]
  const metrics = [
    metric('metric1', '累计数量'),
    metric('metric2', '当前比率', '18.5%'),
    metric('metric3', '长数值换行检查', '123456789012345678901234567890'),
  ]
  const compactNested = add('nestedMetrics', 'grid', { columns: 3 }, [
    metric('nestedA', 'A', 10),
    metric('nestedB', 'B', 20),
    metric('nestedC', 'C', 30),
  ])
  const nested = add('nested', 'grid', { columns: 2 }, [
    add('nestedCard', 'card', { title: '嵌套容器也要按实际宽度适配' }, [compactNested]),
    add('otherCard', 'card', { title: '单列不能被断点变成双列' }, [
      add('oneColumn', 'grid', { columns: 1 }, [
        add('oneA', 'text', { text: '第一行' }),
        add('oneB', 'text', { text: '第二行' }),
      ]),
    ]),
  ])
  const charts = ['bar', 'line', 'area', 'pie'].map((type) =>
    add(type + 'Chart', 'chart', {
      title: `${type} · 图表`,
      chart_type: type,
      labels: ['第一类', '第二类', '第三类'],
      x_label: '类别',
      y_label: '演示单位',
      series: [{ name: '样例数值', values: type === 'pie' ? [3, 5, 8] : [-3, 0, 8] }],
    }),
  )
  const roots = [
    add('hero', 'row', {}, [
      add('heading', 'heading', { text: '组件样式检查', size: 'lg' }),
      add('badge', 'badge', { text: '仅用于样式验收', tone: 'success' }),
    ]),
    add('tabs', 'tabs', { label: '视图切换' }, null, { bind: 'choice' }),
    add('metrics', 'grid', { columns: 3 }, metrics),
    add('controlCard', 'card', { title: '输入与选项', description: '紧凑尺寸不应缩小文字或隐藏焦点。' }, [
      add('controls', 'grid', { columns: 2 }, controls),
    ]),
    add('button', 'button', { label: '明确的操作按钮', tone: 'primary' }, null, { action: 'toggle' }),
    add('divider', 'divider'),
    nested,
    add('progress', 'progress', { label: '验证进度', value: 65, max: 100, detail: '这不是实际任务进度。' }),
    add('callout', 'callout', {
      title: '需要保留的提示',
      text: '边界用于分组，不用重复边框和过大的内边距制造层级。',
      tone: 'warning',
    }),
    add('details', 'details', { title: '按需展开的说明' }, [
      add('detailText', 'text', { text: '这里仍需要正常的段落行距。' }),
    ]),
    add('table', 'table', {
      title: '紧凑数据表',
      columns: [
        { key: 'label', label: '项目' },
        { key: 'value', label: '内容' },
      ],
      rows: [
        { label: '短文本', value: '清楚可读' },
        {
          label: '长中文',
          value: '这个字段用于检查很长的中文内容是否会在表格内部正常换行，而不是挤出整个回答的容器。',
        },
        { label: '连续字符', value: 'long_identifier_'.repeat(18) },
      ],
    }),
    add(
      'tasks',
      'tasks',
      {
        title: '任务',
        items: [
          { id: 'one', title: '短任务' },
          { id: 'two', title: '较长的任务名称也应完整显示且保留勾选区域', detail: '说明文字不用额外的卡片包裹。' },
        ],
      },
      null,
      { bind: 'selection' },
    ),
    add('timeline', 'timeline', {
      title: '观察记录',
      items: [
        { id: 'start', title: '开始', detail: '观察说明' },
        { id: 'done', title: '完成样式检查', detail: '不插入无意义的空白行。' },
      ],
    }),
    add(
      'chartGrid',
      'grid',
      { columns: 2 },
      charts.map((id, index) => add(`chartCard${index}`, 'card', {}, [id])),
    ),
    add('longText', 'text', {
      text: '没有空格的长文本也不应导致横向溢出：' + 'long_identifier_'.repeat(25),
      size: 'sm',
    }),
  ]
  add('root', 'stack', { gap: 'md' }, roots)
  assert.equal(new Set(nodes.map((node) => node.kind)).size, 24)
  return {
    version: 1,
    title: 'Inline UI · 密度与主题验收',
    blocks: [
      {
        type: 'panel',
        id: 'gallery',
        program: {
          root: 'root',
          nodes,
          state: {
            choice: {
              kind: 'choice',
              initial: 'one',
              options: [
                { value: 'one', label: '经营总览' },
                { value: 'two', label: '项目执行' },
                { value: 'three', label: '情景推演' },
              ],
            },
            amount: { kind: 'number', initial: 35, min: 0, max: 100, step: 1 },
            enabled: { kind: 'boolean', initial: false },
            query: { kind: 'text', initial: '', max_length: 128 },
            selection: { kind: 'selection', initial: [] },
          },
          actions: { toggle: [{ type: 'toggle', key: 'enabled' }] },
        },
      },
    ],
  }
}

async function main() {
  const { createServer } = await import('vite')
  const web = path.resolve(__dirname, '..'),
    repo = path.resolve(web, '../..')
  const temporary = await fs.mkdtemp(path.join(web, '.inline-ui-probe-'))
  const output = process.env.INLINE_UI_ARTIFACTS || path.join(repo, 'artifacts/inline-ui/density-after')
  await fs.mkdir(output, { recursive: true })
  const fixture = galleryDocument()
  const plain = JSON.parse(await fs.readFile(path.join(repo, 'examples/inline-ui/reply.json'), 'utf8'))
  const entry = path.join(temporary, 'entry.ts')
  await fs.writeFile(
    entry,
    `
import {createApp,h,ref} from 'vue'
import InlineReply from '/@fs/${web}/src/features/inlineUi/InlineReply.vue'
import {parseInlineDocument} from '/@fs/${web}/src/features/inlineUi/model.ts'
import Button from '/@fs/${web}/src/components/ui/Button.vue'
import Input from '/@fs/${web}/src/components/ui/Input.vue'
import {i18n,setAppLocale} from '/@fs/${web}/src/i18n/index.ts'
import '/@fs/${web}/src/style.css'
const fixture=parseInlineDocument(${JSON.stringify(fixture)}), plain=parseInlineDocument(${JSON.stringify(plain)})
const staticMode=ref(false)
window.__layoutProbe={setStatic:(value)=>{staticMode.value=value}}
setAppLocale('zh-CN')
createApp({setup:()=>()=>h('main',{class:'probe-page'},[
 h('aside',{'data-host-controls':'',class:'host-controls'},[h(Button,{size:'sm',variant:'outline'},()=> 'Agena 按钮'),h(Input,{placeholder:'Agena 输入框'})]),
 h('div',{'data-style-sentinel':'',class:'ui-card'},[h('button',{class:'ui-button'},'宿主内容不应受 Inline UI 样式影响')]),
 h(InlineReply,{document:staticMode.value?plain:fixture,scope:'layout-regression'})
])}).use(i18n).mount('#app')
`,
  )
  let server, browser
  const errors = [],
    measurements = []
  try {
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, fs: { allow: [repo, temporary] } },
      plugins: [
        {
          name: 'inline-style-fixture',
          configureServer(dev) {
            dev.middlewares.use(async (req, res, next) => {
              if (req.url?.startsWith('/api/')) {
                res.statusCode = 500
                res.end('No backend in style test')
                return
              }
              if (req.url !== '/__inline-style') return next()
              const html = `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="app"></div><style>html,body,#app{height:auto!important;min-height:100%;overflow:visible!important}body{margin:0;background:var(--color-background);color:var(--color-foreground)}.probe-page{width:calc(100% - 24px);max-width:920px;margin:16px auto}.host-controls{display:flex;gap:8px;margin-bottom:12px}.host-controls input{max-width:180px}[data-style-sentinel]{margin-bottom:12px;font-size:12px}</style><script type="module" src="/@fs/${entry}"></script></body></html>`
              try {
                res.setHeader('Content-Type', 'text/html')
                res.end(await dev.transformIndexHtml('/__inline-style', html))
              } catch (error) {
                next(error)
              }
            })
          },
        },
      ],
    })
    await server.listen()
    const origin = `http://127.0.0.1:${server.httpServer.address().port}`
    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      headless: true,
    })
    const context = await browser.newContext({
      viewport: { width: 1240, height: 1000 },
      permissions: ['clipboard-read', 'clipboard-write'],
    })
    const page = await context.newPage()
    page.on('pageerror', (error) => errors.push(error.message))
    await page.route('**/*', (route) => (route.request().url().startsWith(origin) ? route.continue() : route.abort()))
    await page.goto(origin + '/__inline-style', { waitUntil: 'networkidle' })
    await page.locator('[data-reply-chart]').nth(3).waitFor()
    async function measure(label) {
      await page.evaluate(() => document.fonts.ready)
      await page.waitForTimeout(70) // settle ResizeObserver and container-query layout
      const result = await page.evaluate(() => {
        const panel = document.querySelector('[data-interactive-panel]'),
          bounds = panel.getBoundingClientRect()
        const style = (id) => getComputedStyle(document.querySelector(`[data-ui-node="${id}"]`))
        const boxes = [...panel.querySelectorAll('[data-ui-node]')].filter(
          (node) => node.getBoundingClientRect().height > 0,
        )
        const violations = boxes
          .filter((node) => {
            const r = node.getBoundingClientRect()
            return r.left < bounds.left - 1 || r.right > bounds.right + 1
          })
          .map((node) => node.dataset.uiNode)
        const input = document.querySelector('[data-ui-node="input"] input')
        return {
          viewport: innerWidth,
          dark: document.documentElement.classList.contains('dark'),
          rtl: document.documentElement.dir === 'rtl',
          width: bounds.width,
          height: bounds.height,
          overflow: document.documentElement.scrollWidth > innerWidth,
          violations,
          gap: Number.parseFloat(style('root').gap),
          inset: Number.parseFloat(style('controlCard').paddingTop),
          tabs: document.querySelector('[data-ui-node="tabs"]').getBoundingClientRect().height,
          inputHeight: input.getBoundingClientRect().height,
          inputFont: Number.parseFloat(getComputedStyle(input).fontSize),
          hostFont: Number.parseFloat(getComputedStyle(document.querySelector('[data-host-controls] input')).fontSize),
          oneColumn: style('oneColumn').gridTemplateColumns.split(' ').length,
          nestedColumns: style('nestedMetrics').gridTemplateColumns.split(' ').length,
          metricsColumns: style('metrics').gridTemplateColumns.split(' ').length,
          sentinelPadding: getComputedStyle(document.querySelector('[data-style-sentinel]')).padding,
          sentinelBorder: getComputedStyle(document.querySelector('[data-style-sentinel]')).borderTopWidth,
          plots: [...panel.querySelectorAll('svg[role="img"]')].map((svg) => ({
            width: svg.getBoundingClientRect().width,
            height: svg.getBoundingClientRect().height,
            effectiveFont:
              Number.parseFloat(getComputedStyle(svg.querySelector('text') || svg).fontSize) * svg.getScreenCTM().a,
          })),
        }
      })
      measurements.push({ label, ...result })
      assert.equal(result.overflow, false, `${label}: page overflow`)
      assert.deepEqual(result.violations, [], `${label}: components leave reply bounds`)
      assert.equal(result.oneColumn, 1, `${label}: preserve explicit one-column grids`)
      assert.equal(result.sentinelPadding, '0px', `${label}: styles leak to host UI`)
      assert.equal(result.sentinelBorder, '0px', `${label}: styles leak to host UI`)
      assert(result.inputFont >= 13, `${label}: controls must remain readable`)
      assert(result.inputHeight >= 32, `${label}: input hit target`)
      for (const plot of result.plots) {
        assert(plot.height >= 150 && plot.height <= 261, `${label}: responsive plot height`)
        assert(plot.effectiveFont >= 12, `${label}: SVG labels must not shrink with mobile width`)
      }
      return result
    }
    for (const width of [1240, 800, 600, 375, 320]) {
      await page.setViewportSize({ width, height: 1000 })
      const metrics = await measure(`light-${width}`)
      assert(
        metrics.inset <= 11 && metrics.gap <= 9 && metrics.tabs <= 40,
        'default density matches compact host controls',
      )
      if (width === 1240) assert.equal(metrics.nestedColumns, 2, 'nested cards use their own actual width')
      if (width === 375) assert.equal(metrics.metricsColumns, 2, 'short metrics do not require three full-height rows')
      if ([1240, 375].includes(width))
        await page.screenshot({ path: path.join(output, `gallery-light-${width}.png`), fullPage: true })
    }
    await page.evaluate(() => document.documentElement.classList.add('dark'))
    await measure('dark-320')
    await page.screenshot({ path: path.join(output, 'gallery-dark-320.png'), fullPage: true })
    await page.evaluate(() => {
      document.documentElement.style.setProperty('--text-ui-label', '18px')
      document.documentElement.style.setProperty('--text-ui-header', '20px')
      document.documentElement.style.setProperty('--text-micro', '16px')
      document.documentElement.style.setProperty('--padding-scale', '1.2')
    })
    const larger = await measure('large-type-320')
    assert.equal(larger.inputFont, 18, 'honor host font settings')
    await page.evaluate(() => {
      document.documentElement.dir = 'rtl'
    })
    await measure('large-type-rtl-320')
    await page.evaluate(() => {
      document.documentElement.dir = ''
      document.documentElement.removeAttribute('style')
    })
    await page.setViewportSize({ width: 1240, height: 1000 })
    await page.locator('[data-ui-node="details"] summary').focus()
    await page.keyboard.press('Enter')
    assert.equal(await page.locator('[data-ui-node="details"]').getAttribute('open'), '')
    const chart = page.locator('[data-reply-chart]').first()
    await chart.locator('.chart-action').first().click()
    assert.equal(await chart.locator('tbody tr').count(), 3)
    const downloadEvent = page.waitForEvent('download')
    await chart.locator('.chart-action').last().click()
    const download = await downloadEvent
    await download.saveAs(path.join(output, 'themed-chart.svg'))
    const svg = await fs.readFile(path.join(output, 'themed-chart.svg'), 'utf8')
    assert(svg.includes('width=') && svg.includes('height='))
    assert(/fill:\s*(?:rgb|oklch|color\()/.test(svg), 'export resolves the app theme colors')
    await page.evaluate(() => {
      document.documentElement.classList.remove('dark')
      window.__layoutProbe.setStatic(true)
    })
    await page.locator('[data-inline-kind="steps"]').waitFor()
    await page.setViewportSize({ width: 320, height: 1000 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    await page.screenshot({ path: path.join(output, 'static-light-320.png'), fullPage: true })
    const touch = await browser.newContext({ viewport: { width: 375, height: 900 }, hasTouch: true, isMobile: true })
    const touchPage = await touch.newPage()
    touchPage.on('pageerror', (error) => errors.push(error.message))
    await touchPage.route('**/*', (route) =>
      route.request().url().startsWith(origin) ? route.continue() : route.abort(),
    )
    await touchPage.goto(origin + '/__inline-style', { waitUntil: 'networkidle' })
    const hitTargets = await touchPage
      .locator('[data-interactive-panel]')
      .locator('.ui-tabs button,.ui-button,.ui-check,.ui-task,.ui-field select,.ui-field input')
      .evaluateAll((nodes) =>
        nodes.map((node) => ({
          kind: node.tagName + ':' + node.getAttribute('type') + ':' + node.className,
          height: node.getBoundingClientRect().height,
        })),
      )
    for (const target of hitTargets) assert(target.height >= 40, `touch target ${target.kind}: ${target.height}`)
    assert.equal(await touchPage.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    assert.deepEqual(errors, [])
    await fs.writeFile(
      path.join(output, 'layout-results.json'),
      JSON.stringify(
        {
          passed: true,
          kinds: 24,
          measurements,
          checks: [
            'all component kinds',
            '320–1240px',
            'light/dark',
            'large host typography',
            'density setting',
            'RTL',
            'scoped CSS',
            'responsive nested grids',
            'readable chart labels',
            'theme-preserving SVG export',
            'keyboard disclosure',
            '40px touch targets',
            'static content',
          ],
        },
        null,
        2,
      ),
    )
    console.log(`Inline UI layout probe passed. Artifacts: ${output}`)
  } finally {
    if (browser) await browser.close()
    if (server) await server.close()
    await fs.rm(temporary, { recursive: true, force: true })
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
