import { computed, ref, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'

import type { WorkspacePreviewSessionCreateInput } from '../api/workspacePreviewApi'
import { parsePreviewArgs } from './previewArgs'

type Options = {
  directory: Readonly<Ref<string>>
  agenaSessionId?: Readonly<Ref<string>>
  createSession: (input: WorkspacePreviewSessionCreateInput & { select: boolean }) => Promise<unknown>
  onCreated?: () => void
}

/** One creation contract for the dock and the standalone preview sidebar. */
export function usePreviewSessionForm(options: Options) {
  const { t } = useI18n()
  const createDialogOpen = ref(false)
  const createPreviewId = ref('')
  const createRunDirectory = ref('')
  const createCommand = ref('')
  const createArgsText = ref('')
  const createLogsPath = ref('')
  const createTargetUrl = ref('')
  const actionLoading = ref(false)
  const actionError = ref('')

  const parsedArgs = computed(() => {
    try {
      return { args: parsePreviewArgs(createArgsText.value), error: '' }
    } catch (error) {
      return { args: [], error: error instanceof Error ? error.message : String(error) }
    }
  })
  const createArgsError = computed(() => parsedArgs.value.error)
  const validationError = computed(() => {
    const required = (key: string) => String(t(`workspaceDock.preview.emptyState.${key}`))
    if (!options.directory.value.trim()) return required('directoryRequired')
    if (!createPreviewId.value.trim()) return required('sessionIdRequired')
    if (!/^[A-Za-z0-9_-]+$/.test(createPreviewId.value.trim())) return required('sessionIdInvalid')
    if (!createRunDirectory.value.trim()) return required('runDirectoryRequired')
    if (!createCommand.value.trim()) return required('commandRequired')
    if (createArgsError.value) return createArgsError.value
    if (!createLogsPath.value.trim()) return required('logsPathRequired')
    if (!createTargetUrl.value.trim()) return required('targetUrlRequired')
    return ''
  })
  const canCreateSession = computed(() => !actionLoading.value && !validationError.value)

  function openCreateDialog() {
    actionError.value = ''
    if (!createPreviewId.value.trim()) {
      const parts = options.directory.value
        .trim()
        .replace(/[\\/]+$/, '')
        .split(/[\\/]/)
      createPreviewId.value = (parts[parts.length - 1] || '')
        .replace(/[^A-Za-z0-9_-]/g, '-')
        .replace(/-+/g, '-')
        .replace(/^[-_]+|[-_]+$/g, '')
    }
    if (!createRunDirectory.value.trim()) createRunDirectory.value = options.directory.value
    if (!createLogsPath.value.trim())
      createLogsPath.value = `.agena/preview/${createPreviewId.value.trim() || 'preview'}.log`
    createDialogOpen.value = true
  }

  async function createManagedSession() {
    if (actionLoading.value) return
    actionError.value = validationError.value
    if (actionError.value) return
    actionLoading.value = true
    try {
      const agenaSessionId = options.agenaSessionId?.value.trim()
      await options.createSession({
        id: createPreviewId.value.trim(),
        directory: options.directory.value.trim(),
        runDirectory: createRunDirectory.value.trim(),
        command: createCommand.value.trim(),
        args: parsedArgs.value.args,
        logsPath: createLogsPath.value.trim(),
        targetUrl: createTargetUrl.value.trim(),
        ...(agenaSessionId ? { agenaSessionId } : {}),
        select: true,
      })
      for (const field of [
        createPreviewId,
        createRunDirectory,
        createCommand,
        createArgsText,
        createLogsPath,
        createTargetUrl,
      ]) {
        field.value = ''
      }
      createDialogOpen.value = false
      options.onCreated?.()
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error)
      actionError.value = String(t('workspaceDock.preview.emptyState.createFailed', { detail }))
    } finally {
      actionLoading.value = false
    }
  }

  return {
    createDialogOpen,
    createPreviewId,
    createRunDirectory,
    createCommand,
    createArgsText,
    createArgsError,
    createLogsPath,
    createTargetUrl,
    actionLoading,
    actionError,
    canCreateSession,
    createManagedSession,
    openCreateDialog,
  }
}
