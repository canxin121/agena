<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiGlobalLine, RiHand, RiRefreshLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { useVisibleResource } from '@/pages/chat/useVisibleResource'
import type { MessageEntry } from '@/types/chat'
import { readBrowserState } from '@/features/browser/api'
import { isAcpSessionId } from '@/lib/acpSession'
import { useBrowserCopy } from '@/features/browser/copy'
import BrowserTabChip from '@/features/browser/BrowserTabChip.vue'
import SessionChildrenSection from './SessionChildrenSection.vue'
import SessionChip from './SessionChip.vue'
import SessionSchedules from './SessionSchedules.vue'
import SessionTerminalSection from './SessionTerminalSection.vue'

const props = defineProps<{
  sessionId: string
  messages: MessageEntry[]
  busy: boolean
  childCount?: number
  browserOpen: boolean
  browserRequested: boolean
}>()
const emit = defineEmits<{ browser: [pageId?: string] }>()
const schedulesOpen = defineModel<boolean>('schedulesOpen', { default: false })
const { t } = useI18n()
const browserText = useBrowserCopy()
const tabsOpen = ref(false)
const browser = useVisibleResource({
  key: computed(() => props.sessionId),
  minInterval: 800,
  interval: (): number => (props.busy || browser.data.value?.tabs.length || props.browserRequested ? 2500 : 30000),
  load: readBrowserState,
})
const tabs = computed(() => browser.data.value?.tabs ?? [])
const helpPage = computed(() => browser.data.value?.help?.page_id)
const browserLabel = computed(() =>
  tabs.value.length ? `${browserText('browser')} · ${tabs.value.length}` : browserText('browser'),
)

function openPage(pageId?: string) {
  tabsOpen.value = false
  emit('browser', pageId)
}

watch([() => props.busy, () => props.browserOpen, () => props.browserRequested], () => void browser.refresh())
watch(tabsOpen, (open) => {
  if (open) void browser.refresh()
})
</script>

<template>
  <SessionChildrenSection :session-id="sessionId" :child-count="childCount" :busy="busy" />
  <SessionTerminalSection :session-id="sessionId" :messages="messages" :busy="busy" />
  <SessionChip
    v-if="tabsOpen || tabs.length || browserRequested || browser.error.value"
    data-browser-chip
    :title="browserText('browser')"
    :label="browserLabel"
    :hint="browserRequested ? browserText('requested') : browserLabel"
    :open="tabsOpen"
    @update:open="tabsOpen = $event"
  >
    <template #icon>
      <RiHand v-if="browserRequested" class="h-3 w-3 shrink-0 text-primary" aria-hidden="true" />
      <RiGlobalLine v-else class="h-3 w-3 shrink-0" aria-hidden="true" />
    </template>
    <template #actions>
      <IconButton
        class="h-7 w-7"
        :tooltip="t('common.refresh')"
        :aria-label="t('common.refresh')"
        :disabled="browser.loading.value"
        @click="browser.refresh()"
      >
        <RiRefreshLine class="h-3.5 w-3.5" />
      </IconButton>
    </template>
    <p v-if="browser.error.value" role="alert" class="mb-1 break-words text-xs text-destructive">
      {{ browser.error.value }}
    </p>
    <Button
      v-if="browserRequested && !tabs.some((tab) => tab.page_id === helpPage)"
      data-keyboard-option
      size="sm"
      variant="ghost"
      class="text-primary"
      @click="openPage(helpPage)"
    >
      {{ browserText('requested') }}
    </Button>
    <p v-else-if="!tabs.length && !browser.error.value" class="text-xs text-muted-foreground">
      {{ browserText(browser.loading.value ? 'loading' : 'empty') }}
    </p>
    <BrowserTabChip
      v-for="tab in tabs"
      :key="tab.page_id"
      :session-id="sessionId"
      :tab="tab"
      :human="Boolean(browser.data.value?.human_control)"
      :requested="browserRequested && tab.page_id === helpPage"
      :viewer-open="browserOpen"
      @open="openPage"
    />
  </SessionChip>
  <SessionSchedules v-if="!isAcpSessionId(sessionId)" v-model:open="schedulesOpen" :session-id="sessionId" />
</template>
