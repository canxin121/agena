import { readonly, shallowRef } from 'vue'

export type AppTextPromptOptions = {
  title: string
  description?: string
  placeholder?: string
  initialValue?: string
  confirmText?: string
  cancelText?: string
}

type AppTextPromptRequest = AppTextPromptOptions & {
  id: number
  resolve: (value: string | null) => void
}

const currentRequest = shallowRef<AppTextPromptRequest | null>(null)
const queue: AppTextPromptRequest[] = []
let serial = 0

export const appTextPromptRequest = readonly(currentRequest)

function pumpTextPromptQueue() {
  if (currentRequest.value || queue.length === 0) return
  currentRequest.value = queue.shift() || null
}

export function promptForText(options: AppTextPromptOptions): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    queue.push({ ...options, id: ++serial, resolve })
    pumpTextPromptQueue()
  })
}

export function resolveAppTextPrompt(value: string | null) {
  const request = currentRequest.value
  if (!request) return
  currentRequest.value = null
  request.resolve(value)
  queueMicrotask(pumpTextPromptQueue)
}

export function clearAppTextPromptQueue() {
  const active = currentRequest.value
  currentRequest.value = null
  active?.resolve(null)
  while (queue.length > 0) queue.shift()?.resolve(null)
}
