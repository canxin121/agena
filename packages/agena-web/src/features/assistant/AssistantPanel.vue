<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '@/components/ui/Button.vue'
import Dialog from '@/components/ui/Dialog.vue'
import MemoryMetadataView from '@/components/MemoryMetadataView.vue'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { assistantApi, type AssistantSnapshot, type AssistantMemory, type Reminder } from '@/lib/assistantApi'
import { assistantMemoryRequest } from '@/lib/memoryRequests'
import type { AssistantPreferences, AssistantUpdate } from '@/generated/agenaState'
import { minuteText, parseMinute, validPreferences, profileChanged, preferencePatch } from './profileForm'

function copy<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

const props = defineProps<{ source: number }>()
const emit = defineEmits<{ openChat: [id: number] }>()
const { t, locale } = useI18n()
const visible = usePaneVisibility()
const snapshot = ref<AssistantSnapshot | null>(null)
const draft = ref<AssistantPreferences | null>(null)
const draftBase = ref<AssistantPreferences | null>(null),
  draftRevision = ref(0)
const tab = ref<'profile' | 'memory' | 'reminders'>('profile')
const loading = ref(false),
  busy = ref(false),
  error = ref(''),
  notice = ref(''),
  conflict = ref(false)
const pending = ref<AssistantUpdate | null>(null)
const memoryForm = ref<{ original: AssistantMemory; body: string; description: string; remove: boolean } | null>(null)
const confirmReminder = ref<{ reminder: Reminder; action: 'pause' | 'resume' | 'cancel' } | null>(null)
const dirty = computed(() => Boolean(draftBase.value && draft.value && profileChanged(draftBase.value, draft.value)))
let version = 0,
  disposed = false,
  abort: AbortController | null = null

function fenceReads() {
  ++version
  abort?.abort()
  loading.value = false
}

function failure(e: unknown) {
  error.value = e instanceof Error ? e.message : String(e)
}
async function load(reset = false) {
  if (loading.value || busy.value || disposed) return
  const request = ++version
  abort?.abort()
  abort = new AbortController()
  loading.value = true
  try {
    const next = await assistantApi.get(props.source, abort.signal)
    if (disposed || request !== version) return
    const preserve = !reset && (dirty.value || pending.value !== null)
    snapshot.value = next
    if (!preserve) {
      draft.value = copy(next.profile.preferences)
      draftBase.value = copy(next.profile.preferences)
      draftRevision.value = next.profile.revision
    }
    if (reset) {
      pending.value = null
      conflict.value = false
      error.value = ''
    }
    return true
  } catch (e) {
    if (request === version && !abort.signal.aborted) failure(e)
  } finally {
    if (request === version) loading.value = false
  }
}
async function saveProfile() {
  if (busy.value || !draft.value || !snapshot.value || conflict.value || !validPreferences(draft.value)) return
  pending.value ||= {
    expected_revision: draftRevision.value,
    request_key: crypto.randomUUID(),
    patch: preferencePatch(draftBase.value!, draft.value),
  }
  fenceReads()
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const result = await assistantApi.update(props.source, pending.value)
    if (disposed) return
    snapshot.value.profile = result
    draft.value = copy(result.preferences)
    draftBase.value = copy(result.preferences)
    draftRevision.value = result.revision
    pending.value = null
    notice.value = String(t('assistant.saved'))
  } catch (e) {
    failure(e)
    conflict.value = typeof e === 'object' && e !== null && 'status' in e && e.status === 409
  } finally {
    busy.value = false
  }
}
async function reconfirm() {
  // Keep the user's proposed values; refresh authority only on explicit action.
  const changes = draft.value && draftBase.value ? preferencePatch(draftBase.value, draft.value) : null
  const refreshed = await load(true)
  if (refreshed && changes && draft.value) draft.value = { ...draft.value, ...changes } as AssistantPreferences
}
function edit(memory: AssistantMemory, remove = false) {
  error.value = ''
  notice.value = ''
  memoryForm.value = { original: copy(memory), body: memory.body, description: memory.description, remove }
}
async function saveMemory() {
  const form = memoryForm.value
  if (
    !form ||
    busy.value ||
    (!form.remove &&
      (form.original.truncated || !form.body.trim() || new TextEncoder().encode(form.body).length > 16384))
  )
    return
  fenceReads()
  busy.value = true
  error.value = ''
  try {
    await assistantApi.memory(
      props.source,
      assistantMemoryRequest(
        form.original,
        form.remove
          ? null
          : {
              body: form.body,
              description: form.description,
            },
      ),
    )
    memoryForm.value = null
    notice.value = String(t('assistant.saved'))
  } catch (e) {
    failure(e)
  } finally {
    busy.value = false
  }
  await load()
}
async function controlReminder() {
  const form = confirmReminder.value
  if (!form || busy.value) return
  fenceReads()
  busy.value = true
  error.value = ''
  try {
    await assistantApi.reminder(props.source, form.reminder.id, {
      action: form.action,
      expected_revision: form.reminder.revision,
    })
    confirmReminder.value = null
    notice.value = String(t('assistant.saved'))
  } catch (e) {
    failure(e)
    confirmReminder.value = null
  } finally {
    busy.value = false
  }
  await load()
}
function minute(field: 'quiet_start_minute' | 'quiet_end_minute', event: Event) {
  const value = parseMinute((event.target as HTMLInputElement).value)
  if (draft.value && value !== null) draft.value[field] = value
}
function date(ms: number) {
  try {
    return new Date(ms).toLocaleString(String(locale.value), { timeZone: snapshot.value?.profile.preferences.timezone })
  } catch {
    return new Date(ms).toISOString()
  }
}
watch(
  visible,
  (value) => {
    if (value) void load()
  },
  { immediate: true },
)
const timer = setInterval(() => {
  if (visible.value && !dirty.value && !pending.value && !memoryForm.value && !confirmReminder.value) void load()
}, 5000)
onBeforeUnmount(() => {
  disposed = true
  ++version
  clearInterval(timer)
  abort?.abort()
})
</script>

