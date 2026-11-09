import { describe, expect, test } from 'bun:test'
import { SaveOwnership } from '../src/features/bots/saveOwnership'

describe('pending Bot saves', () => {
  test('cancellation prevents navigation even if the remote request later succeeds', async () => {
    const save = new SaveOwnership(1, '/project', 'window-a')
    let complete!: () => void
    const remote = new Promise<void>((resolve) => {
      complete = resolve
    })
    const result = remote.then(() => save.canNavigate(1, '/project', 'window-a'))
    save.cancel()
    complete()
    expect(await result).toBe(false)
  })
  test('leaving and returning to the same route invalidates navigation ownership', () => {
    const save = new SaveOwnership(1, '/project', 'window-a')
    expect(save.canNavigate(1, '/project', 'window-a')).toBe(true)
    expect(save.canNavigate(2, '/project', 'window-a')).toBe(false)
    expect(save.canNavigate(1, '/project', 'window-b')).toBe(false)
  })
})
