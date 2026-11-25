// Prompt and settings list behavior against the real components with
// same-origin API fixtures only. The shared runner owns Vite and Chromium and
// exposes openPage(url, { viewport, narrow, api, storage }).
const assert = require('node:assert/strict')
const { channelAssignmentsApi, promptListsApi, sessionStorageApi } = require('./fixtures/workbench-list-reuse-api.cjs')

const phoneViewport = { width: 390, height: 844 }

const routes = {
  systemPrompt: (query = '') => `/__workbench-lists?case=system-prompt${query}`,
  botPrompt: (query = '') => `/__workbench-lists?case=bot-prompt${query}`,
  promptReference: (query = '') => `/__workbench-lists?case=prompt-reference${query}`,
  sessionStorage: (query = '') => `/__workbench-lists?case=session-storage${query}`,
  channelAssignments: (query = '') => `/__workbench-lists?case=channel-assignments${query}`,
}

async function runListChecks(openPage) {
  assert.equal(typeof openPage, 'function', 'runListChecks needs the shared openPage runner')
  const results = []
  const check = (condition, description) => {
    assert(condition, description)
    results.push(description)
    console.log(`PASS ${description}`)
  }
  // Factories resolve true when they fulfilled the request. Anything else is
  // surfaced as a 404 and listed so a missing route never hangs the probe.
  const unhandled = []
  const api = (fixture) => async (route, url) => {
    const handled = await fixture(route, url)
    if (handled === true) return
    unhandled.push(`${route.request().method()} ${url.pathname}${url.search}`)
    await route.fulfill({ status: 404, json: { error: `Unhandled list fixture request: ${url.pathname}` } })
  }

  await checkSystemPrompt(openPage, check, api)
  await checkBotPrompt(openPage, check, api)
  await checkPromptReference(openPage, check)
  await checkSessionStorage(openPage, check, api)
  await checkChannelAssignments(openPage, check, api)
  assert.deepEqual(unhandled, [], 'every request made by the fixtures was handled')
  return results
}

function focused(page, locator) {
  return locator.and(page.locator(':focus')).waitFor({ state: 'attached' })
}

async function press(page, key) {
  await page.keyboard.press(key)
}

async function until(predicate, description, timeout = 5000) {
  const start = Date.now()
  while (Date.now() - start < timeout) {
    if (await predicate()) return
    await new Promise((resolve) => setTimeout(resolve, 40))
  }
  throw new Error(`Timed out waiting for ${description}`)
}

/* -------------------------------------------------------------- system prompt */