<template>
  <section class="h-full min-h-0 overflow-y-auto bg-background p-4 sm:p-6" data-testid="assistant-panel">
    <header class="mx-auto max-w-4xl">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <h1 class="text-xl font-semibold">{{ t('assistant.title') }}</h1>
        <div class="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            :disabled="loading || busy"
            @click="load()"
            data-testid="assistant-refresh"
            >{{ t('assistant.refresh') }}</Button
          ><Button size="sm" variant="ghost" @click="emit('openChat', source)">{{ t('assistant.openChat') }}</Button>
        </div>
      </div>
      <p class="mt-2 text-sm text-muted-foreground">{{ t('assistant.description') }}</p>
      <p v-if="snapshot" class="mt-2 text-xs text-muted-foreground">
        {{ t('assistant.scope') }} #{{ snapshot.profile.workspace_id }} · {{ t('assistant.revision') }}
        {{ snapshot.profile.revision }}
      </p>
    </header>
    <div class="mx-auto mt-5 max-w-4xl">
      <div class="flex gap-2 border-b border-border" role="tablist" :aria-label="t('assistant.title')">
        <button
          v-for="key in ['profile', 'memory', 'reminders'] as const"
          :key="key"
          role="tab"
          class="px-4 py-3 text-sm border-b-2 focus-visible:ring-2 focus-visible:ring-ring"
          :class="tab === key ? 'border-primary font-semibold' : 'border-transparent text-muted-foreground'"
          :aria-selected="tab === key"
          @click="tab = key"
          :data-testid="`assistant-tab-${key}`"
        >
          {{ t(`assistant.${key}`) }}
        </button>
      </div>
      <p v-if="loading && !snapshot" role="status" class="py-6 text-sm">{{ t('assistant.loading') }}</p>
      <p
        v-if="error"
        role="alert"
        class="mt-4 rounded-md border border-destructive/30 p-3 text-sm text-destructive break-words"
      >
        {{ error }}
      </p>
      <p v-if="notice" role="status" class="mt-3 text-sm text-muted-foreground">{{ notice }}</p>
      <form v-if="tab === 'profile' && draft" class="mt-5 space-y-5" @submit.prevent="saveProfile">
        <fieldset :disabled="busy || !!pending" class="grid grid-cols-1 gap-4 sm:grid-cols-2 disabled:opacity-70">
          <label class="assistant-field"
            >{{ t('assistant.name') }}<input v-model="draft.name" maxlength="64" data-testid="assistant-name"
          /></label>
          <label class="assistant-field"
            >{{ t('assistant.address')
            }}<input data-testid="assistant-address" v-model="draft.address_user" maxlength="64"
          /></label>
          <label class="assistant-field"
            >{{ t('assistant.tone')
            }}<select v-model="draft.tone">
              <option v-for="tone in ['balanced', 'warm', 'direct', 'playful'] as const" :key="tone" :value="tone">
                {{ t(`assistant.tone_${tone}`) }}
              </option>
            </select></label
          >
          <label class="assistant-field"
            >{{ t('assistant.language')
            }}<input v-model="draft.language" placeholder="auto / 中文 / English" maxlength="32"
          /></label>
          <label class="assistant-field sm:col-span-2"
            >{{ t('assistant.instructions') }}<textarea v-model="draft.instructions" rows="4" />
          </label>
          <label class="flex items-center gap-2 text-sm"
            ><input type="checkbox" v-model="draft.memory_enabled" data-testid="assistant-memory-enabled" />{{
              t('assistant.memoryRecall')
            }}</label
          >
          <label class="flex items-center gap-2 text-sm"
            ><input type="checkbox" v-model="draft.proactive_enabled" data-testid="assistant-proactive-enabled" />{{
              t('assistant.proactive')
            }}</label
          >
          <label class="assistant-field"
            >{{ t('assistant.timezone') }}<input v-model="draft.timezone" placeholder="Asia/Shanghai" maxlength="64"
          /></label>
          <label class="flex items-center gap-2 text-sm"
            ><input type="checkbox" v-model="draft.quiet_enabled" />{{ t('assistant.quiet') }}</label
          >
          <label class="assistant-field"
            >{{ t('assistant.quietStart')
            }}<input
              type="time"
              :value="minuteText(draft.quiet_start_minute)"
              @input="minute('quiet_start_minute', $event)"
          /></label>
          <label class="assistant-field"
            >{{ t('assistant.quietEnd')
            }}<input
              type="time"
              :value="minuteText(draft.quiet_end_minute)"
              @input="minute('quiet_end_minute', $event)"
          /></label>
          <label class="assistant-field"
            >{{ t('assistant.daily') }}<input v-model.number="draft.daily_limit" type="number" min="1" max="24"
          /></label>
          <label class="assistant-field"
            >{{ t('assistant.cooldown')
            }}<input v-model.number="draft.cooldown_seconds" type="number" min="60" max="86400" step="60"
          /></label>
        </fieldset>
        <p class="text-xs text-muted-foreground">{{ t('assistant.policyHint') }}</p>
        <p v-if="pending && error && !conflict" class="text-sm">{{ t('assistant.retryHint') }}</p>
        <div class="flex flex-wrap gap-2">
          <Button
            type="submit"
            :disabled="busy || conflict || !validPreferences(draft) || (!dirty && !pending)"
            data-testid="assistant-save"
            >{{ busy ? t('assistant.saving') : t('assistant.save') }}</Button
          ><Button
            v-if="conflict"
            type="button"
            variant="outline"
            @click="reconfirm"
            data-testid="assistant-reconfirm"
            >{{ t('assistant.reconfirm') }}</Button
          >
        </div>
      </form>
      <section v-if="tab === 'memory' && snapshot" class="mt-5 space-y-3" role="tabpanel">
        <p class="text-sm text-muted-foreground">{{ t('assistant.memoryHint') }}</p>
        <p v-if="!snapshot.profile.preferences.memory_enabled" class="rounded-md bg-muted p-3 text-sm">
          {{ t('assistant.memoryDisabled') }}
        </p>
        <p v-if="!snapshot.memories.length" class="py-10 text-center text-muted-foreground">
          {{ t('assistant.noMemories') }}
        </p>
        <p v-if="snapshot.memories_truncated" class="text-sm">{{ t('assistant.bounded') }}</p>
        <article
          v-for="memory in snapshot.memories"
          :key="memory.name"
          class="rounded-lg border border-border p-4"
          :data-memory-name="memory.name"
        >
          <div class="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h2 class="font-medium break-all">{{ memory.name }}</h2>
              <p class="mt-1 text-xs text-muted-foreground">{{ memory.description }}</p>
              <MemoryMetadataView :metadata="memory.metadata" :timezone="snapshot.profile.preferences.timezone" />
            </div>
            <div class="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                :disabled="busy || memory.truncated"
                @click="edit(memory)"
                data-testid="assistant-memory-edit"
                >{{ t('assistant.edit') }}</Button
              ><Button
                size="sm"
                variant="ghost"
                :disabled="busy"
                @click="edit(memory, true)"
                data-testid="assistant-memory-delete"
                >{{ t('assistant.delete') }}</Button
              >
            </div>
          </div>
          <p class="mt-3 whitespace-pre-wrap break-words text-sm max-h-40 overflow-y-auto">{{ memory.body }}</p>
          <p v-if="memory.truncated" class="mt-2 text-xs">{{ t('assistant.truncated') }}</p>
        </article>
      </section>
      <section v-if="tab === 'reminders' && snapshot" class="mt-5 space-y-3" role="tabpanel">
        <p class="text-sm text-muted-foreground">{{ t('assistant.reminderHint') }}</p>
        <p v-if="!snapshot.profile.preferences.proactive_enabled" class="rounded-md bg-muted p-3 text-sm">
          {{ t('assistant.muted') }}
        </p>
        <p v-if="!snapshot.reminders.length" class="py-10 text-center text-muted-foreground">
          {{ t('assistant.noReminders') }}
        </p>
        <article
          v-for="reminder in snapshot.reminders"
          :key="reminder.id"
          :data-reminder-id="reminder.id"
          class="rounded-lg border border-border p-4"
        >
          <div class="flex flex-wrap items-center justify-between gap-3">
            <span class="rounded bg-muted px-2 py-1 text-xs">{{ t(`assistant.state_${reminder.state}`) }}</span
            ><span class="text-xs text-muted-foreground"
              >{{ reminder.delivered }} / {{ reminder.spec.occurrences }}</span
            >
          </div>
          <p class="mt-3 text-sm whitespace-pre-wrap break-words">{{ reminder.spec.text }}</p>
          <p class="mt-2 text-xs text-muted-foreground">
            {{ date(reminder.next_due_ms) }} · {{ snapshot.profile.preferences.timezone }}
          </p>
          <p v-if="reminder.retry_at_ms > reminder.next_due_ms" class="mt-1 text-xs">
            {{ t('assistant.deferred') }} {{ date(reminder.retry_at_ms) }}
          </p>
          <p v-if="reminder.last_error" class="mt-2 text-sm text-destructive">{{ reminder.last_error }}</p>
          <div class="mt-3 flex flex-wrap gap-2">
            <Button size="sm" variant="ghost" @click="emit('openChat', reminder.source_session_id)">{{
              t('assistant.openChat')
            }}</Button
            ><template v-if="!['completed', 'cancelled'].includes(reminder.state)"
              ><Button
                size="sm"
                variant="outline"
                :disabled="busy"
                @click="confirmReminder = { reminder, action: reminder.state === 'paused' ? 'resume' : 'pause' }"
                >{{ t(reminder.state === 'paused' ? 'assistant.resume' : 'assistant.pause') }}</Button
              ><Button
                size="sm"
                variant="ghost"
                :disabled="busy"
                @click="confirmReminder = { reminder, action: 'cancel' }"
                data-testid="assistant-reminder-cancel"
                >{{ t('assistant.cancel') }}</Button
              ></template
            >
          </div>
        </article>
      </section>
    </div>
    <Dialog
      :open="!!memoryForm"
      :title="t(memoryForm?.remove ? 'assistant.delete' : 'assistant.edit')"
      :description="memoryForm?.original.name"
      @close="!busy && (memoryForm = null)"
      ><form v-if="memoryForm" class="p-4 space-y-4" @submit.prevent="saveMemory">
        <MemoryMetadataView
          :metadata="memoryForm.original.metadata"
          :timezone="snapshot?.profile.preferences.timezone"
        />
        <p v-if="memoryForm.remove" class="text-sm">{{ t('assistant.confirmDelete') }}</p>
        <template v-else
          ><label class="assistant-field"
            >{{ t('assistant.descriptionLabel') }}<input v-model="memoryForm.description" :disabled="busy" /></label
          ><label class="assistant-field"
            >{{ t('assistant.memory')
            }}<textarea
              data-testid="assistant-memory-body"
              v-model="memoryForm.body"
              rows="8"
              :disabled="busy"
            /></label
        ></template>
        <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
        <div class="flex justify-end gap-2">
          <Button type="button" variant="ghost" :disabled="busy" @click="memoryForm = null">{{
            t('assistant.close')
          }}</Button
          ><Button type="submit" :disabled="busy" data-testid="assistant-memory-submit">{{
            t('assistant.confirm')
          }}</Button>
        </div>
      </form></Dialog
    >
    <Dialog
      :open="!!confirmReminder"
      :title="t('assistant.reminders')"
      :description="t('assistant.confirmSchedule')"
      @close="!busy && (confirmReminder = null)"
      ><div class="p-4 space-y-4">
        <p class="text-sm whitespace-pre-wrap">{{ confirmReminder?.reminder.spec.text }}</p>
        <div class="flex justify-end gap-2">
          <Button variant="ghost" :disabled="busy" @click="confirmReminder = null">{{ t('assistant.close') }}</Button
          ><Button :disabled="busy" @click="controlReminder" data-testid="assistant-reminder-confirm">{{
            t('assistant.confirm')
          }}</Button>
        </div>
      </div></Dialog
    >
  </section>
</template>
<style scoped>
.assistant-field {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
  font-size: 0.875rem;
}
.assistant-field input,
.assistant-field select,
.assistant-field textarea {
  border: 1px solid var(--border);
  background: var(--background);
  border-radius: 0.4rem;
  padding: 0.6rem;
  min-width: 0;
}
.assistant-field input:focus,
.assistant-field select:focus,
.assistant-field textarea:focus {
  outline: 2px solid var(--ring);
  outline-offset: 1px;
}
</style>
