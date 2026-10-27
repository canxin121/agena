// Isolated Skills REST + browser regression probe. Requires a built Agena binary,
// a production Web build, Playwright, and Chrome/Chromium. It never uses the live
// service: all mutations target a fresh data directory and temporary workspaces.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const os = require('node:os')
const net = require('node:net')
const { spawn, spawnSync } = require('node:child_process')
const { once } = require('node:events')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function until(read, predicate, label, attempts = 100) {
  for (let attempt = 0; attempt < attempts; attempt++) {
    const value = await read()
    if (predicate(value)) return value
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error('Timed out: ' + label)
}

function document(name, body = '# Evidence review\n\nCheck sources and record any missing evidence.\n') {
  return (
    '---\nname: ' +
    name +
    '\ndescription: Review source evidence. Use for research and verification.\nmetadata:\n  version: "1.0"\ncustom-extension:\n  retained: true\n---\n' +
    body
  )
}

async function main() {
  const repo = path.resolve(__dirname, '../../..')
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-skills-probe-'))
  const startup = path.join(temporary, 'startup')
  const workspace = path.join(temporary, 'project')
  const other = path.join(temporary, 'other')
  const source = path.join(temporary, 'source', 'folder-review')
  await Promise.all([startup, workspace, other, source].map((folder) => fs.mkdir(folder, { recursive: true })))
  await fs.writeFile(path.join(source, 'SKILL.md'), document('folder-review'))
  await fs.writeFile(path.join(source, 'notes.md'), 'Preserve the complete package.\n')
  const compat = path.join(workspace, '.agents/skills/compatible-review')
  await fs.mkdir(compat, { recursive: true })
  await fs.writeFile(path.join(compat, 'SKILL.md'), document('compatible-review'))

  const listener = net.createServer()
  await new Promise((resolve) => listener.listen(0, '127.0.0.1', resolve))
  const port = listener.address().port
  await new Promise((resolve) => listener.close(resolve))
  const origin = 'http://127.0.0.1:' + port
  const serverLog = await fs.open(path.join(temporary, 'server.log'), 'w')
  const server = spawn(
    process.env.AGENA_BIN || path.join(repo, 'target/debug/agena'),
    [
      '--data-dir',
      path.join(temporary, 'data'),
      'server',
      '--host',
      '127.0.0.1',
      '--port',
      String(port),
      '--workspace',
      startup,
      '--ui-dir',
      path.join(repo, 'packages/agena-web/dist'),
      '--mcp-enabled',
      'false',
    ],
    {
      cwd: startup,
      stdio: ['ignore', serverLog.fd, serverLog.fd],
      env: {
        ...process.env,
        AGENA_SERVER_PASSWORD: '',
        AGENA_SERVER_UI_PASSWORD: '',
        AGENA_DATA_DIR: path.join(temporary, 'data'),
        AGENA_MCP_ENABLED: 'false',
      },
    },
  )
  let browser
  let page
  try {
    await until(
      async () => {
        if (server.exitCode !== null)
          throw new Error('Probe server exited: ' + (await fs.readFile(path.join(temporary, 'server.log'), 'utf8')))
        return fetch(origin + '/auth/session')
          .then((response) => response.ok)
          .catch(() => false)
      },
      Boolean,
      'isolated server readiness',
      1200,
    )

    async function api(method, url, body, status = 200) {
      const response = await fetch(origin + '/api/v1' + url, {
        method,
        headers: { 'content-type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
      })
      const text = await response.text()
      assert.equal(response.status, status, method + ' ' + url + ': ' + text)
      return JSON.parse(text)
    }
    const project = await api('POST', '/workspaces/resolve', { path: workspace, create_if_missing: true })
    const secondary = await api('POST', '/workspaces/resolve', { path: other, create_if_missing: true })
    const scoped = (suffix = '', id = project.id) =>
      '/skills' + suffix + (suffix.includes('?') ? '&' : '?') + 'workspace_id=' + id
    let global = await api('POST', scoped(), { scope: 'global', document: document('evidence-review') })
    let skill = await api('POST', scoped(), { scope: 'workspace', document: document('evidence-review') })
    assert.ok(skill.path.startsWith(workspace + path.sep))
    assert.ok(global.path.startsWith(path.join(temporary, 'data') + path.sep))
    await assert.rejects(fs.stat(path.join(startup, '.agena/skills/evidence-review')))
    let catalog = await api('GET', scoped())
    assert.equal(catalog.skills.find((item) => item.id === global.id).shadowed, true)
    assert.equal(catalog.skills.find((item) => item.name === 'compatible-review').editable, false)
    const elsewhere = await api('GET', scoped('', secondary.id))
    assert.equal(
      elsewhere.skills.some((item) => item.id === skill.id),
      false,
    )
    assert.equal(elsewhere.skills.find((item) => item.id === global.id).shadowed, false)
    await api('POST', scoped(), { document: '---\nname: Invalid Name\n---\n' }, 400)

    const resource = await api('PUT', scoped('/' + skill.id + '/resource'), {
      path: 'references/checklist.md',
      content: '- Verify sources\n',
    })
    await api(
      'PUT',
      scoped('/' + skill.id + '/resource'),
      {
        path: resource.path,
        content: 'Unexpected replacement',
      },
      409,
    )
    await api(
      'PUT',
      scoped('/' + skill.id),
      {
        document: document('evidence-review', '# Concurrent edit\n'),
        expected_revision: skill.revision,
      },
      409,
    )
    skill = await api('GET', scoped('/' + skill.id))
    skill = await api('PUT', scoped('/' + skill.id + '/enabled'), {
      enabled: false,
      expected_revision: skill.revision,
    })
    assert.equal((await api('GET', scoped())).skills.find((item) => item.id === global.id).shadowed, false)
    skill = await api('PUT', scoped('/' + skill.id + '/enabled'), {
      enabled: true,
      expected_revision: skill.revision,
    })
    assert.ok(skill.document.includes('custom-extension:'))

    const files = [
      { path: 'asset-review/SKILL.md', content_base64: Buffer.from(document('asset-review')).toString('base64') },
      { path: 'asset-review/assets/sample.bin', content_base64: Buffer.from([0, 255, 128, 1]).toString('base64') },
      { path: 'asset-review/dist/report.json', content_base64: Buffer.from('{"retained":true}').toString('base64') },
      {
        path: 'asset-review/scripts/check.sh',
        content_base64: Buffer.from('#!/bin/sh\nexit 0\n').toString('base64'),
        executable: true,
      },
    ]
    const sourceRequest = { scope: 'workspace', source: { kind: 'files', files } }
    const preview = await api('POST', scoped('/install/preview'), sourceRequest)
    const selection = preview.candidates.filter((item) => item.valid).map((item) => item.path)
    assert.equal(selection.length, 1)
    const changedSource = structuredClone(sourceRequest)
    changedSource.source.files[2].content_base64 = Buffer.from('changed').toString('base64')
    await api(
      'POST',
      scoped('/install'),
      {
        ...changedSource,
        selected: selection,
        expected_source_revision: preview.revision,
      },
      409,
    )
    const installed = await api('POST', scoped('/install'), {
      ...sourceRequest,
      selected: selection,
      expected_source_revision: preview.revision,
    })
    let asset = await api('GET', scoped('/' + installed.installed[0].id))
    const assetRoot = path.dirname(asset.path)
    assert.deepEqual(await fs.readFile(path.join(assetRoot, 'assets/sample.bin')), Buffer.from([0, 255, 128, 1]))
    assert.equal(JSON.parse(await fs.readFile(path.join(assetRoot, 'dist/report.json'), 'utf8')).retained, true)
    if (process.platform !== 'win32') assert.ok((await fs.stat(path.join(assetRoot, 'scripts/check.sh'))).mode & 0o111)
    await api('GET', scoped('/' + asset.id + '/resource?path=assets%2Fsample.bin'), undefined, 400)
    asset = await api(
      'DELETE',
      scoped('/' + asset.id + '/resource?path=assets%2Fsample.bin&expected_revision=' + asset.revision),
    )
    assert.equal(
      asset.files.some((item) => item.path === 'assets/sample.bin'),
      false,
    )

    const archive = path.join(temporary, 'archive-review.zip')
    const zip = spawnSync(
      'python3',
      [
        '-c',
        'import sys,zipfile\nwith zipfile.ZipFile(sys.argv[1],"w") as z:\n z.writestr("archive-review/SKILL.md",sys.argv[2])\n z.writestr("archive-review/assets/value.bin",bytes([0,255,1]))',
        archive,
        document('archive-review'),
      ],
      { encoding: 'utf8' },
    )
    assert.equal(zip.status, 0, zip.stderr)
    const archiveRequest = {
      scope: 'workspace',
      source: {
        kind: 'archive',
        filename: 'archive-review.zip',
        data_base64: (await fs.readFile(archive)).toString('base64'),
      },
    }
    const archivePreview = await api('POST', scoped('/install/preview'), archiveRequest)
    const archiveInstall = await api('POST', scoped('/install'), {
      ...archiveRequest,
      selected: archivePreview.candidates.filter((item) => item.valid).map((item) => item.path),
      expected_source_revision: archivePreview.revision,
    })
    assert.equal(archiveInstall.installed[0].source.kind, 'upload')

    if (process.env.SKILLS_PROBE_GITHUB_URL) {
      const request = {
        scope: 'workspace',
        source: { kind: 'github', url: process.env.SKILLS_PROBE_GITHUB_URL },
      }
      const preview = await api('POST', scoped('/install/preview'), request)
      const selected = preview.candidates.filter((item) => item.valid).map((item) => item.path)
      assert.ok(selected.length, 'the GitHub source contains a valid skill')
      const result = await api('POST', scoped('/install'), {
        ...request,
        selected,
        expected_source_revision: preview.revision,
      })
      assert.ok(result.installed.length)
      assert.equal(result.installed[0].source.kind, 'github')
    }

    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || undefined,
      headless: true,
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
    })
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' })
    await context.addInitScript((directory) => {
      localStorage.setItem('studio.ui.locale', 'en-US')
      localStorage.setItem(
        'studio.directory.current-directory-by-window',
        JSON.stringify({ 'window-default': directory }),
      )
    }, workspace)
    page = await context.newPage()
    const pageErrors = []
    page.on('pageerror', (error) => pageErrors.push(error.message))
    await page.goto(origin + '/settings/skills?view=library&directory=' + encodeURIComponent(workspace))
    await page.getByRole('button', { name: 'Create skill', exact: true }).waitFor()
    await page.getByRole('button', { name: 'Create skill', exact: true }).click()
    const dialog = page.getByRole('dialog')
    await dialog.getByLabel(/^Name/).fill('browser-review')
    await dialog.getByLabel(/^Description/).fill('Review browser evidence. Use for interface checks.')
    await dialog
      .getByLabel('Instructions', { exact: true })
      .fill('# Browser review\n\nInspect the page and preserve evidence.\n')
    await dialog.getByRole('button', { name: 'Create skill', exact: true }).click()
    await dialog.waitFor({ state: 'hidden' })
    const editor = page.getByRole('textbox', { name: 'Skill file contents', exact: true })
    await editor.waitFor()
    await editor.fill(document('browser-review', '# Browser review\n\nUpdated from the real browser editor.\n'))
    await editor.press('Control+s')
    await until(
      () => api('GET', scoped()),
      (value) => value.skills.some((item) => item.name === 'browser-review'),
      'browser-created skill',
    )
    const created = (await api('GET', scoped())).skills.find((item) => item.name === 'browser-review')
    await until(
      () => api('GET', scoped('/' + created.id)),
      (value) => value.document.includes('real browser editor'),
      'browser edit persisted',
    )
    await page.getByRole('button', { name: 'Disable', exact: true }).click()
    await page.getByRole('button', { name: 'Enable', exact: true }).waitFor()
    await page.getByRole('button', { name: 'Enable', exact: true }).click()
    await page.getByRole('button', { name: 'Disable', exact: true }).waitFor()
    await page.getByRole('button', { name: 'Preview', exact: true }).last().click()
    await page.getByText('Updated from the real browser editor.', { exact: true }).waitFor()
    await page.screenshot({ path: path.join(temporary, 'skills-library-desktop.png'), fullPage: true })

    await page.getByRole('button', { name: 'Install skills', exact: true }).last().click()
    await page.getByRole('button', { name: /^Server folder/ }).click()
    await page.getByLabel(/^Folder path on the server/).fill(source)
    await page.getByRole('button', { name: 'Preview installation', exact: true }).click()
    await page.getByRole('button', { name: 'Install selected skills', exact: true }).waitFor()
    await page.screenshot({ path: path.join(temporary, 'skills-install-desktop.png'), fullPage: true })
    await page.getByRole('button', { name: 'Install selected skills', exact: true }).click()
    await until(
      () => api('GET', scoped()),
      (value) => value.skills.some((item) => item.name === 'folder-review'),
      'browser folder installation',
    )

    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto(origin + '/settings/skills?view=library&directory=' + encodeURIComponent(workspace))
    await page.getByText('browser-review', { exact: true }).first().waitFor()
    assert.ok(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 2),
      'mobile page should not overflow horizontally',
    )
    await page.screenshot({ path: path.join(temporary, 'skills-library-mobile.png'), fullPage: true })
    await page.getByText('browser-review', { exact: true }).first().click()
    await page.getByRole('textbox', { name: 'Skill file contents', exact: true }).waitFor()
    await page.getByRole('button', { name: 'Preview', exact: true }).last().click()
    await page.getByText('Updated from the real browser editor.', { exact: true }).waitFor()
    assert.ok(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 2),
      'mobile details should not overflow horizontally',
    )
    await page.screenshot({ path: path.join(temporary, 'skills-detail-mobile.png'), fullPage: true })

    const latest = await api('GET', scoped('/' + created.id))
    await api('DELETE', scoped('/' + created.id + '?expected_revision=' + latest.revision))
    await api('GET', scoped('/' + created.id), undefined, 404)
    assert.deepEqual(pageErrors, [])
    console.log(
      JSON.stringify(
        {
          ok: true,
          temporary,
          browser: await browser.version(),
          checks: [
            'workspace isolation',
            'global precedence',
            'compatibility discovery',
            'strict validation',
            'optimistic revisions',
            'enable/disable',
            'complete uploaded packages',
            'binary resource deletion',
            'archive upload',
            'browser create/edit/toggle',
            'browser folder preview/install',
            'desktop/mobile layout',
            'remove',
          ],
        },
        null,
        2,
      ),
    )
  } catch (error) {
    if (page) {
      await page.screenshot({ path: path.join(temporary, 'failure.png'), fullPage: true }).catch(() => {})
      console.error(
        (
          await page
            .locator('body')
            .innerText()
            .catch(() => '')
        ).slice(0, 6000),
      )
    }
    console.error('Probe artifacts: ' + temporary)
    throw error
  } finally {
    await browser?.close()
    if (server.exitCode === null) {
      const stopped = once(server, 'exit')
      server.kill('SIGTERM')
      await Promise.race([stopped, new Promise((resolve) => setTimeout(resolve, 5000))])
      if (server.exitCode === null) server.kill('SIGKILL')
    }
    await serverLog.close()
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