async function checkSystemPrompt(openPage, check, api) {
  const fixture = promptListsApi()
  // Two required built-ins and two library prompts in one layer exercise
  // locking, removal, and reordering together.
  fixture.state.global.order = ['agena.system.identity', 'agena.system.evidence', 'saved.alpha', 'saved.beta']
  const page = await openPage(routes.systemPrompt(), { api: api(fixture) })
  try {
    const rows = page.locator('[data-system-prompt-entry]')
    await rows.first().waitFor()
    assert.equal(await rows.count(), 4, 'required builtins plus the two selected prompts')

    const identity = page.locator('[data-system-prompt-entry="agena.system.identity"]')
    const evidence = page.locator('[data-system-prompt-entry="agena.system.evidence"]')
    check(
      (await identity.locator('[aria-label="Required built-in prompt"]').count()) === 1 &&
        (await identity.locator('[aria-label="Remove"]').count()) === 0,
      'required built-ins show a lock instead of Remove',
    )
    check(
      (await page.locator('[data-system-prompt-entry="saved.alpha"] [aria-label="Remove"]').count()) === 1,
      'selected library prompts keep their Remove action',
    )
    check(
      (await identity.locator('[data-keyboard-primary]').getAttribute('aria-pressed')) === 'true',
      'the initially selected prompt is pressed',
    )

    const heading = page.locator('[data-settings-detail-heading]')
    check(
      (await heading.innerText()).trim() === 'Assistant identity',
      'the initial selection fills the detail without stealing focus',
    )
    await identity.locator('[data-keyboard-primary]').focus()
    await press(page, 'ArrowDown')
    await focused(page, evidence.locator('[data-keyboard-primary]'))
    check(
      (await evidence.locator('[data-keyboard-primary]').getAttribute('aria-pressed')) === 'false',
      'arrow navigation does not press the row it focuses',
    )
    check(
      (await heading.innerText()).trim() === 'Assistant identity',
      'arrow navigation does not change the inspected prompt',
    )

    await evidence.locator('[data-keyboard-primary]').click()
    await focused(page, heading)
    check(
      (await heading.innerText()).trim() === 'Runtime facts and evidence',
      'clicking a row opens and focuses its detail heading',
    )

    // Move down keeps keyboard navigation on the row that moved.
    await page.locator('[data-system-prompt-entry="saved.alpha"] [aria-label="Move down"]').click()
    await focused(page, page.locator('[data-system-prompt-entry="saved.alpha"] [data-keyboard-primary]'))
    check(true, 'Move down keeps focus on the moved prompt row')

    await page.getByRole('button', { name: 'Save changes', exact: true }).click()
    await until(() => fixture.writes.length === 1, 'the prompt layer write')
    await page.getByRole('button', { name: 'Save changes', exact: true }).waitFor({ state: 'detached' })
    const write = fixture.writes[0]
    check(
      write.path === 'prompts' && write.value.order.indexOf('saved.beta') < write.value.order.indexOf('saved.alpha'),
      'saving writes the reordered prompt ids to the global layer',
    )

    await page.locator('[data-system-prompt-entry="saved.beta"] [aria-label="Remove"]').click()
    await page.waitForFunction(() => !document.querySelector('[data-system-prompt-entry="saved.beta"]'))
    const removal = await page.evaluate(() => ({
      tag: document.activeElement?.tagName,
      inCollection: Boolean(document.activeElement?.closest('[data-keyboard-collection]')),
    }))
    check(removal.tag !== 'BODY' && removal.inCollection, 'removing a prompt keeps focus inside the prompt list')
  } finally {
    await page.close()
  }

  // Stacked layout: arrows do not reveal the detail, a click does, Back returns.
  const narrowFixture = promptListsApi()
  narrowFixture.state.global.order = ['agena.system.identity', 'agena.system.evidence', 'saved.alpha', 'saved.beta']
  const narrow = await openPage(routes.systemPrompt(), {
    narrow: true,
    viewport: phoneViewport,
    api: api(narrowFixture),
  })
  try {
    const detail = narrow.locator('[data-settings-detail]')
    check((await detail.isVisible()) === false, 'the stacked detail pane starts closed')
    const first = narrow.locator('[data-system-prompt-entry="agena.system.identity"] [data-keyboard-primary]')
    await first.focus()
    await press(narrow, 'ArrowDown')
    await focused(narrow, narrow.locator('[data-system-prompt-entry="agena.system.evidence"] [data-keyboard-primary]'))
    check((await detail.isVisible()) === false, 'arrow navigation never opens the stacked detail pane')

    await narrow.locator('[data-system-prompt-entry="saved.alpha"] [data-keyboard-primary]').click()
    await focused(narrow, narrow.locator('[data-settings-detail-heading]'))
    check((await detail.isVisible()) === true, 'clicking a prompt opens the stacked detail pane')

    await narrow.getByRole('button', { name: 'Prompt order', exact: true }).click()
    await focused(narrow, narrow.locator('[data-system-prompt-entry="saved.alpha"] [data-keyboard-primary]'))
    check(true, 'stacked Back returns focus to the prompt that opened the detail')
  } finally {
    await narrow.close()
  }
}

/* ---------------------------------------------------------------- bot prompt */

