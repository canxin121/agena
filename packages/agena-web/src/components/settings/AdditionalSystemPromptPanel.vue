<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'

import AdditionalSystemPromptEditor from './AdditionalSystemPromptEditor.vue'
import SettingsCard from './SettingsCard.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { apiJson } from '@/lib/api'
import { confirmAction } from '@/lib/appConfirm'
import {
  additionalSystemPromptDraft,
  additionalSystemPromptIsDirty,
  additionalSystemPromptIsValid,
  additionalSystemPromptValue,
  type AdditionalSystemPromptDraft,
} from '@/lib/additionalSystemPrompt'
import { deleteRuntimeSetting, getRuntimeSettingLayer, setRuntimeSetting } from '@/lib/runtimeSettings'
import { useChatStore } from '@/stores/chat'
import { setSessionAdditionalSystemPrompt, type AgenaExecutionState } from '@/stores/chat/api'
import { useDirectoryStore } from '@/stores/directory'
import { useToastsStore } from '@/stores/toasts'

type PromptScope = 'effective' | 'global' | 'workspace' | 'session'
type SavedDraft = { draft: AdditionalSystemPromptDraft; saved: string | null }
const SETTING_PATH = 'session.additional_system_prompt'
const props = defineProps<{ scope: string }>()
const chat = useChatStore()
const directory = useDirectoryStore()
const toasts = useToastsStore()
const scope = computed<PromptScope>(() =>
  props.scope === 'global' || props.scope === 'workspace' || props.scope === 'session' ? props.scope : 'effective',
)
const sessionId = computed(() => {
  const id = Number(chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? String(id) : null
})
const needsSession = computed(() => scope.value === 'session' || scope.value === 'effective')
const contextAvailable = computed(() => !needsSession.value || sessionId.value !== null)
const contextKey = computed(() =>
  JSON.stringify([
    scope.value,
    needsSession.value ? sessionId.value : null,
    scope.value === 'global' ? null : directory.currentDirectory,
  ]),
)
const draft = ref(additionalSystemPromptDraft(null))
const saved = ref<string | null>(null)
const loaded = ref(false)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const dirty = computed(() => additionalSystemPromptIsDirty(draft.value, saved.value))
const valid = computed(() => additionalSystemPromptIsValid(draft.value))
const editingDisabled = computed(() => !loaded.value || loading.value || saving.value || !contextAvailable.value)
const drafts = new Map<string, SavedDraft>()
let loadedContext = ''
let loadGeneration = 0
let loadController: AbortController | undefined

function rememberDraft() {
  if (!loaded.value || !loadedContext) return
  if (dirty.value) drafts.set(loadedContext, { draft: { ...draft.value }, saved: saved.value })
  else drafts.delete(loadedContext)
}

async function readValue(source: PromptScope, id: string | null, signal: AbortSignal): Promise<string | null> {
  if (source === 'global' || source === 'workspace') {
    const response = await getRuntimeSettingLayer(source, SETTING_PATH)
    return typeof response.value === 'string' ? response.value : null
  }
  if (!id) return null
  const state = await apiJson<AgenaExecutionState>('/api/v1/sessions/' + encodeURIComponent(id) + '/state', { signal })
  const value =
    source === 'session'
      ? state.execution?.additional_system_prompt
      : state.execution?.effective_additional_system_prompt
  return typeof value === 'string' ? value : null
}

async function load() {
  rememberDraft()
  const key = contextKey.value
  const source = scope.value
  const id = sessionId.value
  const generation = ++loadGeneration
  loadController?.abort()
  const controller = new AbortController()
  loadController = controller
  loading.value = true
  loaded.value = false
  error.value = ''
  try {
    const value = await readValue(source, id, controller.signal)
    if (generation !== loadGeneration || key !== contextKey.value) return
    const cached = drafts.get(key)
    saved.value = cached ? cached.saved : value
    draft.value = cached ? { ...cached.draft } : additionalSystemPromptDraft(value)
    loadedContext = key
    loaded.value = true
  } catch (reason) {
    if (generation === loadGeneration && key === contextKey.value && !controller.signal.aborted)
      error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (generation === loadGeneration) loading.value = false
  }
}

function discard() {
  draft.value = additionalSystemPromptDraft(saved.value)
  drafts.delete(loadedContext)
  error.value = ''
}

async function save() {
  const source = scope.value
  if (editingDisabled.value || source === 'effective' || !dirty.value || !valid.value) return
  const key = contextKey.value
  const id = sessionId.value
  const value = additionalSystemPromptValue(draft.value)
  saving.value = true
  error.value = ''
  try {
    if (source === 'session') {
      if (!id) return
      await setSessionAdditionalSystemPrompt(id, value)
    } else if (value === null) {
      await deleteRuntimeSetting(SETTING_PATH, undefined, source)
    } else {
      await setRuntimeSetting(SETTING_PATH, value, undefined, source)
    }
    drafts.delete(key)
    if (key === contextKey.value) {
      saved.value = value
      draft.value = additionalSystemPromptDraft(value)
      await load()
    }
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    if (key === contextKey.value) error.value = message
    else toasts.push('error', message)
  } finally {
    saving.value = false
  }
}

async function canLeave() {
  if (saving.value) return false
  rememberDraft()
  if (!dirty.value && !drafts.size) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    description: st('You have unsaved changes.'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}
onBeforeRouteUpdate(async () => {
  if (!(await canLeave())) return false
  discard()
  drafts.clear()
  return true
})
onBeforeRouteLeave(canLeave)
function beforeUnload(event: BeforeUnloadEvent) {
  rememberDraft()
  if (!dirty.value && !drafts.size) return
  event.preventDefault()
  event.returnValue = ''
}
async function refresh() {
  if (!saving.value) await load()
}
defineExpose({ refresh })
watch(contextKey, () => void load())
onMounted(() => {
  void load()
  window.addEventListener('beforeunload', beforeUnload)
})
onBeforeUnmount(() => {
  loadGeneration += 1
  loadController?.abort()
  window.removeEventListener('beforeunload', beforeUnload)
})
</script>

<template>
  <div class="grid min-w-0 gap-4" :aria-busy="loading || saving">
    <SettingsCard
      :title="$st('Additional system prompt')"
      :description="
        $st('Priority: session, then model, workspace, and global. Each override replaces the lower scopes.')
      "
    >
      <p v-if="needsSession && sessionId" class="mb-3 text-xs text-muted-foreground">
        {{ $st('Selected session: {id}', { id: sessionId }) }}
      </p>
      <p v-if="!contextAvailable" class="text-sm text-muted-foreground">
        {{ $st('Select a session to inspect or edit its additional instructions.') }}
      </p>
      <p v-else-if="loading" class="text-sm text-muted-foreground" role="status">{{ $st('Loading…') }}</p>
      <div v-else-if="loaded && scope === 'effective'" class="grid min-w-0 gap-3">
        <p class="text-xs text-muted-foreground">
          {{
            $st('Resolved additional instructions for the selected session. Built-in instructions are not shown here.')
          }}
        </p>
        <pre
          v-if="saved !== null && saved !== ''"
          class="max-h-[32rem] overflow-auto whitespace-pre-wrap break-words rounded-md border border-border/60 bg-muted/10 p-3 font-sans text-sm leading-relaxed"
          >{{ saved }}</pre
        >
        <p v-else class="text-sm text-muted-foreground">
          {{
            saved === ''
              ? $st('Additional instructions are explicitly disabled.')
              : $st('No additional instructions are configured.')
          }}
        </p>
      </div>
      <AdditionalSystemPromptEditor v-else-if="loaded" v-model="draft" :disabled="editingDisabled" />
      <p v-if="error" class="mt-3 break-words text-xs text-destructive" role="alert">{{ error }}</p>
    </SettingsCard>
    <SettingsSaveBar
      v-if="scope !== 'effective' && contextAvailable && loaded"
      :dirty="dirty"
      :saving="saving"
      :disabled="editingDisabled"
      :save-disabled="!valid"
      @save="save"
      @discard="discard"
    />
  </div>
</template>
