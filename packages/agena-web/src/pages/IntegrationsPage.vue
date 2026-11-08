<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useChatStore } from '@/stores/chat'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import Button from '@/components/ui/Button.vue'
import Dialog from '@/components/ui/Dialog.vue'
import {
  integrationApi,
  type IntegrationsSnapshot,
  type IntegrationDetail,
  type IntegrationBinding,
  type CreateIntegration,
} from '@/lib/integrationApi'
import { integrationTemplate, parseIntegration } from '@/features/integrations/model'
import ChannelAccessPanel from '@/features/integrations/ChannelAccessPanel.vue'

const route = useRoute(),
  router = useRouter(),
  chat = useChatStore(),
  { t } = useI18n(),
  visible = usePaneVisibility()
const source = computed(() => {
  const n = Number(route.query.source || chat.selectedSessionId)
  return Number.isSafeInteger(n) && n > 0 ? n : null
})
const snapshot = ref<IntegrationsSnapshot | null>(null),
  detail = ref<IntegrationDetail | null>(null)
const accessEditing = ref(false)
const selected = ref(''),
  loading = ref(false),
  busy = ref(false),
  error = ref('')
const form = ref<{ text: string; request: CreateIntegration | null } | null>(null)
const confirm = ref<{ binding: IntegrationBinding; action: 'enable' | 'pause' | 'poll_now' } | null>(null)
let generation = 0,
  disposed = false,
  reading: AbortController | null = null
