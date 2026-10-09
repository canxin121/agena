<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RiRefreshLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import Dialog from '@/components/ui/Dialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '@/lib/api'
import { normalizeMcpStatus, type McpStatusItem } from '@/lib/mcpStatus'
import { useUiStore } from '@/stores/ui'

type RuntimeStatus = {
  operator?: {
    mcp?: {
      server_count?: number
      tool_count?: number
      servers?: Array<{ name: string; tool_count: number }>
    }
  }
}

const ui = useUiStore()
const { t } = useI18n()

const loading = ref(false)
const error = ref('')
const runtime = ref<RuntimeStatus | null>(null)
const entries = ref<McpStatusItem[]>([])

const serverCount = computed(() => runtime.value?.operator?.mcp?.server_count ?? entries.value.length)
const toolCount = computed(
  () => runtime.value?.operator?.mcp?.tool_count ?? entries.value.reduce((total, entry) => total + entry.toolCount, 0),
)

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const data = await apiJson<RuntimeStatus>('/api/v1/runtime')
    runtime.value = data
    entries.value = normalizeMcpStatus(data)
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    runtime.value = null
    entries.value = []
  } finally {
    loading.value = false
  }
}

watch(
  () => ui.isMcpDialogOpen,
  (open) => {
    if (open) void refresh()
  },
)
</script>

<template>
  <Dialog
    :open="ui.isMcpDialogOpen"
    :title="t('mcp.dialog.title')"
    :description="t('mcp.dialog.runtimeDescription')"
    maxWidth="max-w-[calc(100vw-2rem)] sm:max-w-lg"
    @update:open="(value) => ui.setMcpDialogOpen(value)"
  >
    <div class="space-y-3">
      <div class="flex items-center gap-3 border-b border-border/50 pb-2">
        <IconButton
          variant="outline"
          size="md"
          :tooltip="String(loading ? t('mcp.dialog.actions.refreshing') : t('common.refresh'))"
          :aria-label="String(loading ? t('mcp.dialog.actions.refreshing') : t('common.refresh'))"
          :disabled="loading"
          @click="refresh"
        >
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
        <div class="text-xs text-muted-foreground">
          <span class="font-mono font-semibold text-foreground">{{ serverCount }}</span>
          {{ t('mcp.dialog.serversLabel') }} ·
          <span class="font-mono font-semibold text-foreground">{{ toolCount }}</span>
          {{ t('mcp.dialog.toolsLabel') }}
        </div>
      </div>

      <div v-if="loading" class="text-xs text-muted-foreground">{{ t('mcp.dialog.loading') }}</div>
      <div v-else-if="error" class="text-xs text-destructive">{{ error }}</div>
      <div v-else-if="entries.length === 0" class="text-xs text-muted-foreground">{{ t('mcp.dialog.empty') }}</div>

      <div v-else class="divide-y divide-border/60">
        <div v-for="entry in entries" :key="entry.name" class="flex items-center justify-between gap-3 py-2.5">
          <div class="min-w-0">
            <div class="break-words font-mono text-sm font-semibold">{{ entry.name }}</div>
            <div class="mt-0.5 text-[11px] text-success">
              {{ entry.status }}
            </div>
          </div>
          <div class="shrink-0 font-mono text-xs text-muted-foreground">{{ entry.toolCount }} tools</div>
        </div>
      </div>
    </div>
  </Dialog>
</template>
