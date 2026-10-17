// Start Vite, then run with AGENA_UI_TEST_BROWSER_MODULE pointing to playwright-core.
// The fixture uses production components and cursor navigation, with isolated reply actions.
import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import { join } from 'node:path'

const { chromium, webkit } = await import(process.env.AGENA_UI_TEST_BROWSER_MODULE || 'playwright-core')
const base = process.env.AGENA_UI_TEST_URL || 'http://127.0.0.1:5173'
const output = process.env.AGENA_UI_TEST_OUTPUT || '/tmp/agena-interaction-keyboard'
const engine = process.env.AGENA_UI_TEST_ENGINE === 'webkit' ? webkit : chromium
const browser = await engine.launch({
  headless: true,
  ...(process.env.AGENA_UI_TEST_CHROME_PATH ? { executablePath: process.env.AGENA_UI_TEST_CHROME_PATH } : {}),
})
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
page.setDefaultTimeout(8000)
const errors = []
const checks = []
page.on('pageerror', (error) => errors.push(error.message))
const activeMatches = (selector) => page.evaluate((selector) => document.activeElement?.matches(selector), selector)
const ask = '[data-interaction-request-id="ask"]'
const review = '[data-interaction-request-id="review"]'
const permission = '[data-interaction-request-id="permission"]'
const control = (root, key) => `${root} [data-interaction-control-key="${key}"]`
const focused = async (selector) => {
  await page.waitForFunction((selector) => document.activeElement?.matches(selector), selector)
}
async function tabTo(selector, reverse = false) {
  for (let i = 0; i < 70; i++) {
    if (await activeMatches(selector)) return
    await page.keyboard.press(reverse ? 'Shift+Tab' : 'Tab')
  }
  assert.fail(
    `Keyboard could not reach ${selector}. Active: ${await page.evaluate(() => document.activeElement.outerHTML.slice(0, 1200))}`,
  )
}
async function reset() {
  await page.goto(`${base}/tests/fixtures/interaction-keyboard.html`, { waitUntil: 'networkidle' })
  await page.locator(`${ask} input`).first().waitFor()
}
function check(name) {
  checks.push(name)
  console.log(`PASS ${name}`)
}
async function assertCompactChoices() {
  const layout = await page.evaluate(() => {
    const rowHeight =
      parseFloat(getComputedStyle(document.documentElement).fontSize) *
      (matchMedia('(pointer: coarse)').matches ? 2.25 : 2)
    const rows = Array.from(
      document.querySelectorAll('.interaction-choice, .part-action, .part-disclosure, .part-link, .part-toggle'),
    )
      .filter((row) => row.getBoundingClientRect().height > 0)
      .map((row) => {
        const style = getComputedStyle(row)
        return {
          height: row.getBoundingClientRect().height,
          width: row.getBoundingClientRect().width,
          available: row.parentElement.clientWidth,
          border: parseFloat(style.borderTopWidth),
          background: style.backgroundColor,
          outlined: row.matches('.interaction-choice, .part-action'),
          iconOnly: row.classList.contains('part-action-icon'),
          label: row.textContent.trim().slice(0, 80),
        }
      })
    const workspaces = Array.from(document.querySelectorAll('.interaction-workspace')).map((workspace) => ({
      width: workspace.getBoundingClientRect().width,
      gap: parseFloat(getComputedStyle(workspace).rowGap),
    }))
    return {
      rowHeight,
      rows,
      workspaces,
      maxWidth: parseFloat(getComputedStyle(document.documentElement).fontSize) * 48,
    }
  })
  assert.ok(layout.rows.length >= 8)
  for (const row of layout.rows) {
    assert.ok(Math.abs(row.height - layout.rowHeight) <= 1, `Choice grew to ${row.height}px: ${row.label}`)
    assert.ok(row.width <= row.available + 1, `Choice overflows its pane: ${row.label}`)
    if (row.outlined) {
      assert.ok(row.border >= 1, `Choice lacks its visible boundary: ${row.label}`)
      assert.notEqual(row.background, 'rgba(0, 0, 0, 0)', `Choice lacks its button surface: ${row.label}`)
    }
    if (row.iconOnly) assert.ok(Math.abs(row.width - layout.rowHeight) <= 1, 'Icon controls keep square targets')
  }
  for (const workspace of layout.workspaces) {
    assert.ok(workspace.width <= layout.maxWidth + 1, 'Action list stretches beyond its reading width')
    assert.ok(workspace.gap >= 4, 'Options need visible space between their boundaries')
  }
}
async function assertPlainTextFlow() {
  const surfaces = await page.evaluate(() =>
    Array.from(
      document.querySelectorAll(
        '[data-transcript-interaction-part], .interaction-result, .part-disclosure, .part-link, .part-toggle, .part-toolbar, [data-content-output], [data-activity-output]',
      ),
    )
      .filter((element) => element.getBoundingClientRect().height > 0)
      .map((element) => {
        const style = getComputedStyle(element)
        return {
          label: element.textContent.trim().slice(0, 80),
          borders: [style.borderTopWidth, style.borderRightWidth, style.borderBottomWidth, style.borderLeftWidth],
          background: style.backgroundColor,
        }
      }),
  )
  assert.ok(surfaces.length > 0)
  for (const surface of surfaces) {
    assert.ok(
      surface.borders.every((border) => parseFloat(border) === 0),
      `Content gained a nested frame: ${surface.label}`,
    )
    assert.equal(surface.background, 'rgba(0, 0, 0, 0)', `Content gained a panel fill: ${surface.label}`)
  }
}
async function stressChoiceCopy() {
  await page.evaluate(async () => {
    for (const part of interactionKeyboardFixture.message.parts) {
      const choice = part.agenaContent.user_input?.requests[0]?.request.questions[0]?.options[0]
      if (!choice) continue
      choice.label = '超长选项中文🙂LongLabel'.repeat(80)
      choice.description = Array.from({ length: 80 }, (_, i) => `第 ${i + 1} 段说明 ${'long-text-'.repeat(20)}`).join(
        '\n',
      )
    }
    await interactionKeyboardFixture.revise()
  })
  await page.waitForFunction(() =>
    document
      .querySelector('[data-interaction-request-id="review"] .interaction-choice-label')
      ?.textContent.includes('超长选项'),
  )
}
try {
  await mkdir(output, { recursive: true })
  await reset()
  await assertCompactChoices()
  await assertPlainTextFlow()
  assert.equal(await page.locator('[data-fixture-resources] a').count(), 1)
  assert.equal(
    await page.locator('[data-fixture-resources] a').getAttribute('href'),
    'https://example.test/reference/keyboard-navigation',
  )
  await tabTo('[data-fixture-resources] button')
  await page.keyboard.press('Enter')
  assert.deepEqual(await page.evaluate(() => interactionKeyboardFixture.state.opened), [
    { path: 'docs/keyboard-navigation-reference.md', url: '' },
  ])
  await page.evaluate(() => document.documentElement.classList.add('dark'))
  await assertCompactChoices()
  await assertPlainTextFlow()
  await page.evaluate(() => document.documentElement.classList.remove('dark'))
  await tabTo('[data-part-id="1"] [data-tool-details-toggle]')
  await page.keyboard.press('Enter')
  await page.locator('[data-part-id="1"] [data-tool-detail-section="input"]').waitFor()
  await assertCompactChoices()
  await assertPlainTextFlow()
  await page.keyboard.press('Enter')
  check('context, resources and technical disclosures stay unframed while their native controls remain reachable')
  await stressChoiceCopy()
  await assertCompactChoices()
  check('desktop choices retain one-row height with long labels, multiline descriptions and emoji')
  await reset()
  assert.ok(await activeMatches('body'))
  // Drag a few pixels within the choice text, then release on the same row.
  // Native labels must remain controls instead of starting transcript selection.
  for (const key of ['question:0:option:Production', 'question:0:option:Staging']) {
    const input = page.locator(control(ask, key))
    const label = input.locator('..').locator('.interaction-choice-label')
    const box = await label.boundingBox()
    await page.mouse.move(box.x + 4, box.y + box.height / 2)
    await page.mouse.down()
    await page.mouse.move(box.x + 9, box.y + box.height / 2 + 1, { steps: 3 })
    await page.mouse.up()
    assert.equal(await input.isChecked(), true)
    assert.equal(await page.evaluate(() => window.getSelection()?.toString() || ''), '')
  }
  assert.equal(await page.evaluate(() => interactionKeyboardFixture.state.replies.length), 0)
  await page.locator(control(ask, 'question:0:option:Production')).focus()
  await page.keyboard.press('ArrowRight')
  const multiInput = page.locator(control(ask, 'question:1:option:Types'))
  const multiBox = await multiInput.locator('..').locator('.interaction-choice-description').boundingBox()
  for (const expected of [true, false]) {
    await page.mouse.move(multiBox.x + 4, multiBox.y + multiBox.height / 2)
    await page.mouse.down()
    await page.mouse.move(multiBox.x + 10, multiBox.y + multiBox.height / 2 + 1, { steps: 3 })
    await page.mouse.up()
    assert.equal(await multiInput.isChecked(), expected, 'a slight drag activates the option exactly once')
    assert.equal(await page.evaluate(() => window.getSelection()?.toString() || ''), '')
  }
  check('clicks with slight pointer movement over labels and descriptions select reliably exactly once')
  await reset()
  await tabTo(`${ask} input`)
  check('pending interactions are reachable with Tab without taking focus on mount')
  await page.keyboard.press('ArrowDown')
  await focused(control(ask, 'question:0:option:Production'))
  await page.keyboard.press('Space')
  await page.keyboard.press('Enter')
  await focused(control(ask, 'question:1:option:Types'))
  assert.equal(await page.evaluate(() => interactionKeyboardFixture.state.replies.length), 0)
  await page.keyboard.press('Space')
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Space')
  await page.keyboard.press('Space')
  assert.equal(await page.locator(control(ask, 'question:1:option:Build')).isChecked(), false)
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Space')
  await focused(control(ask, 'question:1:option:Lint'))
  assert.equal(await page.locator(control(ask, 'question:1:option:Lint')).isChecked(), true)
  await page.keyboard.press('Enter')
  await focused(control(ask, 'question:2:custom'))
  assert.equal(await page.locator(`${ask} input[type="radio"]`).count(), 1)
  await page.keyboard.press('Space')
  assert.equal(await page.locator(control(ask, 'question:2:custom')).isChecked(), true)
  await focused(`${ask} textarea`)
  await page.keyboard.insertText('中文补充')
  await page.keyboard.press('Shift+Enter')
  await page.keyboard.insertText('第二行')
  assert.equal(await page.locator(`${ask} textarea`).inputValue(), '中文补充\n第二行')
  await page.keyboard.press('Tab')
  await focused(control(ask, 'page:previous'))
  await page.keyboard.press('Shift+Tab')
  await focused(`${ask} textarea`)
  check('single choice, multiple choice and custom-only questions work with the keyboard')
  check('Tab and Shift+Tab reach navigation from a custom editor; Shift+Enter inserts a line')

  await page.evaluate(() => {
    window.nativeKeys = []
    window.addEventListener('keydown', (event) => {
      if (event.ctrlKey && event.key !== 'Control')
        window.nativeKeys.push({ key: event.key, prevented: event.defaultPrevented })
    })
  })
  await page.keyboard.press('Control+x')
  await page.keyboard.press('Control+e')
  await page.keyboard.press('Control+d')
  assert.equal(await page.evaluate(() => interactionKeyboardFixture.state.replies.length), 0)
  assert.deepEqual(await page.evaluate(() => nativeKeys.map((key) => key.prevented)), [false, false, false])
  await page.locator(`${ask} textarea`).dispatchEvent('keydown', { key: 'Enter', isComposing: true, keyCode: 229 })
  await focused(`${ask} textarea`)
  await page.locator(`${ask} textarea`).dispatchEvent('keydown', { key: 'Enter', keyCode: 229 })
  await focused(`${ask} textarea`)
  check('native editing shortcuts and Chinese IME confirmation do not cancel or submit an answer')
  // Ctrl+D may edit text on macOS; restore the intended answer using keyboard input.
  await page.keyboard.press('Meta+a')
  await page.keyboard.insertText('中文补充\n第二行')
  await tabTo(control(ask, 'page:review'))
  await page.keyboard.press('Enter')
  await focused(control(ask, 'summary:0'))
  assert.equal(await page.locator(`${ask} [data-interaction-summary] > button`).count(), 3)
  await page.keyboard.press('Enter')
  await focused(control(ask, 'question:0:option:Staging'))
  assert.equal(await page.locator(control(ask, 'question:0:option:Production')).isChecked(), true)
  await page.keyboard.press('ArrowLeft')
  await focused(control(ask, 'question:0:option:Staging'))
  await page.keyboard.press('ArrowRight')
  await focused(control(ask, 'question:1:option:Types'))
  assert.equal(await page.locator(control(ask, 'question:1:option:Types')).isChecked(), true)
  assert.equal(await page.locator(control(ask, 'question:1:option:Lint')).isChecked(), true)
  await page.keyboard.press('h')
  await focused(control(ask, 'question:0:option:Staging'))
  await page.keyboard.press('l')
  await focused(control(ask, 'question:1:option:Types'))
  await page.keyboard.press('Enter')
  await focused(control(ask, 'question:2:custom'))
  await page.keyboard.press('ArrowRight')
  await focused(control(ask, 'question:2:custom'))
  assert.equal(await page.locator(`${ask} input[type="radio"]`).count(), 1)
  await page.keyboard.press('Space')
  assert.equal(await page.locator(control(ask, 'question:2:custom')).isChecked(), true)
  await focused(`${ask} textarea`)
  assert.equal(await page.locator(`${ask} textarea`).inputValue(), '中文补充\n第二行')
  await page.keyboard.press('Enter')
  await focused('[data-part-id="1"] [data-transcript-vim-toggle="true"]')
  assert.deepEqual(await page.evaluate(() => interactionKeyboardFixture.state.replies[0]), {
    id: 'ask',
    payload: { answers: { 0: ['Production'], 1: ['Types', 'Lint'], 2: ['中文补充\n第二行'] } },
  })
  await assertPlainTextFlow()
  check('horizontal question navigation preserves answers; final Enter submits directly and restores part focus')

  await reset()
  await tabTo(`${ask} input`)
  await page.keyboard.press('Control+Enter')
  await focused(`${ask} input`)
  assert.equal(await page.locator(`${ask} [data-interaction-summary]`).count(), 0)
  await page.keyboard.press('Enter')
  await focused(`${ask} input`)
  assert.equal(await page.evaluate(() => interactionKeyboardFixture.state.replies.length), 0)
  check('incomplete answers cannot be submitted and keyboard validation returns to the unanswered question')

  // Enter the expanded card from the production Vim cursor, then leave it with Escape.
  await page.keyboard.press('Escape')
  await focused('[data-part-id="1"] [data-transcript-vim-toggle="true"]')
  await page.keyboard.press('ArrowRight')
  await focused(`${ask} input`)
  await page.keyboard.press('e')
  await focused(`${ask} textarea`)
  await page.keyboard.insertText('Draft remains available')
  await page.evaluate(() => {
    interactionKeyboardFixture.state.viewKey++
  })
  await focused(`${ask} textarea`)
  assert.equal(await page.locator(`${ask} textarea`).inputValue(), 'Draft remains available')
  check('remounting the request preserves both the custom draft and keyboard focus')
  await page.keyboard.press('Escape')
  await focused(control(ask, 'question:0:custom'))
  await page.keyboard.press('Escape')
  await focused('[data-part-id="1"] [data-transcript-vim-toggle="true"]')
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('e')
  await focused(`${ask} textarea`)
  assert.equal(await page.locator(`${ask} textarea`).inputValue(), 'Draft remains available')
  await page.keyboard.press('Escape')
  await page.keyboard.press('Escape')
  check('transcript arrows enter interactions and Escape restores the part without losing the custom draft')

  await tabTo(`${review} pre`)
  await page.keyboard.press('ArrowDown')
  await focused(`${review} pre`)
  await page.keyboard.press('e')
  assert.equal(await page.locator(`${review} textarea`).count(), 0)
  await page.keyboard.press('Control+Enter')
  assert.equal(await page.evaluate(() => interactionKeyboardFixture.state.replies.length), 0)
  check('review Markdown controls keep their keys without opening the answer editor or submitting a decision')
  const approveOn = control(review, 'question:0:option:Approve with autorun on')
  const approveOff = control(review, 'question:0:option:Approve with autorun off')
  await tabTo(approveOn)
  assert.match(await page.locator(review).innerText(), /批准并开启自动执行/)
  assert.doesNotMatch(
    await page.locator(review).innerText(),
    /Approve with|Return to planning|Reject the current|Cancel the plan entirely/,
  )
  for (const locale of ['en-US', 'es-ES', 'fr-FR', 'hi-IN', 'pt-BR', 'ar-SA', 'zh-CN']) {
    await page.evaluate((locale) => interactionKeyboardFixture.locale(locale), locale)
    await page.waitForFunction(
      (locale) =>
        document.querySelector('[data-interaction-request-id="review"]').getAttribute('aria-label') !==
        'chat.planReview.creationTitle',
      locale,
    )
    if (locale !== 'en-US') assert.doesNotMatch(await page.locator(review).innerText(), /Approve with autorun on/)
    assert.equal(await page.locator(review).locator('input[type="radio"]').count(), 0)
    assert.equal(await page.evaluate(() => interactionKeyboardFixture.state.replies.length), 0)
  }
  await page.evaluate(() => {
    interactionKeyboardFixture.state.clone = true
  })
  await page.locator(review).nth(1).waitFor()
  assert.equal(await page.locator(approveOn).count(), 2)
  assert.equal(await page.locator(review).nth(1).locator('[data-selected="true"]').count(), 0)
  await page.evaluate(() => {
    interactionKeyboardFixture.state.clone = false
  })
  await focused(approveOn)
  await page.keyboard.press('j')
  await focused(approveOff)
  await page.keyboard.press('Enter')
  await focused('[data-part-id="2"] [data-transcript-vim-toggle="true"]')
  assert.deepEqual(await page.evaluate(() => interactionKeyboardFixture.state.replies.at(-1)), {
    id: 'review',
    payload: { answers: { 0: ['Approve with autorun off'] } },
  })
  assert.equal(await page.locator(`${review} [data-interaction-control]`).count(), 0)
  assert.equal(
    await page.locator(`${review} [data-interaction-result] .interaction-result-value`).textContent(),
    '批准并关闭自动执行',
  )
  assert.equal(await page.locator(`${review} .interaction-choice`).count(), 0)
  await assertPlainTextFlow()
  check('all seven locales translate review options live while replies keep the original protocol values')
  check('review cards activate directly and moving their cursor never records an answer')

  await tabTo(control(permission, 'permission:once'))
  await page.keyboard.press('l')
  await focused(control(permission, 'permission:always'))
  await page.keyboard.press('End')
  await focused(control(permission, 'permission:reject'))
  await page.keyboard.press('Home')
  await focused(control(permission, 'permission:once'))
  await page.evaluate(() => {
    interactionKeyboardFixture.state.failNext = true
  })
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => !interactionKeyboardFixture.state.failNext)
  await focused(control(permission, 'permission:once'))
  assert.equal(await page.evaluate(() => interactionKeyboardFixture.state.replies.length), 1)
  await page.keyboard.press('Enter')
  await focused('[data-part-id="3"] [data-transcript-vim-toggle="true"]')
  assert.equal(await page.evaluate(() => interactionKeyboardFixture.state.replies.at(-1).payload.decision), 'once')
  check('permission action movement, Home/End, API failure retry and successful focus restoration work')

  await tabTo('#composer')
  await page.keyboard.press('Control+p')
  const body = '[data-session-section-body]'
  await focused(body)
  await page.locator(body).getByText('Keyboard task 80', { exact: true }).waitFor()
  await page.keyboard.press('PageDown')
  await page.waitForFunction(() => document.querySelector('[data-session-section-body]').scrollTop > 100)
  await page.keyboard.press('End')
  await page.waitForFunction(() => {
    const body = document.querySelector('[data-session-section-body]')
    return body.scrollTop > 100 && body.scrollHeight - body.clientHeight - body.scrollTop < 2
  })
  await page.keyboard.press('Home')
  await page.waitForFunction(() => document.querySelector('[data-session-section-body]').scrollTop === 0)
  await page.keyboard.press('Tab')
  await focused(`${body} input[type="checkbox"]`)
  const autorun = await page.locator(`${body} input`).isChecked()
  await page.keyboard.press('Space')
  await page.waitForFunction((value) => interactionKeyboardFixture.plan.autorun.value !== value, autorun)
  await focused(`${body} input[type="checkbox"]`)
  await page.keyboard.press('Shift+Tab')
  await focused(body)
  await page.keyboard.press('Shift+Tab')
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => !interactionKeyboardFixture.plan.loading.value)
  await focused('[data-session-section] button')
  const toc = '[data-session-section] [data-markdown-toc-trigger]'
  await tabTo(toc)
  await page.waitForFunction(
    () =>
      getComputedStyle(document.querySelector('[data-session-section] [data-markdown-toc-trigger]').parentElement)
        .opacity === '1',
  )
  await page.keyboard.press('ArrowDown')
  await focused('[data-session-section] [data-markdown-toc-item]')
  await page.keyboard.press('End')
  assert.equal(await page.evaluate(() => document.activeElement.textContent.trim()), 'Code sample')
  await page.keyboard.press('Enter')
  await focused(toc)
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Escape')
  await focused(toc)
  assert.equal(await page.locator(toc).getAttribute('aria-expanded'), 'false')
  assert.equal(await page.locator('[data-session-section]').count(), 1)
  check(
    'Markdown contents are visible on keyboard focus, support arrows and Home/End, and restore focus on selection or Escape',
  )
  const markdownToggle = '[data-session-section] .oc-md-codeblock__header [data-oc-code-action="toggle"]'
  await tabTo('[data-session-section] .oc-md-codeblock__more')
  await page.keyboard.press('Enter')
  await focused(markdownToggle)
  assert.equal(await page.locator('[data-session-section] [data-oc-codeblock]').getAttribute('data-oc-expanded'), '1')
  await page.keyboard.press('Enter')
  assert.equal(await page.locator('[data-session-section] [data-oc-codeblock]').getAttribute('data-oc-expanded'), '0')
  await page.keyboard.press('Tab')
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => interactionKeyboardFixture.state.clipboard.includes('Keyboard line 40:'))
  check('Markdown code expansion and copying are reachable with Tab; expansion keeps a visible focus destination')
  await page.keyboard.press('Escape')
  await focused('[data-plan-state-viewer="true"]')
  check(
    'Ctrl+P opens Plan from the composer; keyboard scrolling, autorun and refresh preserve focus; Escape restores its trigger',
  )

  await reset()
  await tabTo('#composer')
  await page.evaluate(() => interactionKeyboardFixture.revise())
  await focused('#composer')
  await page.evaluate(() => {
    interactionKeyboardFixture.pending.delete('review')
  })
  await focused('#composer')
  check('streaming updates and requests resolved elsewhere do not take focus from the composer')
  await tabTo(`${ask} input`, true)
  await page.evaluate(() => {
    interactionKeyboardFixture.pending.delete('ask')
  })
  await focused('[data-part-id="1"] [data-transcript-vim-toggle="true"]')
  check('resolving the focused request elsewhere restores a visible keyboard destination')

  await tabTo('[data-fixture-code] .absolute button')
  await page.keyboard.press('Enter')
  await focused('[data-fixture-code] button[aria-label="收起代码块"]')
  await page.keyboard.press('Enter')
  check('standalone code blocks keep focus on their toggle after keyboard expansion')
  assert.match(
    await page.evaluate(async () => {
      const { transcriptTextProjection } = await import('/src/pages/chat/transcriptDomCursor.ts')
      return transcriptTextProjection(document.querySelector('[data-fixture-code]')).text
    }),
    /Keyboard line 1:/,
  )
  assert.match(
    await page.evaluate(async () => {
      const { transcriptTextProjection } = await import('/src/pages/chat/transcriptDomCursor.ts')
      return transcriptTextProjection(document.querySelector('[data-fixture-diff]')).text
    }),
    /const line0 = 0/,
  )
  check('keyboard scrolling regions retain code and diff content in transcript text projections')
  await tabTo('[data-fixture-diff] button[aria-expanded="false"]')
  await page.keyboard.press('Enter')
  await tabTo('[data-fixture-diff] [data-diff-viewport]', true)
  await page.keyboard.press('PageDown')
  await page.waitForFunction(() => document.querySelector('[data-diff-viewport]').scrollTop > 100)
  await focused('[data-diff-viewport]')
  check('expanded diff viewports retain keyboard ownership while scrolling')

  await reset()
  await tabTo(approveOn)
  await page.keyboard.press('e')
  await focused(`${review} textarea`)
  await page.keyboard.insertText('请修改步骤二')
  await page.keyboard.press('Enter')
  await focused('[data-part-id="2"] [data-transcript-vim-toggle="true"]')
  await tabTo(review)
  await page.keyboard.press('Escape')
  assert.equal(
    await page.locator(`${review} [data-interaction-result] .interaction-result-value`).textContent(),
    '请修改步骤二',
  )
  assert.equal(await page.locator(`${review} textarea, ${review} [data-interaction-control]`).count(), 0)
  check('Escape on a completed custom review preserves the committed answer')

  await page.setViewportSize({ width: 390, height: 844 })
  await reset()
  await stressChoiceCopy()
  await assertCompactChoices()
  await assertPlainTextFlow()
  check('narrow panes keep every option compact without horizontal overflow')
  await reset()
  await tabTo(`${ask} input`)
  await page.keyboard.press('Enter')
  await focused(control(ask, 'question:1:option:Types'))
  await page.keyboard.press('e')
  await focused(`${ask} textarea`)
  await page.keyboard.insertText('移动端外接键盘')
  await page.keyboard.press('Tab')
  await focused(control(ask, 'page:previous'))
  check('the same Ask navigation works at mobile viewport sizes')
  await page.screenshot({ path: join(output, 'mobile.png'), fullPage: true })
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.screenshot({ path: join(output, 'desktop.png'), fullPage: true })
  await page.keyboard.press('Control+x')
  await focused('[data-part-id="1"] [data-transcript-vim-toggle="true"]')
  assert.deepEqual(await page.evaluate(() => interactionKeyboardFixture.state.replies.at(-1)), {
    id: 'ask',
    payload: { cancelled: true },
  })
  check('Ask cancellation outside the editor returns focus to its part')
  assert.deepEqual(errors, [])
  console.log(
    JSON.stringify(
      { engine: process.env.AGENA_UI_TEST_ENGINE || 'chromium', checks: checks.length, errors, output },
      null,
      2,
    ),
  )
} catch (error) {
  await page.screenshot({ path: join(output, 'failure.png'), fullPage: true })
  console.error('Active element:', await page.evaluate(() => document.activeElement.outerHTML.slice(0, 1200)))
  console.error('Browser errors:', errors)
  throw error
} finally {
  await browser.close()
}
