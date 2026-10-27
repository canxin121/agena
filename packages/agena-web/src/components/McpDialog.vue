<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { RiRefreshLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import Dialog from '@/components/ui/Dialog.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { mcpConnectionsApi } from '@/lib/mcpConnectionsApi'
import { normalizeMcpStatus, type McpStatusItem } from '@/lib/mcpStatus'
import { settingsText as st } from '@/i18n/settingsText'
import { useUiStore } from '@/stores/ui'

const ui = useUiStore()
const router = useRouter()
const { t } = useI18n()
const loading = ref(false)
const error = ref('')
const entries = ref<McpStatusItem[]>([])
const toolCount = computed(() => entries.value.filter((entry) => entry.status === 'connected').reduce((total, entry) => total + entry.toolCount, 0))
const labels = computed<Record<string, string>>(() => ({
  connected: st('Connected'), disconnected: st('Disconnected'), disabled: st('Disabled'), shadowed: st('Overridden'),
  bridge_disabled: st('MCP bridge disabled'), invalid: st('Invalid configuration'), authentication_required: st('Sign-in required'),
  pending_reload: st('Pending reload'),
}))
let generation = 0
let controller: AbortController | undefined
async function refresh() {
  const request = ++generation
  controller?.abort()
  const current = new AbortController()
  controller = current
  loading.value = true
  error.value = ''
  try {
    const data = await mcpConnectionsApi.list(current.signal)
    if (request === generation) entries.value = normalizeMcpStatus(data)
  } catch (reason) {
    if (request === generation && !current.signal.aborted) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally { if (request === generation) loading.value = false }
}
function manage(id?: string) {
  ui.setMcpDialogOpen(false)
  void router.push({ path: '/settings/mcp-connections', query: { view: 'connections', ...(id ? { connection: id } : {}) } })
}
watch(() => ui.isMcpDialogOpen, (open) => {
  if (open) void refresh()
  else { generation++; controller?.abort(); loading.value = false }
})
onBeforeUnmount(() => { generation++; controller?.abort() })
</script>

<template>
  <Dialog :open="ui.isMcpDialogOpen" :title="t('mcp.dialog.title')" :description="st('External MCP connections configured on this Agena server.')" max-width="max-w-[calc(100vw-2rem)] sm:max-w-lg" @update:open="(value) => ui.setMcpDialogOpen(value)">
    <div class="space-y-3">
      <div class="flex items-center gap-3 border-b border-border/50 pb-2"><IconButton variant="outline" size="md" :tooltip="String(loading ? t('mcp.dialog.actions.refreshing') : t('common.refresh'))" :aria-label="String(loading ? t('mcp.dialog.actions.refreshing') : t('common.refresh'))" :disabled="loading" @click="refresh"><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" /></IconButton><div class="text-xs text-muted-foreground"><span class="font-mono font-semibold text-foreground">{{ entries.length }}</span> {{ t('mcp.dialog.serversLabel') }} · <span class="font-mono font-semibold text-foreground">{{ toolCount }}</span> {{ t('mcp.dialog.toolsLabel') }}</div></div>
      <p v-if="loading" role="status" class="text-xs text-muted-foreground">{{ t('mcp.dialog.loading') }}</p>
      <p v-if="error" role="alert" class="text-xs text-destructive">{{ error }}</p>
      <p v-else-if="!loading && !entries.length" class="text-xs text-muted-foreground">{{ t('mcp.dialog.empty') }}</p>
      <div class="divide-y divide-border/60"><button v-for="entry in entries" :key="entry.id" type="button" class="flex w-full items-start justify-between gap-3 rounded-md px-1 py-3 text-left hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" :aria-label="st('Manage connection') + ': ' + entry.name" @click="manage(entry.id)"><span class="min-w-0"><span class="block break-words font-mono text-sm font-semibold">{{ entry.name }}</span><span class="mt-1 block text-[11px]" :class="entry.status === 'connected' ? 'text-success' : ['invalid', 'authentication_required', 'disconnected', 'pending_reload'].includes(entry.status) ? 'text-amber-700 dark:text-amber-300' : 'text-muted-foreground'">{{ labels[entry.status] || entry.status }} · {{ entry.scope === 'workspace' ? st('Workspace') : st('Global') }}</span><span v-if="entry.error" class="mt-1 line-clamp-2 block break-words text-[11px] text-muted-foreground">{{ entry.error }}</span></span><span class="shrink-0 font-mono text-xs text-muted-foreground">{{ entry.toolCount }} {{ st('Tools') }}</span></button></div>
    </div>
    <template #footer><Button class="ml-auto" variant="outline" size="sm" @click="manage()">{{ st('Manage MCP connections') }}</Button></template>
  </Dialog>
</template>