async function checkBotPrompt(openPage, check, api) {
  const fixture = promptListsApi()
  const page = await openPage(routes.botPrompt(), { api: api(fixture) })
  try {
    const editor = page.locator('[data-testid="bot-prompt-order"]')
    await editor.locator('[data-bot-prompt-id]').first().waitFor()
    const instructions = editor.locator('[data-bot-prompt-id="agena.bot.instructions"]')
    check((await instructions.count()) === 1, 'a non-empty instruction draft appears in the prompt order')
    check(
      (await instructions.locator('[aria-label="Required built-in prompt"]').count()) === 0,
      'the instruction row stays removable',
    )

    await editor.locator('[data-bot-prompt-id="saved.alpha"] [data-keyboard-primary]').focus()
    await press(page, 'ArrowDown')
    await focused(page, editor.locator('[data-bot-prompt-id="saved.beta"] [data-keyboard-primary]'))
    check(
      (await editor
        .locator('[data-bot-prompt-id="saved.beta"] [data-keyboard-primary]')
        .getAttribute('aria-expanded')) === 'false',
      'arrow navigation does not expand the focused Bot prompt',
    )

    await editor.locator('[data-bot-prompt-id="saved.alpha"] [aria-label="Move down"]').click()
    await focused(page, editor.locator('[data-bot-prompt-id="saved.alpha"] [data-keyboard-primary]'))
    check(true, 'bot prompt Move down keeps focus on the moved row')

    const persona = JSON.parse(await page.locator('#bot-persona').innerText())
    check(
      persona.prompts.ids.indexOf('saved.beta') < persona.prompts.ids.indexOf('saved.alpha'),
      'the Bot draft order mirrors the visible move',
    )
    const required = editor.locator('[data-bot-prompt-id="agena.system.identity"]')
    check(
      persona.prompts.ids.includes('agena.system.identity') &&
        (await required.locator('[aria-label="Required built-in prompt"]').count()) === 1 &&
        (await required.locator('[aria-label="Remove"]').count()) === 0,
      'reordering preserves the required built-in and its lock',
    )

    await editor.getByTestId('bot-add-prompt').locator('button').press('Enter')
    const picker = page.getByRole('dialog', { name: 'Prompt library', exact: true })
    await picker.locator('[data-option-menu-search]').fill('Verification and evidence')
    await picker.locator('[data-option-menu-item]').filter({ hasText: 'Verification and evidence' }).press('Enter')
    await picker.waitFor({ state: 'hidden' })
    const reusable = editor.locator('[data-bot-prompt-id="agena.profile.verification"]')
    await reusable.locator('[aria-label="Remove"]').waitFor()
    check(
      JSON.parse(await page.locator('#bot-persona').innerText()).prompts.ids.includes('agena.profile.verification'),
      'search can add a reusable built-in to the Bot draft without locking the row',
    )
  } finally {
    await page.close()
  }

  // Empty instructions: adding them creates the row and focuses the editor.
  const emptyFixture = promptListsApi()
  const empty = await openPage(routes.botPrompt('&empty-instructions'), { api: api(emptyFixture) })
  try {
    const editor = empty.locator('[data-testid="bot-prompt-order"]')
    check(
      (await editor.locator('[data-bot-prompt-id="agena.bot.instructions"]').count()) === 0,
      'an empty instruction draft has no instruction row',
    )
    await editor.getByRole('button', { name: 'Additional system instructions (optional)', exact: true }).click()
    const textarea = empty.locator('[data-testid="bot-edit-instructions"]')
    await focused(empty, textarea)
    await textarea.fill('Prefer short answers.')
    check(
      (await editor.locator('[data-bot-prompt-id="agena.bot.instructions"]').count()) === 1,
      'typing instructions adds the instruction prompt row',
    )
    const persona = JSON.parse(await empty.locator('#bot-persona').innerText())
    check(persona.instructions === 'Prefer short answers.', 'the textarea updates the Bot persona instructions')
    check(
      (await empty.locator('[data-testid="bot-instructions-count"]').innerText()).includes(
        String(Buffer.byteLength('Prefer short answers.')),
      ),
      'the instruction byte counter follows the textarea',
    )

    await editor.getByRole('button', { name: 'Close', exact: true }).click()
    await textarea.waitFor({ state: 'detached' })
    const instructionRow = editor.locator('[data-bot-prompt-id="agena.bot.instructions"] [data-keyboard-primary]')
    await instructionRow.click()
    await textarea.waitFor()
    check(
      (await instructionRow.getAttribute('aria-expanded')) === 'true',
      'clicking the instruction row reopens its inline editor without stealing focus from the row',
    )
  } finally {
    await empty.close()
  }
}

