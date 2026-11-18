<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from 'vue'
import { RiArrowDownSLine, RiChat3Line } from '@remixicon/vue'
import GroupedPicker from '@/components/ui/GroupedPicker.vue'
import { listSessions, getSession } from '@/stores/chat/api'
import { listBots } from '@/lib/bots'
import { useChatStoreUnscoped } from '@/stores/chat'
import { settingsText as st } from '@/i18n/settingsText'
import { createSettingsSessionCatalog, sessionCatalogId, settingsSessionGroups } from './settingsSessionCatalog'

const props = defineProps<{ modelValue: string; disabled?: boolean }>()
const emit = defineEmits<{ select: [id: string] }>()
const chat = useChatStoreUnscoped()
const catalog = createSettingsSessionCatalog({
  listSessions,
  getSession,
  listBots,
  cacheMetadata: (sessions) => chat.cacheSessionMetadata(sessions),
})
const id = computed(() => sessionCatalogId(props.modelValue))
const selected = computed(() => {
  if (catalog.selected.value?.id === id.value) return catalog.selected.value
  return chat.getSessionById(id.value)
})
const label = computed(() => (id.value ? String(selected.value?.title || st('Session {id}', { id: id.value })) : ''))
const groups = computed(() =>
  settingsSessionGroups({
    sessions: catalog.sessions.value,
    bots: catalog.bots.value,
    selectedId: id.value,
    selected: selected.value,
    query: catalog.query.value,
    labels: {
      projects: st('Project conversations'),
      bots: st('AI Bots'),
      selected: st('Selected conversation'),
      session: (id) => st('Session {id}', { id }),
    },
  }),
)
const busy = computed(() => catalog.loading.value || catalog.botsLoading.value)
const errors = computed(() =>
  [
    catalog.error.value && st('Unable to load conversations.') + ' ' + catalog.error.value,
    catalog.botsError.value && st('Unable to load AI Bots.') + ' ' + catalog.botsError.value,
    catalog.selectedError.value && st('Unable to load the selected conversation.') + ' ' + catalog.selectedError.value,
  ]
    .filter(Boolean)
    .join(' '),
)
const helper = computed(() => errors.value || (busy.value ? st('Loading…') : ''))

function select(value: string) {
  const candidate = sessionCatalogId(value)
  if (props.disabled || !candidate || !groups.value.some((group) => group.items.some((item) => item.id === candidate)))
    return
  const session = catalog.sessions.value.find((session) => session.id === candidate)
  if (session) chat.cacheSessionMetadata([session])
  emit('select', candidate)
}
function opened(open: boolean) {
  if (open) void Promise.all([catalog.loadPage(), catalog.loadBots()])
}
watch(
  () => props.modelValue,
  (value) => void catalog.loadSelected(value),
  { immediate: true },
)
onBeforeUnmount(catalog.dispose)
</script>

<template>
  <div class="min-w-0 flex-1" data-permission-session-picker>
    <GroupedPicker
      block
      size="sm"
      trigger-class="text-[13px]"
      :icon="RiChat3Line"
      :label="label"
      :selected-id="id"
      :placeholder="$st('Select a conversation')"
      :title="$st('Session for this setting')"
      :groups="groups"
      :disabled="disabled"
      :query="catalog.query.value"
      filter-mode="external"
      :search-placeholder="$st('Search conversations…')"
      :empty-text="busy ? $st('Loading…') : $st('No conversations found.')"
      :helper-text="helper"
      :page="catalog.page.value"
      :page-count="catalog.pageCount.value"
      :page-loading="catalog.loading.value"
      :loading="busy && !groups.length"
      refreshable
      :refreshing="busy"
      :refresh-label="$st('Retry')"
      @select="select"
      @update:open="opened"
      @update:query="catalog.search"
      @request-page="catalog.loadPage"
      @refresh="catalog.refresh"
    >
      <template #trailing><RiArrowDownSLine class="h-4 w-4 shrink-0 text-muted-foreground" /></template>
    </GroupedPicker>
    <p v-if="catalog.selectedError.value" class="mt-1 text-[13px] text-destructive" role="alert">
      {{ $st('Unable to load the selected conversation.') }}
      <button
        type="button"
        class="underline"
        :disabled="disabled || catalog.selectedLoading.value"
        @click="catalog.loadSelected(id)"
      >
        {{ $st('Retry') }}
      </button>
    </p>
  </div>
</template>
