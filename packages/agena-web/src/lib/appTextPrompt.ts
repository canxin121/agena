import { clearAppDialogs, enqueueAppDialog, type AppDialogOptions } from './appDialogQueue'

export type AppTextPromptOptions = AppDialogOptions & {
  placeholder?: string
  initialValue?: string
}

export function promptForText(options: AppTextPromptOptions): Promise<string | null> {
  return new Promise((resolve) => enqueueAppDialog({ kind: 'text', options, resolve }))
}

export function clearAppTextPromptQueue() {
  clearAppDialogs('text')
}
