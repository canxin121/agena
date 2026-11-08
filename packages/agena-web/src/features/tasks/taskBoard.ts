/** Presentation state only. Server permissions and revisions remain authoritative. */
import type { Task, TaskApi, TaskState, CreateTask, ResumeTask } from '../../lib/taskApi'

export const TASK_PAGE_SIZE = 20
export const TASK_STATES: TaskState[] = [
  'queued',
  'starting',
  'running',
  'waiting',
  'review',
  'completed',
  'interrupted',
  'failed',
  'cancelling',
  'cancelled',
]
export const canResumeTask = (task: Task) => ['review', 'interrupted', 'failed', 'cancelled'].includes(task.state)
export const canCancelTask = (task: Task) => !['completed', 'cancelled', 'cancelling'].includes(task.state)
export const needsAttention = (task: Task) => ['waiting', 'review', 'failed', 'interrupted'].includes(task.state)
export const validGoal = (text: string) => Boolean(text.trim()) && new TextEncoder().encode(text).length <= 32768

export type TaskForm = {
  kind: 'create' | 'resume' | 'cancel' | 'accept'
  target: Task | null
  source: number | null
  text: string
  key: string
  /** Freeze the exact command after first submit, including uncertain outcomes. */
  submitted?: CreateTask | ResumeTask | { expected_revision: number }
  conflict: boolean
}

export function newTaskForm(kind: TaskForm['kind'], source: number | null, target: Task | null, text = ''): TaskForm {
  return { kind, source, target: target ? { ...target } : null, text, key: crypto.randomUUID(), conflict: false }
}

export type TaskBoardState = {
  items: Task[]
  selected: Task | null
  page: number
  hasMore: boolean
  source: number | null
  filter: TaskState | ''
  loading: boolean
  error: string
  actionError: string
  busy: boolean
  form: TaskForm | null
}
export function newTaskBoardState(source: number | null): TaskBoardState {
  return {
    items: [],
    selected: null,
    page: 0,
    hasMore: false,
    source,
    filter: '',
    loading: false,
    error: '',
    actionError: '',
    busy: false,
    form: null,
  }
}

const message = (error: unknown) => (error instanceof Error ? error.message : String(error))
const status = (error: unknown) =>
  typeof error === 'object' && error !== null && 'status' in error ? Number(error.status) : 0

/** Bounded reads, stale-response fencing, retained commands and no auto-retry of writes. */
export class TaskBoardController {
  private epoch = 0
  private reads = 0
  private detailReads = 0
  private abort?: AbortController
  private detailAbort?: AbortController
  private disposed = false
  readonly state: TaskBoardState
  readonly api: TaskApi
  constructor(state: TaskBoardState, api: TaskApi) {
    this.state = state
    this.api = api
  }

  async refresh(): Promise<void> {
    if (this.disposed) return
    const generation = this.epoch,
      request = ++this.reads
    this.abort?.abort()
    this.abort = new AbortController()
    const state = this.state
    state.loading = true
    try {
      const rows = await this.api.list(
        {
          offset: state.page * TASK_PAGE_SIZE,
          limit: TASK_PAGE_SIZE + 1,
          ...(state.source ? { source_session_id: state.source } : {}),
          ...(state.filter ? { state: state.filter } : {}),
        },
        this.abort.signal,
      )
      if (this.disposed || generation !== this.epoch || request !== this.reads) return
      const previous = new Map(state.items.map((task) => [task.id, task]))
      if (state.selected) previous.set(state.selected.id, state.selected)
      state.hasMore = rows.length > TASK_PAGE_SIZE
      state.items = rows.slice(0, TASK_PAGE_SIZE).map((task) => {
        const saved = previous.get(task.id)
        return saved && saved.revision > task.revision ? saved : task
      })
      const selected = state.items.find((task) => task.id === state.selected?.id)
      if (selected) state.selected = selected
      state.error = ''
    } catch (error) {
      if (!this.disposed && generation === this.epoch && request === this.reads && !this.abort?.signal.aborted)
        state.error = message(error)
    } finally {
      if (generation === this.epoch && request === this.reads) state.loading = false
    }
  }

  async select(id: string): Promise<void> {
    const epoch = this.epoch,
      request = ++this.detailReads
    this.detailAbort?.abort()
    this.detailAbort = new AbortController()
    const candidate = this.state.items.find((task) => task.id === id)
    if (candidate) this.state.selected = candidate
    try {
      const task = await this.api.get(id, this.detailAbort.signal)
      if (this.disposed || epoch !== this.epoch || request !== this.detailReads) return
      if (this.state.source && task.source_session_id !== this.state.source) {
        this.state.selected = null
        return
      }
      this.update(task)
    } catch (error) {
      if (epoch === this.epoch && request === this.detailReads && !this.detailAbort.signal.aborted)
        this.state.error = message(error)
    }
  }

  setScope(source: number | null, filter: TaskState | '' = this.state.filter): void {
    ++this.epoch
    this.abort?.abort()
    this.detailAbort?.abort()
    Object.assign(this.state, {
      source,
      filter,
      page: 0,
      items: [],
      selected: null,
      hasMore: false,
      loading: false,
      error: '',
    })
  }

  update(task: Task): void {
    const selected = this.state.selected
    if (selected?.id !== task.id || selected.revision <= task.revision) this.state.selected = task
    this.state.items = this.state.items.map((old) => (old.id === task.id && old.revision <= task.revision ? task : old))
  }

  async submit(): Promise<Task | null> {
    const form = this.state.form
    if (!form || this.state.busy || form.conflict || this.disposed) return null
    if ((form.kind === 'create' || form.kind === 'resume') && !validGoal(form.text)) return null
    if (form.kind === 'create' && !form.source) return null
    const epoch = this.epoch
    this.state.busy = true
    this.state.actionError = ''
    if (!form.submitted) {
      form.submitted =
        form.kind === 'create'
          ? { source_session_id: form.source!, goal: form.text, request_key: form.key }
          : form.kind === 'resume'
            ? { expected_revision: form.target!.revision, instruction: form.text, request_key: form.key }
            : { expected_revision: form.target!.revision }
    }
    try {
      const task =
        form.kind === 'create'
          ? await this.api.create(form.submitted as CreateTask)
          : form.kind === 'resume'
            ? await this.api.resume(form.target!.id, form.submitted as ResumeTask)
            : form.kind === 'cancel'
              ? await this.api.cancel(form.target!.id, form.target!.revision)
              : await this.api.accept(form.target!.id, form.target!.revision)
      if (!this.disposed && epoch === this.epoch) {
        this.update(task)
        this.state.form = null
        await this.refresh()
      }
      return task
    } catch (error) {
      if (!this.disposed && epoch === this.epoch) {
        this.state.actionError = message(error)
        form.conflict = status(error) === 409
        if (form.conflict && form.target) await this.select(form.target.id)
      }
      return null
    } finally {
      if (!this.disposed) this.state.busy = false
    }
  }

  /** Deliberate reconfirmation after a revision conflict, never automatic replay. */
  rebase(): void {
    const form = this.state.form,
      latest = this.state.selected
    if (!form?.target || !latest || form.target.id !== latest.id) return
    this.state.form = newTaskForm(form.kind, form.source, latest, form.text)
    this.state.actionError = ''
  }

  dispose(): void {
    this.disposed = true
    ++this.epoch
    this.abort?.abort()
    this.detailAbort?.abort()
  }
}
