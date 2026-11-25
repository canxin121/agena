import { nextTick, readonly, shallowRef } from 'vue'

export type AppDialogOptions = {
  title: string
  description?: string
  confirmText?: string
  cancelText?: string
}

type AppDialogRequest =
  | {
      kind: 'confirm'
      options: AppDialogOptions & { variant?: 'default' | 'destructive' }
      resolve: (value: boolean) => void
    }
  | {
      kind: 'text'
      options: AppDialogOptions & { placeholder?: string; initialValue?: string }
      resolve: (value: string | null) => void
    }

type QueuedDialog = AppDialogRequest & { id: number }
type DialogResult = boolean | string | null

const active = shallowRef<QueuedDialog | null>(null)
const pending: QueuedDialog[] = []
let nextId = 0
let closing = false

export const appDialogRequest = readonly(active)

function openNext() {
  if (!closing && !active.value && pending.length) active.value = pending.shift()!
}

export function enqueueAppDialog(request: AppDialogRequest) {
  pending.push({ ...request, id: ++nextId })
  openNext()
}

function resolve(request: AppDialogRequest, result: DialogResult) {
  if (request.kind === 'confirm') request.resolve(result === true)
  else request.resolve(typeof result === 'string' ? result : null)
}

export function finishAppDialog(id: number, result: DialogResult) {
  const request = active.value
  if (!request || request.id !== id) return
  closing = true
  active.value = null
  resolve(request, result)
  // Unmount the old dialog before opening the next focus scope.
  void nextTick(() => {
    closing = false
    openNext()
  })
}

export function clearAppDialogs(kind: AppDialogRequest['kind']) {
  if (active.value?.kind === kind) finishAppDialog(active.value.id, null)
  for (const request of pending.splice(0)) {
    if (request.kind === kind) resolve(request, null)
    else pending.push(request)
  }
}
