import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const { chromium } = await import(process.env.AGENA_UI_TEST_BROWSER_MODULE || 'playwright-core')
const base = process.env.AGENA_UI_TEST_URL || 'http://127.0.0.1:5179'
const temp = await mkdtemp(join(tmpdir(), 'agena-git-sidebar-'))
const repo = join(temp, 'project'),
  worktree = join(temp, 'other')
await mkdir(join(repo, 'src/features'), { recursive: true })
const git = (...args) =>
  execFileSync('git', ['-C', repo, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()
git('init', '-b', 'feature/rich-preview')
git('config', 'user.name', 'UI Test')
git('config', 'user.email', 'test@example.invalid')
git('config', 'commit.gpgSign', 'false')
await writeFile(join(repo, 'src/features/WorkspaceEditorGroupPane.vue'), '<main>Before</main>\n')
await writeFile(join(repo, 'src/features/gitNavigation.ts'), 'export const navigation = []\n')
git('add', '.')
git('commit', '-m', 'Initial fixture')
git('branch', 'main')
git('worktree', 'add', '-b', 'feature/other', worktree)
await writeFile(join(repo, 'src/features/WorkspaceEditorGroupPane.vue'), '<main>After</main>\n')
await writeFile(join(repo, 'src/features/gitNavigation.ts'), 'export const navigation = ["branches"]\n')
git('add', 'src/features/gitNavigation.ts')
await writeFile(join(repo, 'src/features/ReplyChart.vue'), '<svg />\n')
const browser = await chromium.launch({
  headless: true,
  ...(process.env.AGENA_UI_TEST_CHROME_PATH ? { executablePath: process.env.AGENA_UI_TEST_CHROME_PATH } : {}),
})
try {
  const page = await browser.newPage({ viewport: { width: 1000, height: 900 } })
  page.setDefaultTimeout(20000)
  await page.goto(`${base}/tests/fixtures/git-sidebar.html?${new URLSearchParams({ directory: repo })}`)
  await page.getByText('WorkspaceEditorGroupPane.vue', { exact: true }).waitFor()
  await page.locator('[data-git-fixture]').evaluate((element) => {
    element.style.width = '345px'
  })
  const pane = page.locator('[data-git-fixture]')
  assert.equal(await pane.getByText('src/features', { exact: true }).count(), 3)
  const changesTab = pane.getByRole('tab', { name: /^变更/ })
  const historyTab = pane.getByRole('tab', { name: '历史', exact: true })
  await changesTab.focus()
  await page.keyboard.press('ArrowRight')
  assert.equal(await historyTab.getAttribute('aria-selected'), 'true')
  await page.keyboard.press('Home')
  assert.equal(await changesTab.getAttribute('aria-selected'), 'true')
  const commit = pane.getByRole('button', { name: '提交', exact: true })
  assert.equal(await commit.isDisabled(), true)
  await pane.getByRole('textbox', { name: /提交信息/ }).fill('test: verify staged commit from sidebar')
  assert.equal(await commit.isEnabled(), true)
  await pane.getByRole('button', { name: '分支', exact: true }).click()
  await page.getByRole('dialog').getByText('main', { exact: true }).waitFor()
  await page.getByRole('dialog').getByRole('button', { name: '关闭', exact: true }).first().click()
  await pane.getByRole('button', { name: '工作树', exact: true }).click()
  await page.getByRole('dialog').getByText('feature/other', { exact: true }).waitFor()
  await page.getByRole('dialog').getByRole('button', { name: '关闭', exact: true }).first().click()
  await pane.getByRole('button', { name: '对比', exact: true }).click()
  await page.getByRole('dialog').waitFor()
  await page.getByRole('dialog').getByRole('button', { name: '关闭', exact: true }).first().click()
  const row = pane.locator('.git-file-row').filter({ hasText: 'WorkspaceEditorGroupPane.vue' })
  await row.hover()
  await row.getByRole('button', { name: '暂存', exact: true }).click()
  await pane.getByText('已暂存 2 个文件', { exact: true }).waitFor()
  assert.ok(git('diff', '--cached', '--name-only').includes('WorkspaceEditorGroupPane.vue'))
  const committed = page.waitForResponse(
    (response) => response.url().includes('/git/commit') && response.request().method() === 'POST',
  )
  await commit.click()
  const response = await committed
  assert.equal(response.ok(), true, await response.text())
  await pane.getByText('已暂存 2 个文件', { exact: true }).waitFor({ state: 'hidden' })
  assert.equal(git('log', '-1', '--format=%s'), 'test: verify staged commit from sidebar')
  assert.equal(git('diff', '--cached', '--name-only'), '')
  await page.waitForFunction(() =>
    Array.from(document.querySelectorAll('[data-git-fixture] button')).some(
      (button) => button.textContent.trim() === '分支' && !button.disabled,
    ),
  )
  assert.equal(await commit.isDisabled(), true)
  await page.screenshot({ path: join(temp, 'git-sidebar.png') })
  console.log(
    `Git sidebar filenames, branches, worktrees, compare, quick staging and real commit passed. Screenshot: ${join(temp, 'git-sidebar.png')}`,
  )
} catch (error) {
  await browser
    .contexts()[0]
    ?.pages()[0]
    ?.screenshot({ path: join(temp, 'failure.png'), fullPage: true })
    .catch(() => {})
  console.error(`Failure screenshot: ${join(temp, 'failure.png')}`)
  throw error
} finally {
  await browser.close()
  git('worktree', 'remove', '--force', worktree)
  await rm(repo, { recursive: true, force: true })
}
