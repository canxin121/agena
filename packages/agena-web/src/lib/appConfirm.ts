import { readonly, shallowRef } from 'vue'

export type AppConfirmOptions = {
  title: string
  description?: string
  confirmText?: string
  cancelText?: string
  variant?: 'default' | 'destructive'
}

type AppConfirmRequest = AppConfirmOptions & {
  id: number
  resolve: (accepted: boolean) => void
}

const currentRequest = shallowRef<AppConfirmRequest | null>(null)
const queue: AppConfirmRequest[] = []
let serial = 0

export const appConfirmRequest = readonly(currentRequest)

function pumpConfirmQueue() {
  if (currentRequest.value || queue.length === 0) return
  currentRequest.value = queue.shift() || null
}

export function confirmAction(input: string | AppConfirmOptions): Promise<boolean> {
  const options: AppConfirmOptions =
    typeof input === 'string'
      ? { title: input, variant: 'destructive' }
      : { variant: 'destructive', ...input }

  return new Promise<boolean>((resolve) => {
    queue.push({
      ...options,
      id: ++serial,
      resolve,
    })
    pumpConfirmQueue()
  })
}

export function resolveAppConfirm(accepted: boolean) {
  const request = currentRequest.value
  if (!request) return
  currentRequest.value = null
  request.resolve(Boolean(accepted))
  queueMicrotask(pumpConfirmQueue)
}

export function clearAppConfirmQueue() {
  const active = currentRequest.value
  currentRequest.value = null
  active?.resolve(false)
  while (queue.length > 0) {
    queue.shift()?.resolve(false)
  }
}
