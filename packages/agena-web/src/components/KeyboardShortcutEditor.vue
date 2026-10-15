<script setup lang="ts">
import { ref, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import { isImeComposing } from '@/lib/imeKeyboard'
import { keyboardOverrides, saveKeyboardOverrides } from '@/app/runtime/keyboardPreferences'
import {
  SHORTCUT_ACTIONS,
  formatShortcut,
  shortcutBindings,
  shortcutFromEvent,
  shortcutMayConflictWithBrowser,
  validateShortcut,
  type ShortcutActionId,
  type ShortcutOverrides,
} from '@/app/runtime/keyboardShortcutActions'

const { t } = useI18n()
type ActionLabelMap = Record<ShortcutActionId, string>
const props = defineProps<{ labels: ActionLabelMap }>()
const recording = ref<ShortcutActionId | null>(null)
const status = ref('')
const expanded = ref(false)
const panelId = useId()

function toggleEditor() {
  expanded.value = !expanded.value
  recording.value = null
}
function startRecording(action: ShortcutActionId) {
  recording.value = action
  status.value = String(t('keyboard.shortcutRecording'))
}

function save(next: ShortcutOverrides) {
  const saved = saveKeyboardOverrides(next)
  status.value = String(t(saved ? 'keyboard.shortcutSaved' : 'keyboard.shortcutStorageError'))
  return saved
}
function reset(action?: ShortcutActionId) {
  const next = { ...keyboardOverrides.value }
  if (action) delete next[action]
  const candidate = action ? next : {}
  // A reset must not silently introduce a conflict with another customization.
  if (action) {
    const issue = shortcutBindings(action, candidate)
      .map((binding) => validateShortcut(binding, action, candidate))
      .find(Boolean)
    if (issue) {
      status.value = String(t('keyboard.shortcutResetConflict'))
      return
    }
  }
  save(candidate)
}
function record(event: KeyboardEvent, action: ShortcutActionId) {
  if (recording.value !== action || isImeComposing(event) || event.repeat) return
  event.preventDefault()
  event.stopImmediatePropagation()
  if (event.key === 'Escape') {
    recording.value = null
    status.value = ''
    return
  }
  const binding = shortcutFromEvent(event)
  if (!binding) return
  const issue = validateShortcut(binding, action, keyboardOverrides.value)
  if (issue) {
    status.value =
      issue.type === 'duplicate'
        ? String(t('keyboard.shortcutDuplicate', { action: props.labels[issue.action!] }))
        : String(t(issue.type === 'modifier' ? 'keyboard.shortcutNeedsModifier' : 'keyboard.shortcutReserved'))
    return
  }
  if (!save({ ...keyboardOverrides.value, [action]: [binding] })) return
  recording.value = null
  if (shortcutMayConflictWithBrowser(binding)) status.value = String(t('keyboard.shortcutBrowserConflict'))
}
</script>

<template>
  <section class="rounded-md border border-border/60 p-2">
    <button
      type="button"
      :aria-expanded="expanded"
      :aria-controls="panelId"
      class="rounded px-1 py-1 text-sm font-medium focus-visible:ring-2 focus-visible:ring-ring"
      @click="toggleEditor"
    >
      {{ t('keyboard.customizeShortcuts') }}
    </button>
    <div v-if="expanded" :id="panelId">
      <p class="py-2 text-xs text-muted-foreground">{{ t('keyboard.shortcutScope') }}</p>
      <div class="space-y-3">
        <div
          v-for="action in SHORTCUT_ACTIONS"
          :key="action"
          role="group"
          :aria-label="labels[action]"
          class="flex flex-wrap items-center gap-2 text-xs"
          :data-shortcut-action="action"
        >
          <span class="min-w-32 flex-1">{{ labels[action] }}</span>
          <span class="break-words font-mono">{{
            shortcutBindings(action, keyboardOverrides).map(formatShortcut).join(' / ') ||
            t('keyboard.shortcutDisabled')
          }}</span>
          <button
            type="button"
            class="rounded border border-border px-2 py-1 focus-visible:ring-2 focus-visible:ring-ring"
            :aria-pressed="recording === action"
            @click="startRecording(action)"
            @blur="recording === action && (recording = null)"
            @keydown.capture="record($event, action)"
          >
            {{ t(recording === action ? 'keyboard.shortcutRecording' : 'keyboard.changeShortcut') }}
          </button>
          <button
            type="button"
            class="rounded px-2 py-1 hover:bg-secondary focus-visible:ring-2 focus-visible:ring-ring"
            @click="save({ ...keyboardOverrides, [action]: [] })"
          >
            {{ t('keyboard.disableShortcut') }}
          </button>
          <button
            type="button"
            class="rounded px-2 py-1 hover:bg-secondary focus-visible:ring-2 focus-visible:ring-ring"
            @click="reset(action)"
          >
            {{ t('keyboard.resetShortcut') }}
          </button>
        </div>
      </div>
      <p role="status" aria-live="polite" class="min-h-6 py-2 text-xs">{{ status }}</p>
      <button
        type="button"
        class="rounded border border-border px-2 py-1 text-xs focus-visible:ring-2 focus-visible:ring-ring"
        @click="reset()"
      >
        {{ t('keyboard.resetAllShortcuts') }}
      </button>
    </div>
  </section>
</template>
