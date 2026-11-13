<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { RouterLink, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useChatStore } from '@/stores/chat'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { integrationApi, type PlatformInventory, type ConversationEntry } from '@/lib/integrationApi'

const route = useRoute(),
  chat = useChatStore(),
  { t } = useI18n(),
  visible = usePaneVisibility()
const source = computed(() => {
  const n = Number(route.query.source || chat.selectedSessionId)
  return Number.isSafeInteger(n) && n > 0 ? n : null
})
const inventory = ref<PlatformInventory | null>(null),
  loading = ref(false),
  error = ref('')
const collapsed = ref<Record<string, boolean>>({})
let generation = 0,
  disposed = false,
  reading: AbortController | null = null

function kindLabel(kind: ConversationEntry['kind']) {
  return t(`platforms.kind_${kind}`)
}
function stateLabel(state: string) {
  return t(`integrations.state_${state}`)
}
function toggle(key: string) {
  collapsed.value = { ...collapsed.value, [key]: !collapsed.value[key] }
}
async function load() {
  if (!source.value || loading.value || disposed) return
  const id = source.value,
    gen = ++generation,
    controller = new AbortController()
  reading?.abort()
  reading = controller
  loading.value = true
  error.value = ''
  try {
    const next = await integrationApi.platforms(id, controller.signal)
    if (disposed || gen !== generation) return
    inventory.value = next
  } catch (e) {
    if (disposed || gen !== generation) return
    if (e instanceof Error && e.name === 'AbortError') return
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    if (gen === generation) loading.value = false
  }
}
watch(
  () => [visible.value, source.value] as const,
  ([isVisible]) => {
    if (isVisible) void load()
    else reading?.abort()
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  disposed = true
  ++generation
  reading?.abort()
})
defineExpose({ reload: load })
</script>

<template>
  <section class="space-y-3" data-testid="platform-inventory">
    <header class="flex items-center justify-between gap-2">
      <div>
        <h2 class="font-medium">{{ t('platforms.title') }}</h2>
        <p class="text-sm text-muted-foreground">{{ t('platforms.hint') }}</p>
      </div>
      <button
        type="button"
        class="rounded border border-border px-2 py-1 text-xs"
        :disabled="loading || !source"
        data-testid="platforms-reload"
        @click="load"
      >
        {{ t('platforms.reload') }}
      </button>
    </header>
    <p v-if="!source" class="text-sm text-muted-foreground">{{ t('platforms.needSource') }}</p>
    <p v-else-if="error" role="alert" class="text-sm text-destructive break-words">{{ error }}</p>
    <p
      v-else-if="inventory && !inventory.platforms.length"
      class="text-sm text-muted-foreground"
      data-testid="platforms-empty"
    >
      {{ t('platforms.empty') }}
    </p>
    <article
      v-for="entry in inventory?.platforms || []"
      :key="entry.platform"
      class="rounded border border-border p-3"
      :data-testid="`platform-${entry.platform}`"
    >
      <button
        type="button"
        class="flex w-full items-center justify-between gap-2 text-left"
        @click="toggle(entry.platform)"
      >
        <span>
          <span class="font-medium">{{ entry.title || entry.platform }}</span>
          <span class="ml-2 text-xs text-muted-foreground">{{ entry.platform }}</span>
        </span>
        <span class="text-xs text-muted-foreground">
          {{ t('platforms.conversations', { count: entry.conversation_count }) }} ·
          {{ t('platforms.groups', { count: entry.conversation_counts.group }) }} ·
          {{ t('platforms.directs', { count: entry.conversation_counts.direct }) }} ·
          {{ t('platforms.channels', { count: entry.conversation_counts.broadcast }) }}
        </span>
      </button>
      <div v-if="!collapsed[entry.platform]" class="mt-3 space-y-3">
        <div
          v-for="account in entry.accounts"
          :key="account.binding_id"
          class="rounded border border-border/60 p-2"
          :data-testid="`platform-account-${account.binding_id}`"
        >
          <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span>{{ account.tenant }} / {{ account.account }}</span>
            <span>· {{ stateLabel(account.state) }}</span>
            <span>· {{ t('platforms.selector', { selector: account.selector }) }}</span>
            <span v-if="account.last_error" class="text-destructive break-words">{{ account.last_error }}</span>
          </div>
          <p
            v-if="!account.conversations.length"
            class="mt-2 text-xs text-muted-foreground"
            data-testid="account-no-conversations"
          >
            {{ t('platforms.noConversations') }}
          </p>
          <ul v-else class="mt-2 space-y-1">
            <li
              v-for="conversation in account.conversations"
              :key="`${conversation.resource}:${conversation.thread || ''}:${conversation.session_id || ''}`"
              class="rounded px-2 py-1 text-sm hover:bg-muted"
              :data-testid="`conversation-${conversation.resource}`"
            >
              <div class="flex flex-wrap items-center gap-2">
                <span class="rounded bg-muted px-1.5 py-0.5 text-xs">{{ kindLabel(conversation.kind) }}</span>
                <span class="font-mono text-xs break-all">{{ conversation.resource }}</span>
                <span v-if="conversation.thread" class="text-xs text-muted-foreground">
                  {{ t('platforms.thread', { thread: conversation.thread }) }}
                </span>
                <span v-if="conversation.role" class="text-xs text-muted-foreground">
                  {{ t(`channelAccess.role_${conversation.role}`) }}
                </span>
              </div>
              <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <RouterLink
                  v-if="conversation.session_id"
                  :to="`/chat?sessionId=${conversation.session_id}`"
                  class="underline"
                  >{{ conversation.title || t('platforms.untitled') }}</RouterLink
                >
                <span v-else>{{ t('platforms.noSession') }}</span>
              </div>
            </li>
          </ul>
        </div>
      </div>
    </article>
  </section>
</template>