/* ----------------------------------------------------------- prompt reference */

async function checkPromptReference(openPage, check) {
  const page = await openPage(routes.promptReference('&ids=alpha'), {})
  try {
    const ids = () => page.locator('#reference-json').innerText()
    assert.equal(await page.locator('[data-keyboard-item]').count(), 1)

    const addTrigger = page.locator('[data-prompt-reference-add] button[aria-haspopup="dialog"]')
    await addTrigger.click()
    const picker = page.getByRole('dialog', { name: 'Select a saved prompt', exact: true })
    await picker.waitFor()
    check(
      (await picker.getByText('Guides', { exact: true }).count()) === 1 &&
        (await picker.getByText('Runtime', { exact: true }).count()) === 1 &&
        (await picker.getByText('Uncategorized', { exact: true }).count()) === 1,
      'the picker groups options by prompt category with an Uncategorized fallback',
    )

    const search = picker.locator('[data-option-menu-search]')
    await focused(page, search)
    check(
      (await search.getAttribute('aria-label')) === 'Search...',
      'PromptReferenceList keeps the generic picker search label',
    )
    await search.fill('gamma')
    const options = picker.locator('[data-option-menu-item]')
    await options.first().waitFor()
    check(
      (await options.count()) === 1 && (await options.first().innerText()).includes('Gamma prompt'),
      'search filters the grouped options to the matching prompt',
    )
    check(
      (await picker.getByText('Runtime', { exact: true }).count()) === 1 &&
        (await picker.getByText('Guides', { exact: true }).count()) === 0,
      'filtering drops group headers with no remaining options',
    )

    await options.first().click()
    await page.waitForFunction(() => document.getElementById('reference-json').textContent.includes('gamma'))
    check((await ids()) === '["alpha","gamma"]', 'choosing a prompt appends its id')
    await focused(page, addTrigger)
    check(true, 'closing the picker returns focus to the surviving add trigger')

    await page.locator('[data-keyboard-item="gamma"] [aria-label="Move up"]').click()
    await focused(page, page.locator('[data-keyboard-item="gamma"] [data-keyboard-primary]'))
    check((await ids()) === '["gamma","alpha"]', 'Move up swaps the saved ids and keeps focus on the moved row')

    await page.locator('[data-keyboard-item="gamma"] [data-keyboard-primary]').focus()
    await press(page, 'ArrowDown')
    await focused(page, page.locator('[data-keyboard-item="alpha"] [data-keyboard-primary]'))
    check((await ids()) === '["gamma","alpha"]', 'arrow navigation changes focus without reordering ids')

    await page.locator('[data-keyboard-item="alpha"] [aria-label="Remove"]').click()
    await page.waitForFunction(() => document.getElementById('reference-json').textContent === '["gamma"]')
    const removal = await page.evaluate(() => ({
      tag: document.activeElement?.tagName,
      inList: Boolean(document.activeElement?.closest('[data-keyboard-collection]')),
    }))
    check(removal.tag !== 'BODY' && removal.inList, 'removing a reference keeps focus inside the list')
  } finally {
    await page.close()
  }

  const limited = await openPage(routes.promptReference('&ids=alpha&limit=2'), {})
  try {
    const addTrigger = limited.locator('[data-prompt-reference-add] button[aria-haspopup="dialog"]')
    await addTrigger.click()
    const picker = limited.getByRole('dialog', { name: 'Select a saved prompt', exact: true })
    await picker.locator('[data-option-menu-item]').filter({ hasText: 'Beta prompt' }).click()
    await limited.waitForFunction(() => document.getElementById('reference-json').textContent === '["alpha","beta"]')
    check((await addTrigger.isDisabled()) === true, 'the add trigger disables once the configured limit is reached')
    await until(
      async () =>
        limited.evaluate(() => {
          const active = document.activeElement
          return Boolean(
            active && active.matches('[data-keyboard-primary]') && active.closest('[data-keyboard-collection]'),
          )
        }),
      'focus to fall back to a row when the trigger disabled',
    )
    check(true, 'a picker whose trigger disabled returns focus to the prompt list')
  } finally {
    await limited.close()
  }
}

