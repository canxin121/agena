<script setup lang="ts">
import { toRef, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '@/components/ui/Button.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import Input from '@/components/ui/Input.vue'
import { useWorkspacePreviewStore } from '@/stores/workspacePreview'
import { usePreviewSessionForm } from '../model/usePreviewSessionForm'

const props = defineProps<{ directory: string; agenaSessionId?: string }>()
const emit = defineEmits<{ created: [] }>()
const { t } = useI18n()
const preview = useWorkspacePreviewStore()
const formId = useId()
const {
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
} = usePreviewSessionForm({
  directory: toRef(() => props.directory),
  agenaSessionId: toRef(() => props.agenaSessionId || ''),
  createSession: (input) => preview.createSession(input),
  onCreated: () => emit('created'),
})

defineExpose({ open: openCreateDialog, busy: actionLoading })
</script>

<template>
  <FormDialog
    :open="createDialogOpen"
    :title="t('workspaceDock.preview.addDialog.title')"
    :description="t('workspaceDock.preview.addDialog.description')"
    max-width="max-w-md"
    initial-focus="[data-preview-session-name]"
    @update:open="createDialogOpen = $event"
  >
    <form :id="formId" class="grid gap-3" novalidate @submit.prevent="createManagedSession">
      <p class="text-xs text-muted-foreground [overflow-wrap:anywhere]">
        {{ t('workspaceDock.preview.directoryLabel') }}
        <span class="font-mono">{{ directory || t('workspaceDock.preview.emptyState.directoryEmpty') }}</span>
      </p>
      <fieldset :disabled="actionLoading" class="grid min-w-0 gap-3">
        <div class="grid gap-1">
          <label :for="`${formId}-name`" class="text-xs font-medium">
            {{ t('workspaceDock.preview.emptyState.sessionIdLabel') }}
          </label>
          <Input
            :id="`${formId}-name`"
            v-model="createPreviewId"
            data-preview-session-name
            autocapitalize="off"
            autocomplete="off"
            :spellcheck="false"
            :aria-describedby="`${formId}-name-help`"
            :placeholder="t('workspaceDock.preview.emptyState.sessionIdPlaceholder')"
            class="font-mono text-xs"
          />
          <p :id="`${formId}-name-help`" class="text-xs text-muted-foreground">
            {{ t('workspaceDock.preview.emptyState.sessionIdHelp') }}
          </p>
        </div>
        <label class="grid gap-1 text-xs font-medium">
          {{ t('workspaceDock.preview.emptyState.runDirectoryLabel') }}
          <Input
            v-model="createRunDirectory"
            :placeholder="t('workspaceDock.preview.emptyState.runDirectoryPlaceholder')"
            class="font-mono text-xs"
          />
        </label>
        <label class="grid gap-1 text-xs font-medium">
          {{ t('workspaceDock.preview.emptyState.commandLabel') }}
          <Input
            v-model="createCommand"
            :placeholder="t('workspaceDock.preview.emptyState.commandPlaceholder')"
            class="font-mono text-xs"
          />
        </label>
        <div class="grid gap-1">
          <label :for="`${formId}-args`" class="text-xs font-medium">
            {{ t('workspaceDock.preview.emptyState.argsLabel') }}
          </label>
          <Input
            :id="`${formId}-args`"
            v-model="createArgsText"
            placeholder='["--host", "127.0.0.1", "--name", "My app"]'
            :aria-invalid="createArgsError ? true : undefined"
            :aria-describedby="createArgsError ? `${formId}-args-error` : undefined"
            class="font-mono text-xs"
          />
          <p v-if="createArgsError" :id="`${formId}-args-error`" class="text-xs text-destructive" role="alert">
            {{ createArgsError }}
          </p>
        </div>
        <label class="grid gap-1 text-xs font-medium">
          {{ t('workspaceDock.preview.emptyState.logsPathLabel') }}
          <Input
            v-model="createLogsPath"
            :placeholder="t('workspaceDock.preview.emptyState.logsPathPlaceholder')"
            class="font-mono text-xs"
          />
        </label>
        <label class="grid gap-1 text-xs font-medium">
          {{ t('workspaceDock.preview.emptyState.targetUrlLabel') }}
          <Input
            v-model="createTargetUrl"
            type="url"
            :placeholder="t('workspaceDock.preview.emptyState.targetUrlPlaceholder')"
            class="font-mono text-xs"
          />
        </label>
      </fieldset>
      <p v-if="actionError && actionError !== createArgsError" class="text-xs text-destructive" role="alert">
        {{ actionError }}
      </p>
    </form>
    <template #footer>
      <div class="flex w-full flex-wrap items-center justify-end gap-2">
        <Button type="button" size="sm" variant="outline" :disabled="actionLoading" @click="createDialogOpen = false">
          {{ t('common.cancel') }}
        </Button>
        <Button type="submit" size="sm" :form="formId" :disabled="!canCreateSession">
          {{
            t(
              actionLoading
                ? 'workspaceDock.preview.emptyState.createLoading'
                : 'workspaceDock.preview.emptyState.addAction',
            )
          }}
        </Button>
      </div>
    </template>
  </FormDialog>
</template>