const copy = <T,>(v: T): T => JSON.parse(JSON.stringify(v)) as T
function fail(e: unknown) {
  error.value = e instanceof Error ? e.message : String(e)
}
function fence() {
  ++generation
  reading?.abort()
  loading.value = false
}
async function load() {
  if (!source.value || busy.value || loading.value || disposed) return
  const id = source.value,
    gen = ++generation,
    controller = new AbortController()
  reading?.abort()
  reading = controller
  loading.value = true
  try {
    const next = await integrationApi.list(id, controller.signal)
    if (disposed || gen !== generation) return
    snapshot.value = next
    const target = next.bindings.find((b) => b.id === selected.value) || next.bindings[0]
    if (target) {
      selected.value = target.id
      const result = await integrationApi.detail(id, target.id, controller.signal)
      if (gen === generation && !disposed) detail.value = result
    } else detail.value = null
  } catch (e) {
    if (gen === generation && !controller.signal.aborted) fail(e)
  } finally {
    if (gen === generation) loading.value = false
  }
}
async function select(id: string) {
  fence()
  selected.value = id
  detail.value = null
  await load()
}
function create(provider: NonNullable<typeof snapshot.value>['providers'][number]) {
  error.value = ''
  form.value = { text: JSON.stringify(integrationTemplate(provider), null, 2), request: null }
}
async function submit() {
  if (!source.value || !form.value || busy.value) return
  const id = source.value,
    original = form.value
  try {
    original.request ||= { request_key: crypto.randomUUID(), spec: parseIntegration(original.text) }
  } catch (e) {
    fail(e)
    return
  }
  fence()
  busy.value = true
  error.value = ''
  try {
    const result = await integrationApi.create(id, original.request)
    if (source.value !== id || disposed) return
    selected.value = result.id
    form.value = null
  } catch (e) {
    fail(e)
  } finally {
    busy.value = false
  }
  await load()
}
async function apply() {
  if (!source.value || !confirm.value || busy.value) return
  const id = source.value,
    original = confirm.value
  fence()
  busy.value = true
  error.value = ''
  try {
    await integrationApi.control(id, original.binding.id, {
      action: original.action,
      expected_revision: original.binding.revision,
    })
    confirm.value = null
  } catch (e) {
    fail(e)
    confirm.value = null
  } finally {
    busy.value = false
  }
  await load()
}
async function openSession(id: number) {
  try {
    await chat.selectSession(String(id))
    await router.push({ path: '/chat', query: { sessionId: String(id) } })
  } catch (e) {
    fail(e)
  }
}
watch(
  source,
  () => {
    fence()
    snapshot.value = null
    detail.value = null
    selected.value = ''
    form.value = null
    confirm.value = null
    void load()
  },
  { immediate: true },
)
watch(visible, (value) => {
  if (value) void load()
})
const timer = setInterval(() => {
  if (visible.value && !form.value && !confirm.value && !accessEditing.value) void load()
}, 10000)
onBeforeUnmount(() => {
  disposed = true
  fence()
  clearInterval(timer)
})
</script>
<template>
  <main class="h-full min-h-0 overflow-y-auto bg-background p-4 sm:p-6" data-testid="integrations-page">
    <header class="mx-auto max-w-6xl">
      <div class="flex flex-wrap justify-between items-center gap-3">
        <h1 class="text-xl font-semibold">{{ t('integrations.title') }}</h1>
        <Button
          variant="outline"
          size="sm"
          :disabled="loading || busy"
          @click="load"
          data-testid="integrations-refresh"
          >{{ t('assistant.refresh') }}</Button
        >
      </div>
      <p class="mt-2 text-sm text-muted-foreground">{{ t('integrations.description') }}</p>
    </header>
    <p v-if="!source" class="mx-auto mt-6 max-w-6xl">{{ t('assistant.noSession') }}</p>
    <p
      v-if="error"
      class="mx-auto mt-4 max-w-6xl rounded border border-destructive p-3 text-sm text-destructive break-words"
      role="alert"
    >
      {{ error }}
    </p>
    <div v-if="snapshot" class="mx-auto mt-5 max-w-6xl grid gap-5 lg:grid-cols-[minmax(220px,1fr)_minmax(0,2fr)]">
      <aside class="min-w-0 space-y-4">
        <h2 class="font-medium">{{ t('integrations.providers') }}</h2>
        <p v-if="!snapshot.providers.length" class="text-sm text-muted-foreground">
          {{ t('integrations.noProviders') }}
        </p>
        <article v-for="p in snapshot.providers" :key="p.plugin_id" class="rounded border border-border p-3">
          <h3 class="text-sm font-semibold">{{ p.adapter.title }}</h3>
          <p class="mt-1 break-all text-xs text-muted-foreground">{{ p.plugin_id }} · {{ p.adapter.kind }}</p>
          <p class="mt-2 text-xs">{{ p.adapter.description }}</p>
          <Button
            class="mt-3"
            size="sm"
            variant="outline"
            :disabled="busy"
            @click="create(p)"
            :data-provider="p.plugin_id"
            >{{ t('integrations.create') }}</Button
          >
        </article>
        <h2 class="pt-2 font-medium">{{ t('integrations.bindings') }}</h2>
        <p v-if="!snapshot.bindings.length" class="text-sm text-muted-foreground">{{ t('integrations.empty') }}</p>
        <button
          v-for="b in snapshot.bindings"
          :key="b.id"
          class="w-full text-left rounded border border-border p-3 focus-visible:ring-2 focus-visible:ring-ring"
          :class="selected === b.id ? 'bg-muted' : ''"
          @click="select(b.id)"
          :data-binding="b.id"
        >
          <span class="block text-sm font-medium">{{ b.spec.title }}</span
          ><span class="text-xs text-muted-foreground break-all"
            >{{ b.spec.resource }} · {{ t(`integrations.state_${b.state}`) }}</span
          >
        </button>
      </aside>
      <section v-if="detail" class="min-w-0 space-y-4" data-testid="integration-detail">
        <div class="rounded-lg border border-border p-4">
          <h2 class="font-semibold">{{ detail.binding.spec.title }}</h2>
          <p class="mt-1 text-xs text-muted-foreground">
            {{ detail.binding.spec.provider }} · {{ detail.binding.spec.mode }} ·
            {{ t(`integrations.state_${detail.binding.state}`) }}
          </p>
          <p class="mt-3 break-all text-sm">
            {{ detail.binding.spec.resource
            }}<span v-if="detail.binding.spec.thread"> / {{ detail.binding.spec.thread }}</span>
          </p>
          <p class="mt-2 whitespace-pre-wrap text-sm">{{ detail.binding.spec.instruction }}</p>
          <p class="mt-3 text-xs">
            {{ t('integrations.budget') }} {{ detail.binding.day_admitted }}/{{ detail.binding.spec.daily_limit }} ·
            {{ detail.binding.admitted }}/{{ detail.binding.spec.total_limit }} · {{ t('integrations.expires') }}
            {{ new Date(detail.binding.spec.expires_at_ms).toLocaleString() }}
          </p>
          <p v-if="detail.binding.last_error" class="mt-3 text-sm text-destructive">{{ detail.binding.last_error }}</p>
          <div class="mt-4 flex flex-wrap gap-2">
            <Button
              size="sm"
              :disabled="busy || detail.binding.state === 'enabled'"
              @click="confirm = { binding: copy(detail.binding), action: 'enable' }"
              data-testid="integration-enable"
              >{{ t('integrations.enable') }}</Button
            ><Button
              variant="outline"
              size="sm"
              :disabled="busy || detail.binding.state !== 'enabled'"
              @click="confirm = { binding: copy(detail.binding), action: 'pause' }"
              data-testid="integration-pause"
              >{{ t('assistant.pause') }}</Button
            ><Button
              variant="outline"
              size="sm"
              :disabled="busy || detail.binding.state !== 'enabled'"
              @click="confirm = { binding: copy(detail.binding), action: 'poll_now' }"
              >{{ t('integrations.poll') }}</Button
            ><Button
              v-if="detail.binding.channel_session_id"
              variant="ghost"
              size="sm"
              @click="openSession(detail.binding.channel_session_id)"
              >{{ t('assistant.openChat') }}</Button
            >
          </div>
        </div>
        <ChannelAccessPanel
          v-if="source"
          :source="source"
          :detail="detail"
          :readonly="loading || busy"
          @editing="
            (value) => {
              accessEditing = value
              if (value) fence()
            }
          "
          @changed="
            (binding) => {
              if (detail) detail.binding = binding
              fence()
              void load()
            }
          "
        />
        <details class="rounded border border-border p-3">
          <summary class="cursor-pointer text-sm">{{ t('integrations.configuration') }}</summary>
          <pre class="mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-all text-xs">{{
            JSON.stringify(detail.binding.spec, null, 2)
          }}</pre>
        </details>
        <h3 class="font-medium">{{ t('integrations.events') }}</h3>
        <p v-if="!detail.events.length" class="text-sm text-muted-foreground">{{ t('integrations.noEvents') }}</p>
        <article v-for="event in detail.events" :key="event.event.id" class="rounded border border-border p-3">
          <p class="text-xs break-all">{{ event.event.id }} · {{ event.state }} · {{ event.event.actor }}</p>
          <p class="mt-2 line-clamp-4 whitespace-pre-wrap break-words text-sm">{{ event.event.text }}</p>
          <p v-if="event.note" class="mt-2 text-xs text-muted-foreground">{{ event.note }}</p>
          <button
            v-if="event.task_id"
            class="mt-2 text-xs underline"
            @click="
              router.push({
                path: '/tasks',
                query: { source: String(detail.binding.source_session_id), task: event.task_id },
              })
            "
          >
            {{ t('tasks.details') }}
          </button>
        </article>
        <h3 class="font-medium">{{ t('integrations.deliveries') }}</h3>
        <article v-for="d in detail.deliveries" :key="d.id" class="rounded border border-border p-3">
          <p class="text-xs">{{ d.state }} · {{ d.attempts }}</p>
          <p class="mt-2 text-sm whitespace-pre-wrap break-words line-clamp-4">{{ d.text }}</p>
          <p v-if="d.error" class="mt-2 text-xs text-destructive">{{ d.error }}</p>
        </article>
      </section>
      <p v-else class="text-sm text-muted-foreground">
        {{ loading ? t('assistant.loading') : t('integrations.select') }}
      </p>
    </div>
    <Dialog
      :open="!!form"
      :title="t('integrations.create')"
      :description="t('integrations.createHint')"
      @close="!busy && (form = null)"
      ><form v-if="form" class="p-4 space-y-3" @submit.prevent="submit">
        <label class="block text-sm"
          >{{ t('integrations.configuration')
          }}<textarea
            v-model="form.text"
            :disabled="busy || !!form.request"
            rows="17"
            spellcheck="false"
            class="mt-2 w-full rounded border border-border bg-background p-3 font-mono text-xs"
            data-testid="integration-json"
          />
        </label>
        <p v-if="error" class="text-sm text-destructive" role="alert">{{ error }}</p>
        <p v-if="form.request && error" class="text-xs">{{ t('tasks.uncertain') }}</p>
        <div class="flex justify-end gap-2">
          <Button type="button" variant="ghost" :disabled="busy" @click="form = null">{{ t('assistant.close') }}</Button
          ><Button type="submit" :disabled="busy" data-testid="integration-create">{{ t('assistant.save') }}</Button>
        </div>
      </form></Dialog
    >
    <Dialog
      :open="!!confirm"
      :title="t('integrations.confirm')"
      :description="confirm?.action === 'enable' ? t('integrations.enableHint') : t('integrations.controlHint')"
      @close="!busy && (confirm = null)"
      ><div v-if="confirm" class="p-4 space-y-3">
        <pre class="max-h-64 overflow-auto whitespace-pre-wrap break-all text-xs">{{
          JSON.stringify(confirm.binding.spec, null, 2)
        }}</pre>
        <p class="text-sm">{{ t('integrations.review') }}</p>
        <div class="flex justify-end gap-2">
          <Button variant="ghost" :disabled="busy" @click="confirm = null">{{ t('assistant.close') }}</Button
          ><Button :disabled="busy" @click="apply" data-testid="integration-confirm">{{
            t('assistant.confirm')
          }}</Button>
        </div>
      </div></Dialog
    >
  </main>
</template>
