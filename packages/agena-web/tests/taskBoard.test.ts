import { describe, expect, test } from 'bun:test'
import {
  TaskBoardController,
  newTaskBoardState,
  newTaskForm,
  validGoal,
  canResumeTask,
} from '../src/features/tasks/taskBoard'
import type { Task, TaskApi } from '../src/lib/taskApi'

function task(id: string, source = 7, revision = 1): Task {
  return {
    id,
    source_session_id: source,
    workspace_id: 1,
    request_key: `request-${id}`,
    goal: '测试工作',
    instruction: '测试工作',
    state: 'review',
    revision,
    attempt: 1,
    worker_session_id: 11,
    execution_id: null,
    attempt_generation: null,
    resume_key: null,
    resume_from_revision: null,
    note: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
}
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void
  const promise = new Promise<T>((a, b) => {
    resolve = a
    reject = b
  })
  return { promise, resolve, reject }
}
function api(overrides: Partial<TaskApi> = {}): TaskApi {
  return {
    list: async () => [],
    get: async (id) => task(id),
    create: async () => task('created'),
    resume: async (id) => task(id),
    cancel: async (id) => task(id),
    accept: async (id) => task(id),
    ...overrides,
  }
}

describe('durable task board', () => {
  test('a response from the old conversation cannot populate the new one', async () => {
    const old = deferred<Task[]>(),
      next = deferred<Task[]>()
    const state = newTaskBoardState(7)
    const board = new TaskBoardController(
      state,
      api({ list: (query) => (query.source_session_id === 7 ? old.promise : next.promise) }),
    )
    const first = board.refresh()
    board.setScope(8)
    const second = board.refresh()
    next.resolve([task('new', 8)])
    await second
    old.resolve([task('old', 7)])
    await first
    expect(state.items.map((item) => item.id)).toEqual(['new'])
    expect(state.loading).toBe(false)
  })

  test('out-of-order detail reads cannot change the selected task', async () => {
    const a = deferred<Task>(),
      b = deferred<Task>()
    const state = newTaskBoardState(7)
    const board = new TaskBoardController(state, api({ get: (id) => (id === 'a' ? a.promise : b.promise) }))
    const first = board.select('a'),
      second = board.select('b')
    b.resolve(task('b'))
    await second
    a.resolve(task('a'))
    await first
    expect(state.selected?.id).toBe('b')
  })

  test('an older list snapshot cannot roll back a successful action', async () => {
    const list = deferred<Task[]>(),
      state = newTaskBoardState(7)
    const board = new TaskBoardController(state, api({ list: () => list.promise }))
    const read = board.refresh()
    board.update(task('a', 7, 9))
    list.resolve([task('a', 7, 3)])
    await read
    expect(state.selected?.revision).toBe(9)
    expect(state.items[0]?.revision).toBe(9)
  })

  test('network failure keeps the exact request and prevents duplicate clicks', async () => {
    const requests: unknown[] = [],
      pending = deferred<Task>()
    const state = newTaskBoardState(7)
    const board = new TaskBoardController(
      state,
      api({
        create: async (request) => {
          requests.push({ ...request })
          return requests.length === 1 ? pending.promise : task('a')
        },
      }),
    )
    state.form = newTaskForm('create', 7, null, '你好 👩‍💻\n保留草稿')
    const first = board.submit()
    expect(await board.submit()).toBeNull()
    pending.reject(new Error('connection lost'))
    await first
    expect(state.form?.text).toContain('保留草稿')
    expect(state.actionError).toBe('connection lost')
    state.form!.text = 'even a local edit must not change an uncertain request'
    await board.submit()
    expect(requests).toHaveLength(2)
    expect(requests[0]).toEqual(requests[1])
    expect(state.form).toBeNull()
  })

  test('409 requires explicit re-confirmation; refreshing does not silently grant it', async () => {
    let calls = 0
    const revisions: number[] = [],
      state = newTaskBoardState(7)
    const board = new TaskBoardController(
      state,
      api({
        get: async () => task('a', 7, 8),
        accept: async (_id, revision) => {
          calls++
          revisions.push(revision)
          if (calls === 1) throw Object.assign(new Error('changed'), { status: 409 })
          return { ...task('a', 7, 9), state: 'completed' }
        },
      }),
    )
    state.selected = task('a', 7, 2)
    state.form = newTaskForm('accept', 7, state.selected)
    await board.submit()
    expect(state.form?.target?.revision).toBe(2)
    expect(state.selected?.revision).toBe(8)
    await board.submit()
    expect(calls).toBe(1)
    board.rebase()
    await board.submit()
    expect(revisions).toEqual([2, 8])
    expect(state.selected?.state).toBe('completed')
  })

  test('off-scope detail and unmounted requests do not leak to another view', async () => {
    const state = newTaskBoardState(7),
      read = deferred<Task[]>()
    const board = new TaskBoardController(state, api({ get: async () => task('other', 8), list: () => read.promise }))
    await board.select('other')
    expect(state.selected).toBeNull()
    const pending = board.refresh()
    board.dispose()
    read.resolve([task('late')])
    await pending
    expect(state.items).toEqual([])
  })

  test('pagination is bounded and indicates more without inventing total counts', async () => {
    const state = newTaskBoardState(null)
    const board = new TaskBoardController(
      state,
      api({
        list: async (query) => {
          expect(query.limit).toBe(21)
          expect(query.offset).toBe(20)
          return Array.from({ length: 21 }, (_, index) => task(String(index)))
        },
      }),
    )
    state.page = 1
    await board.refresh()
    expect(state.items).toHaveLength(20)
    expect(state.hasMore).toBe(true)
  })

  test('UTF-8 limits and terminal-state controls match the task contract', () => {
    expect(validGoal(' ')).toBe(false)
    expect(validGoal('中'.repeat(11000))).toBe(false)
    expect(validGoal('合法任务 👩‍💻')).toBe(true)
    expect(canResumeTask({ ...task('a'), state: 'running' })).toBe(false)
    expect(canResumeTask({ ...task('a'), state: 'interrupted' })).toBe(true)
  })
})
