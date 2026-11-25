import { clearAppDialogs, enqueueAppDialog, type AppDialogOptions } from './appDialogQueue'

export type AppConfirmOptions = AppDialogOptions & {
  variant?: 'default' | 'destructive'
}

export function confirmAction(input: string | AppConfirmOptions): Promise<boolean> {
  const options: AppConfirmOptions =
    typeof input === 'string' ? { title: input, variant: 'destructive' } : { variant: 'destructive', ...input }
  return new Promise((resolve) => enqueueAppDialog({ kind: 'confirm', options, resolve }))
}

export function clearAppConfirmQueue() {
  clearAppDialogs('confirm')
}
