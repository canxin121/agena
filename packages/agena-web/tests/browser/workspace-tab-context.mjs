import assert from 'node:assert/strict'
const { chromium } = await import(process.env.AGENA_UI_TEST_BROWSER_MODULE || 'playwright-core')
const base = process.env.AGENA_UI_TEST_URL || 'http://127.0.0.1:5179'
const browser = await chromium.launch({
  headless: true,
  ...(process.env.AGENA_UI_TEST_CHROME_PATH ? { executablePath: process.env.AGENA_UI_TEST_CHROME_PATH } : {}),
})
try {
  const page = await browser.newPage()
  await page.goto(`${base}/tests/fixtures/keyboard-workflows.html`)
  const tabs = page.locator('[data-workspace-tab-primary]')
  await tabs.first().waitFor()
  await page.evaluate(() => keyboardWorkflows.router.isReady())
  const settleNavigation = () =>
    page.waitForFunction(
      () => keyboardWorkflows.router.currentRoute.value.query.windowId === keyboardWorkflows.ui.activeWorkspaceWindowId,
    )
  const state = () =>
    page.evaluate(() => ({
      active: keyboardWorkflows.ui.activeWorkspaceWindowId,
      focused: keyboardWorkflows.ui.focusedWorkspaceWindowId,
      group: keyboardWorkflows.ui.activeWorkspaceGroupId,
      directory: keyboardWorkflows.directory.currentDirectory,
      route: keyboardWorkflows.router.currentRoute.value.fullPath,
    }))
  let before = await state()
  await tabs.first().click({ button: 'right' })
  await page.getByText('复制标签', { exact: true }).waitFor()
  assert.deepEqual(await state(), before)
  await page.keyboard.press('Escape')
  await tabs.first().click()
  await settleNavigation()
  assert.notEqual((await state()).active, before.active)
  await tabs.first().press('ArrowRight')
  await settleNavigation()
  assert.equal((await state()).active, before.active)
  await page.evaluate(() =>
    keyboardWorkflows.ui.splitWorkspaceWindowToRight(keyboardWorkflows.ui.activeWorkspaceWindowId),
  )
  before = await state()
  await tabs.first().click({ button: 'right' })
  await page.getByText('复制标签', { exact: true }).waitFor()
  assert.deepEqual(await state(), before)
  await page.keyboard.press('Escape')
  // The overflow button has the same rule as a native context menu.
  await page.locator('[data-workspace-tab-id]').first().locator('button').nth(1).click()
  await page.getByText('复制标签', { exact: true }).waitFor()
  assert.deepEqual(await state(), before)
  console.log('Inactive-tab right click, overflow, split groups, left click and keyboard navigation passed.')
} finally {
  await browser.close()
}