/* -------------------------------------------------------------- session storage */

async function checkSessionStorage(openPage, check, api) {
  const fixture = sessionStorageApi()
  const page = await openPage(routes.sessionStorage(), { api: api(fixture) })
  try {
    const rows = page.locator('[data-keyboard-item]')
    await rows.first().waitFor()
    check(
      fixture.state.listRequests[0].query.offset === '0' && fixture.state.listRequests[0].query.limit === '20',
      'the first inventory page requests offset 0 and limit 20',
    )
    check((await rows.first().getAttribute('aria-pressed')) === 'true', 'the first session is selected on load')

    await rows.first().focus()
    await press(page, 'ArrowDown')
    await focused(page, rows.nth(1))
    check(
      (await rows.nth(1).getAttribute('aria-pressed')) === 'false',
      'arrow navigation focuses the next session without selecting it',
    )

    await page.locator('[data-keyboard-item="102"]').click()
    await page.locator('[data-settings-detail-heading]').waitFor()
    check(
      (await page.locator('[data-settings-detail-heading]').innerText()).trim() === 'Session 02',
      'clicking a session renders its storage detail',
    )
    const detailRequest = fixture.state.detailRequests.at(-1)
    check(
      detailRequest.query.file_offset === '0' && detailRequest.query.file_limit === '15',
      'opening a session requests the first 15 files',
    )

    await page.getByRole('button', { name: 'Next page', exact: true }).click()
    await until(() => fixture.state.listRequests.at(-1).query.offset === '20', 'the second inventory page')
    await page.waitForFunction(() => document.querySelector('[data-keyboard-item="121"]'))
    check(true, 'the next page requests a server offset of 20')

    // Search resets to the first page and forwards the debounced term.
    const find = page.getByRole('textbox', { name: 'Find a session', exact: true })
    await find.fill('Session 1')
    await until(
      () =>
        fixture.state.listRequests.at(-1).query.search === 'Session 1' &&
        fixture.state.listRequests.at(-1).query.offset === '0',
      'the debounced search request',
    )
    check(true, 'search debounces, forwards the term, and resets the offset')
    await find.fill('')
    await until(() => !fixture.state.listRequests.at(-1).query.search, 'the cleared search request')

    await page.locator('[data-keyboard-item="101"]').click()
    const files = page.locator('[data-settings-detail] details').filter({ hasText: 'Managed files (17)' })
    await files.locator('summary').click()
    await files.getByText('artifact-01.bin', { exact: true }).waitFor()
    await files.getByRole('button', { name: 'Next page', exact: true }).click()
    await files.getByText('artifact-16.bin', { exact: true }).waitFor()
    const filePage = fixture.state.detailRequests.at(-1)
    check(
      filePage.pathname === '/api/v1/sessions/101/storage' &&
        filePage.query.file_offset === '15' &&
        filePage.query.file_limit === '15',
      'the next managed-file page requests offset 15 and renders its returned files',
    )

    await page.getByRole('button', { name: 'Preview cleanup', exact: true }).click()
    const dialog = page.getByRole('dialog', { name: 'Clean session storage', exact: true })
    await dialog.waitFor()
    const confirm = dialog.getByRole('button', { name: 'Clean selected items', exact: true })
    await confirm.waitFor()
    await until(
      () =>
        fixture.requests.some(
          (entry) => entry.method === 'POST' && entry.pathname === '/api/v1/sessions/101/storage/preview',
        ),
      'the cleanup preview request',
    )
    check(true, 'Preview cleanup asks the server to price the selection')

    await confirm.click()
    await dialog.waitFor({ state: 'detached' })
    const cleanup = fixture.state.cleanups.at(-1)
    check(
      cleanup.sessionId === 101 &&
        cleanup.selection.cache === true &&
        cleanup.selection.recordings === false &&
        cleanup.token === 'cleanup-token-cache',
      'cleanup posts the chosen selection with the exact preview token',
    )
    await page.getByText('Cleanup completed', { exact: true }).waitFor()
    check(
      fixture.requests.some(
        (entry) => entry.method === 'GET' && entry.pathname === '/api/v1/sessions/101/storage/operation',
      ),
      'the detail pane shows cleanup completion after polling the session operation',
    )
  } finally {
    await page.close()
  }

  // Deep link opens the detail and stacked Back restores list focus.
  const linkedFixture = sessionStorageApi()
  const narrow = await openPage(routes.sessionStorage('&target=102'), {
    narrow: true,
    viewport: phoneViewport,
    api: api(linkedFixture),
  })
  try {
    await focused(narrow, narrow.locator('[data-settings-detail-heading]'))
    check(
      (await narrow.locator('[data-settings-detail-heading]').innerText()).trim() === 'Session 02',
      'a deep-linked session opens its detail focused',
    )
    await narrow.getByRole('button', { name: 'Sessions', exact: true }).click()
    // Nothing in the list was focused before the deep link, so Back lands on
    // the list region's heading rather than an arbitrary row.
    await focused(narrow, narrow.locator('[data-settings-list-heading]'))
    check(
      (await narrow.locator('aside[aria-label="Sessions"]').isVisible()) === true,
      'stacked Back returns focus to the session list heading',
    )
  } finally {
    await narrow.close()
  }
}

