import { describe, expect, test } from 'bun:test'
import { BotAvatarError } from '../src/features/bots/avatar'
import { useBotAvatarDraft } from '../src/features/bots/avatarDraft'
import { botDraftFields } from '../src/features/bots/draft'

const first = 'data:image/png;base64,AA=='
const second = 'data:image/png;base64,AQ=='
const file = new File(['image'], 'avatar.png', { type: 'image/png' })
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}

describe('Bot avatar draft ownership', () => {
  test('untouched renames omit avatar; replace and clear are explicit top-level changes', async () => {
    const draft = useBotAvatarDraft(async () => second)
    draft.reset(first)
    expect(draft.avatar.value).toBe(first)
    expect(botDraftFields('Renamed', '', draft.change.value)).not.toHaveProperty('avatar')
    await draft.upload(file)
    expect(botDraftFields('Renamed', '', draft.change.value)).toEqual({
      name: 'Renamed',
      persona: { instructions: '' },
      avatar: second,
    })
    draft.remove()
    expect(botDraftFields('Renamed', '', draft.change.value).avatar).toBeNull()
    draft.reset()
    await draft.upload(file)
    draft.remove()
    expect(botDraftFields('New', '', draft.change.value)).not.toHaveProperty('avatar')
  })
  test('returning to the saved image produces no avatar update', async () => {
    const draft = useBotAvatarDraft(async () => first)
    draft.reset(first)
    draft.remove()
    expect(draft.change.value).toBeNull()
    await draft.upload(file)
    expect(draft.avatar.value).toBe(first)
    expect(draft.change.value).toBeUndefined()
  })
  test('invalid replacement preserves the current preview and pending save value', async () => {
    let fail = false
    const draft = useBotAvatarDraft(async () => {
      if (fail) throw new BotAvatarError('unsupported')
      return second
    })
    draft.reset(first)
    await draft.upload(file)
    fail = true
    await draft.upload(file)
    expect(draft.avatar.value).toBe(second)
    expect(draft.change.value).toBe(second)
    expect(draft.error.value).toBe('unsupported')
    expect(draft.loading.value).toBe(false)
  })
  test('a late file read cannot replace another editor, including after errors', async () => {
    const pending = deferred<string>()
    let signal: AbortSignal | undefined
    const draft = useBotAvatarDraft((_file, currentSignal) => {
      signal = currentSignal
      return pending.promise
    })
    draft.reset(first)
    const uploading = draft.upload(file)
    expect(draft.loading.value).toBe(true)
    draft.reset(second)
    expect(signal?.aborted).toBe(true)
    pending.resolve(first)
    await uploading
    expect(draft.avatar.value).toBe(second)
    expect(draft.change.value).toBeUndefined()
    expect(draft.loading.value).toBe(false)
    expect(draft.error.value).toBeNull()

    const failure = deferred<string>()
    const failing = useBotAvatarDraft(() => failure.promise)
    const old = failing.upload(file)
    failing.reset(second)
    failure.reject(new BotAvatarError('read'))
    await old
    expect(failing.error.value).toBeNull()
    expect(failing.avatar.value).toBe(second)
  })
  test('cancelling or removing aborts an upload and ignores its eventual result', async () => {
    for (const action of ['cancel', 'remove'] as const) {
      const pending = deferred<string>()
      const draft = useBotAvatarDraft(() => pending.promise)
      draft.reset(first)
      const upload = draft.upload(file)
      draft[action]()
      pending.resolve(second)
      await upload
      expect(draft.avatar.value).toBe(action === 'remove' ? null : first)
      expect(draft.change.value).toBe(action === 'remove' ? null : undefined)
      expect(draft.loading.value).toBe(false)
    }
  })
  test('a newer selection wins and an older completion cannot stop its loading state', async () => {
    const older = deferred<string>()
    const newer = deferred<string>()
    let call = 0
    const draft = useBotAvatarDraft(() => (++call === 1 ? older.promise : newer.promise))
    const firstUpload = draft.upload(file)
    const nextUpload = draft.upload(file)
    older.resolve(first)
    await firstUpload
    expect(draft.loading.value).toBe(true)
    expect(draft.avatar.value).toBeNull()
    newer.resolve(second)
    await nextUpload
    expect(draft.avatar.value).toBe(second)
    expect(draft.loading.value).toBe(false)
  })
})
