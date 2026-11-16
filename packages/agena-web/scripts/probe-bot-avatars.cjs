const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')

// Called by probe-bot.cjs so avatar checks use the real app, isolated API and
// native file inputs, with the same browser/process cleanup as other Bot probes.
module.exports = async function probeBotAvatars({
  cdp,
  base,
  evidence,
  temporary,
  wait,
  check,
  visible,
  profiles,
  profileRequests,
  avatarReads,
  botA,
  botB,
  botC,
}) {
  const preview = '[data-testid="bot-avatar-preview"]'
  const header = '[data-testid="bot-header-avatar"]'
  const welcome = '[data-testid="bot-welcome"] [data-bot-avatar]'
  const row = (id) => `[data-sidebar-bot-id="${id}"] [data-bot-avatar]`
  const frame = () =>
    cdp.evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
  const matches = (selector, source) =>
    cdp.evaluate(`(() => {
    const root=[...document.querySelectorAll(${JSON.stringify(selector)})].find(n=>n.getClientRects().length);
    if(!root) return false;
    const image=root.querySelector('img');
    return ${JSON.stringify(source)} === null
      ? root.dataset.botAvatar==='fallback' && !image
      : !!image && image.getAttribute('src')===${JSON.stringify(source)} && image.complete && image.naturalWidth===128 && image.naturalHeight===128;
  })()`)
  const expectAvatar = async (selector, source, label) => {
    await wait(() => matches(selector, source), label)
    check(true, label)
  }
  const currentPreview = () => cdp.evaluate(`document.querySelector('${preview} img')?.getAttribute('src') || null`)
  const chooseFile = async (file) => {
    let chooser
    cdp.events.set('Page.fileChooserOpened', (event) => {
      chooser = event
    })
    await cdp.send('Page.setInterceptFileChooserDialog', { enabled: true })
    try {
      await cdp.click('[data-testid="bot-avatar-upload"]')
      await wait(() => chooser, 'native avatar file picker')
      assert(chooser.backendNodeId, 'avatar button opens its file input')
      await cdp.send('DOM.setFileInputFiles', { backendNodeId: chooser.backendNodeId, files: [file] })
    } finally {
      cdp.events.delete('Page.fileChooserOpened')
      await cdp.send('Page.setInterceptFileChooserDialog', { enabled: false })
    }
    await frame()
  }
  const upload = async (file) => {
    const before = await currentPreview()
    await chooseFile(file)
    const source = await wait(async () => {
      const value = await currentPreview()
      return value && value !== before && (await matches(preview, value)) ? value : null
    }, 'decoded and cropped avatar preview')
    return source
  }
  const openEditor = async () => {
    await cdp.click('[data-testid="bot-settings"]')
    await wait(() => visible('[data-testid="bot-persona-form"]'), 'Bot avatar editor')
  }
  const save = async () => {
    await cdp.click('[data-testid="bot-save"]')
    await wait(async () => !(await visible('[data-testid="bot-persona-form"]')), 'avatar editor saved and closed')
  }
  const reload = async (session) => {
    const previous = await cdp.evaluate('performance.timeOrigin')
    await cdp.send('Page.reload')
    await wait(
      () =>
        cdp.evaluate(
          `performance.timeOrigin!==${previous} && window.__botProbe?.activeComposerSession()===${JSON.stringify(String(session))}`,
        ),
      'new document and Bot conversation after reload',
      90000,
    )
  }
  const chooseBot = async (id, session) => {
    await cdp.click(`[data-sidebar-bot-id="${id}"] [data-oc-list-item-primary]`)
    await wait(
      () => cdp.evaluate(`window.__botProbe.activeComposerSession()===${JSON.stringify(String(session))}`),
      'selected Bot conversation',
    )
  }
  const noOverflow = () =>
    cdp.evaluate(`document.documentElement.scrollWidth<=innerWidth+1 &&
    [...document.querySelectorAll('[role="dialog"]')].filter(n=>n.getClientRects().length)
      .every(n=>n.getBoundingClientRect().left>=-1 && n.getBoundingClientRect().right<=innerWidth+1)`)

  await cdp.send('Page.navigate', { url: base + '/bot?botId=' + botA })
  await wait(() => cdp.evaluate('window.__botProbe?.activeComposerSession()==="42"'), 'initial Bot conversation', 90000)
  await cdp.evaluate(`window.__botProbe.setAppLocale('en-US')`)
  await expectAvatar(header, null, 'existing Bot without an avatar uses the default')
  await expectAvatar(welcome, null, 'welcome uses the same default avatar')

  const fixtures = await cdp.evaluate(`(() => {
    const canvas=document.createElement('canvas'), ctx=canvas.getContext('2d');
    canvas.width=300;canvas.height=100;
    for(const [x,color] of [[0,'#dd3344'],[100,'#18b45c'],[200,'#2255cc']]){ctx.fillStyle=color;ctx.fillRect(x,0,100,100)}
    const png=canvas.toDataURL('image/png');
    canvas.width=100;canvas.height=300;
    for(const [y,color] of [[0,'#2255cc'],[100,'#ed9d27'],[200,'#18b45c']]){ctx.fillStyle=color;ctx.fillRect(0,y,100,100)}
    const jpeg=canvas.toDataURL('image/jpeg');
    ctx.fillStyle='#8054cf';ctx.fillRect(0,0,100,300);
    return {png,jpeg,webp:canvas.toDataURL('image/webp')};
  })()`)
  const files = {}
  for (const [format, source] of Object.entries(fixtures)) {
    files[format] = path.join(temporary, 'avatar.' + format)
    await fs.writeFile(files[format], Buffer.from(source.split(',')[1], 'base64'))
  }
  files.invalid = path.join(temporary, 'avatar.svg')
  await fs.writeFile(files.invalid, '<svg xmlns="http://www.w3.org/2000/svg"/>')
  files.large = path.join(temporary, 'too-large.png')
  await fs.writeFile(files.large, Buffer.alloc(5 * 1024 * 1024 + 1))
  files.pixels = path.join(temporary, 'too-many-pixels.png')
  const huge = Buffer.from(fixtures.png.split(',')[1], 'base64')
  huge.writeUInt32BE(8193, 16)
  await fs.writeFile(files.pixels, huge)

  await openEditor()
  const first = await upload(files.png)
  check(
    first.startsWith('data:image/png;base64,') && first.length <= 256 * 1024,
    'PNG upload becomes a bounded PNG thumbnail',
  )
  check(
    await cdp.evaluate(`(() => {
    const image=document.querySelector('${preview} img'), canvas=document.createElement('canvas');
    canvas.width=128;canvas.height=128;const ctx=canvas.getContext('2d');ctx.drawImage(image,0,0);
    return [20,100].every(p=>JSON.stringify([...ctx.getImageData(p,p,1,1).data])===JSON.stringify([24,180,92,255]));
  })()`),
    'landscape input is center-cropped, preserving the center color',
  )
  check(await noOverflow(), 'desktop avatar editor fits its dialog')
  await cdp.screenshot(path.join(evidence, 'web-bot-avatar-editor-desktop.png'))
  await save()
  check(profiles[0].avatar === first, 'avatar is persisted with the Bot profile')
  check(
    profileRequests.at(-1).body.avatar === first &&
      Object.keys(profileRequests.at(-1).body.persona).join(',') === 'instructions',
    'avatar is a top-level update, separate from persona',
  )
  await expectAvatar(row(botA), first, 'sidebar shows the saved avatar immediately')
  await expectAvatar(header, first, 'chat header shows the saved avatar immediately')
  await expectAvatar(welcome, first, 'welcome shows the saved avatar immediately')
  await cdp.evaluate("window.__botProbe.bots.open('manager')")
  await expectAvatar(`[data-bot-profile="${botA}"] [data-bot-avatar]`, first, 'Bot manager uses the saved avatar')
  await cdp.key('Escape', 'Escape', 27)
  await openEditor()
  await cdp.fill('[data-testid="bot-edit-name"]', 'Atlas Avatar')
  await save()
  check(
    !Object.hasOwn(profileRequests.at(-1).body, 'avatar') && profiles[0].avatar === first,
    'renaming leaves avatar omitted and preserves the saved image',
  )

  await openEditor()
  const second = await upload(files.jpeg)
  check(
    second !== first && second.startsWith('data:image/png;base64,'),
    'JPEG replacement is decoded and normalized to PNG',
  )
  for (const [file, message] of [
    [files.invalid, 'PNG, JPEG or WebP'],
    [files.large, '5 MiB'],
    [files.pixels, '8192'],
  ]) {
    await chooseFile(file)
    await wait(
      () =>
        cdp.evaluate(
          `document.querySelector('[data-testid="bot-avatar-error"]')?.textContent.includes(${JSON.stringify(message)})`,
        ),
      'specific avatar validation error',
    )
    check(await matches(preview, second), 'invalid replacement preserves the valid draft: ' + path.basename(file))
  }
  await save()
  check(profiles[0].avatar === second, 'valid draft is retained after a failed replacement and can be saved')
  await expectAvatar(header, second, 'replacement reaches the active conversation')
  const mutationsBeforeReload = profileRequests.length
  await reload(42)
  await cdp.evaluate(`window.__botProbe.setAppLocale('en-US')`)
  await expectAvatar(header, second, 'avatar survives a full page reload')
  check(profileRequests.length === mutationsBeforeReload, 'loading a saved avatar makes no profile writes')

  // Exercise a genuinely delayed profile GET: cancelling a reload must release
  // the busy state and stop its response from reopening or overwriting a form.
  await openEditor()
  const third = await upload(files.webp)
  profiles[0].revision++
  await cdp.click('[data-testid="bot-save"]')
  await wait(
    () =>
      cdp.evaluate(
        `document.querySelector('[data-testid="bot-persona-form"]')?.innerText.includes('Revision conflict')`,
      ),
    'revision conflict',
  )
  check(await matches(preview, third), 'save conflict retains the selected avatar draft')
  avatarReads.holdFor = botA
  await cdp.clickText('[role="dialog"] button', 'Reload saved profile')
  await wait(() => avatarReads.release, 'held editor reload GET')
  await cdp.clickText('[role="dialog"] button', 'Cancel')
  await wait(async () => !(await visible('[data-testid="bot-persona-form"]')), 'cancel held profile reload')
  await cdp.click('[data-testid="sidebar-bot-create"]')
  await wait(() => visible('[data-testid="bot-edit-name"]'), 'create after cancelling reload')
  await cdp.fill('[data-testid="bot-edit-name"]', 'Avatar Nova')
  avatarReads.release()
  await wait(() => !avatarReads.release, 'late profile response released')
  await frame()
  check(
    (await cdp.evaluate(`document.querySelector('[data-testid="bot-edit-name"]')?.value==='Avatar Nova'`)) &&
      (await matches(preview, null)),
    'cancelled profile reload cannot overwrite a new editor or its default avatar',
  )

  const createdAvatar = await upload(files.webp)
  check(createdAvatar === third, 'WebP upload uses the same normalization during create and edit')
  await save()
  await wait(() => cdp.evaluate('window.__botProbe.activeComposerSession()==="44"'), 'new Bot conversation')
  check(profiles.find((p) => p.id === botC)?.avatar === createdAvatar, 'new Bot creation persists its chosen avatar')
  await expectAvatar(header, createdAvatar, 'new Bot header shows its avatar')
  await expectAvatar(welcome, createdAvatar, 'new Bot welcome shows its avatar')
  await chooseBot(botB, 43)
  await expectAvatar(header, null, 'switching to another Bot clears the previous avatar')
  await chooseBot(botC, 44)
  await expectAvatar(header, createdAvatar, 'switching back restores that Bot’s avatar')

  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 375, height: 812, deviceScaleFactor: 1, mobile: true })
  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 })
  await wait(() => cdp.evaluate('window.__botProbe.ui.isCompactLayout'), 'mobile layout')
  await expectAvatar(header, createdAvatar, 'mobile chat renders the saved avatar')
  check(await noOverflow(), '375px avatar conversation fits the viewport')
  await cdp.screenshot(path.join(evidence, 'web-bot-avatar-chat-mobile.png'))
  await openEditor()
  await expectAvatar(preview, createdAvatar, 'mobile editor loads the existing avatar')
  check(await noOverflow(), '375px avatar editor and actions fit the viewport')
  await cdp.screenshot(path.join(evidence, 'web-bot-avatar-editor-mobile.png'))
  await cdp.click('[data-testid="bot-avatar-remove"]')
  await expectAvatar(preview, null, 'removing an avatar previews the default before saving')
  await save()
  check(
    profileRequests.at(-1).body.avatar === null && profiles.find((p) => p.id === botC).avatar === null,
    'removing an avatar sends explicit null and persists removal',
  )
  await expectAvatar(header, null, 'removal updates the active header')
  await expectAvatar(welcome, null, 'removal updates the welcome')
  await reload(44)
  await expectAvatar(header, null, 'removed avatar remains the default after reload')
}