/* --------------------------------------------------------- channel assignments */

async function checkChannelAssignments(openPage, check, api) {
  const fixture = channelAssignmentsApi()
  const page = await openPage(routes.channelAssignments(), { api: api(fixture) })
  try {
    const rooms = page.locator('[data-assignment-room]')
    await rooms.first().waitFor()
    check((await rooms.count()) === 3, 'the platform inventory renders one row per conversation')
    // On desktop the side-by-side detail is always rendered; selection only
    // decides whether the stacked pane is opened.
    check(
      (await rooms.first().getAttribute('aria-pressed')) === 'true' &&
        (await page.evaluate(() => !document.activeElement?.closest('[data-settings-detail]'))),
      'the first room is selected on load without moving focus into the detail',
    )

    await rooms.first().focus()
    await press(page, 'ArrowDown')
    await focused(page, rooms.nth(1))
    check(
      (await rooms.nth(1).getAttribute('aria-pressed')) === 'false',
      'arrow navigation focuses the next room without selecting it',
    )

    const roomSearch = page.getByRole('textbox', { name: 'Search chats by name or ID', exact: true })
    await roomSearch.fill('Beta')
    check((await page.locator('[data-assignment-room]').count()) === 1, 'room search filters the conversation list')
    await roomSearch.fill('')

    await page.locator('[data-assignment-room="room-alpha"]').click()
    const heading = page.locator('[data-settings-detail-heading]')
    await heading.waitFor()
    check((await heading.innerText()).trim() === 'Alpha room', 'clicking a room opens its detail heading')

    const member = (actor) => page.locator(`[data-assignment-member="${actor}"]`)
    await member('ada').waitFor()
    check(
      (await member('ada').isChecked()) === false,
      'member selection starts empty even when the catalog already assigns a profile',
    )

    await page.getByRole('button', { name: 'Load more participants', exact: true }).click()
    await member('cleo').waitFor()
    check(
      fixture.requests.some(
        (entry) =>
          entry.pathname.endsWith('/participants') &&
          entry.query.after === 'alpha-page-2' &&
          entry.query.resource === 'room-alpha',
      ),
      'Load more participants sends the server cursor',
    )

    await member('bob').check()
    await page.locator('[data-assignment-member-profile="bob"]').selectOption('operator')
    await page.locator('[data-assignment-save]').getByRole('button', { name: 'Save changes', exact: true }).click()
    await until(() => fixture.writes.length === 1, 'the access_control write')
    const write = fixture.writes[0]
    check(
      write.path === 'access_control' &&
        write.value.users.some(
          (user) => user.actor === 'bob' && user.resource === 'room-alpha' && user.profile_id === 'operator',
        ),
      'saving adds the selected member profile to the catalog',
    )
    await page.getByText('Saved', { exact: true }).waitFor()
    check(true, 'the panel reports the saved state')
  } finally {
    await page.close()
  }

  // Deep link: room, source, and actor seed the detail and member search.
  const linkedFixture = channelAssignmentsApi()
  const linked = await openPage(routes.channelAssignments('&room=room-beta&source=5&actor=dana'), {
    api: api(linkedFixture),
  })
  try {
    const heading = linked.locator('[data-settings-detail-heading]')
    await heading.waitFor()
    check((await heading.innerText()).trim() === 'Beta DM', 'the deep-linked room opens its detail')
    const search = linked.getByRole('textbox', { name: 'Search members by name or ID', exact: true })
    check((await search.inputValue()) === 'dana', 'the deep link seeds the member search with the actor')
    await linked.locator('[data-assignment-member="dana"]').waitFor()
    check(
      (await linked.locator('[data-assignment-member="dana"]').isChecked()) === true,
      'the deep link selects the actor',
    )
  } finally {
    await linked.close()
  }

  // Missing room reports a status and keeps the stacked detail closed even
  // though the list falls back to selecting its first room.
  const missingFixture = channelAssignmentsApi()
  const missing = await openPage(routes.channelAssignments('&room=room-unknown&source=5'), {
    narrow: true,
    viewport: phoneViewport,
    api: api(missingFixture),
  })
  try {
    await missing.getByText('The requested conversation could not be found.', { exact: true }).waitFor()
    check(
      (await missing.locator('[data-settings-detail]').isVisible()) === false,
      'a missing deep-linked room reports a status without opening the detail pane',
    )
    check(
      (await missing.locator('[data-assignment-room="room-alpha"]').getAttribute('aria-pressed')) === 'true',
      'the room list still falls back to its first room after a missing deep link',
    )
  } finally {
    await missing.close()
  }

  // Narrow: Back restores the room button.
  const narrowFixture = channelAssignmentsApi()
  const narrow = await openPage(routes.channelAssignments(), {
    narrow: true,
    viewport: phoneViewport,
    api: api(narrowFixture),
  })
  try {
    const room = narrow.locator('[data-assignment-room="room-alpha"]')
    await room.click()
    await focused(narrow, narrow.locator('[data-settings-detail-heading]'))
    await narrow.getByRole('button', { name: 'Chats and channels', exact: true }).click()
    await focused(narrow, room)
    check(true, 'stacked Back returns focus to the room button')
  } finally {
    await narrow.close()
  }
}

module.exports = { runListChecks }
