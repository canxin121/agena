<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RiCloseLine, RiLoader4Line, RiRefreshLine } from '@remixicon/vue'

import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '@/lib/api'
import { normalizeLspRuntimeList, type LspRuntimeItem } from '@/lib/lspRuntime'
import { normalizeMcpStatus, type McpStatusItem } from '@/lib/mcpStatus'

type RuntimeStatus = {
  operator?: {
    mcp?: { server_count?: number; tool_count?: number; servers?: unknown[] }
    lsp?: {
      server_count?: number
      diagnostics_count?: number
      files_with_diagnostics?: number
      servers?: unknown[]
    }
  }
}

const props = withDefaults(defineProps<{ isCompactTouch?: boolean }>(), { isCompactTouch: false })
const emit = defineEmits<{ (event: 'reserve-change', px: number): void }>()

const rootEl = ref<HTMLElement | null>(null)
const runtime = ref<RuntimeStatus | null>(null)
const loading = ref(false)
const error = ref('')
const activePanel = ref<'mcp' | 'lsp' | null>(null)

let pollId: number | null = null
let resizeObserver: ResizeObserver | null = null

const mcpItems = computed<McpStatusItem[]>(() => normalizeMcpStatus(runtime.value))
const lspItems = computed<LspRuntimeItem[]>(() => normalizeLspRuntimeList(runtime.value || {}))
const mcpToolCount = computed(
  () => runtime.value?.operator?.mcp?.tool_count ?? mcpItems.value.reduce((total, item) => total + item.toolCount, 0),
)
const lspDiagnosticsCount = computed(() => runtime.value?.operator?.lsp?.diagnostics_count ?? 0)
const lspFilesCount = computed(() => runtime.value?.operator?.lsp?.files_with_diagnostics ?? 0)

function badge(value: number): string {
  if (value <= 0) return ''
  return value > 99 ? '99+' : String(value)
}

function updateReserve() {
  if (!activePanel.value || !rootEl.value) {
    emit('reserve-change', 0)
    return
  }
  const height = rootEl.value.getBoundingClientRect().height
  emit('reserve-change', Number.isFinite(height) ? Math.max(0, Math.ceil(height + 8)) : 0)
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    runtime.value = await apiJson<RuntimeStatus>('/api/v1/runtime')
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    runtime.value = null
  } finally {
    loading.value = false
    await nextTick()
    updateReserve()
  }
}

function togglePanel(panel: 'mcp' | 'lsp') {
  activePanel.value = activePanel.value === panel ? null : panel
  if (activePanel.value) void refresh()
}

watch(activePanel, async () => {
  await nextTick()
  updateReserve()
})

onMounted(() => {
  void refresh()
  pollId = window.setInterval(() => void refresh(), 30_000)
  if (typeof ResizeObserver !== 'undefined') {
    resizeObserver = new ResizeObserver(updateReserve)
    if (rootEl.value) resizeObserver.observe(rootEl.value)
  }
})

onBeforeUnmount(() => {
  if (pollId !== null) window.clearInterval(pollId)
  pollId = null
  resizeObserver?.disconnect()
  resizeObserver = null
  emit('reserve-change', 0)
})
</script>

<template>
  <div ref="rootEl" class="pointer-events-none flex w-full flex-col items-end gap-2">
    <section
      v-if="activePanel"
      class="pointer-events-auto flex max-h-[min(56dvh,32rem)] w-full flex-col overflow-hidden rounded-md border border-border/70 bg-background/95 shadow-lg backdrop-blur"
      :class="props.isCompactTouch ? '' : 'max-w-xl'"
    >
      <header class="flex items-center justify-between gap-2 border-b border-border/60 px-3 py-2">
        <div class="min-w-0">
          <h3 class="text-xs font-semibold">{{ activePanel === 'mcp' ? 'MCP runtime' : 'LSP runtime' }}</h3>
          <p class="mt-0.5 text-[11px] text-muted-foreground">
            <template v-if="activePanel === 'mcp'"> {{ mcpItems.length }} servers · {{ mcpToolCount }} tools </template>
            <template v-else>
              {{ lspItems.length }} servers · {{ lspDiagnosticsCount }} diagnostics in {{ lspFilesCount }} files
            </template>
          </p>
        </div>
        <div class="flex items-center gap-1">
          <IconButton
            size="sm"
            tooltip="Refresh runtime status"
            aria-label="Refresh runtime status"
            :disabled="loading"
            @click="refresh"
          >
            <RiLoader4Line v-if="loading" class="h-4 w-4 animate-spin" />
            <RiRefreshLine v-else class="h-4 w-4" />
          </IconButton>
          <IconButton size="sm" tooltip="Close" aria-label="Close" @click="activePanel = null">
            <RiCloseLine class="h-4 w-4" />
          </IconButton>
        </div>
      </header>

      <div v-if="error" class="px-3 py-5 text-xs text-destructive">{{ error }}</div>
      <div v-else-if="activePanel === 'mcp' && mcpItems.length === 0" class="px-3 py-5 text-xs text-muted-foreground">
        No MCP servers are loaded.
      </div>
      <div v-else-if="activePanel === 'lsp' && lspItems.length === 0" class="px-3 py-5 text-xs text-muted-foreground">
        No LSP servers are configured.
      </div>

      <div v-else class="min-h-0 overflow-y-auto divide-y divide-border/50 px-3">
        <div v-if="activePanel === 'mcp'">
          <div v-for="server in mcpItems" :key="server.name" class="flex items-center justify-between gap-3 py-2.5">
            <div class="min-w-0">
              <div class="truncate font-mono text-xs font-semibold" :title="server.name">{{ server.name }}</div>
              <div class="mt-0.5 text-[10px] text-success">{{ server.status }}</div>
            </div>
            <div class="shrink-0 font-mono text-[11px] text-muted-foreground">{{ server.toolCount }} tools</div>
          </div>
        </div>
        <div v-else>
          <div v-for="server in lspItems" :key="server.id" class="py-2.5">
            <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span class="font-mono text-xs font-semibold">{{ server.name }}</span>
              <span class="text-[10px] text-success">{{ server.status }}</span>
            </div>
            <div class="mt-1 break-all font-mono text-[10px] text-muted-foreground">{{ server.transport }}</div>
            <div v-if="server.fileExtensions.length" class="mt-1 text-[10px] text-muted-foreground">
              {{ server.fileExtensions.join(', ') }}
            </div>
          </div>
        </div>
      </div>
    </section>

    <div class="pointer-events-auto flex items-center gap-1.5">
      <button
        type="button"
        class="relative inline-flex h-8 min-w-8 items-center justify-center rounded-md border border-border/60 bg-background/85 px-1.5 text-[9px] font-semibold text-muted-foreground shadow-sm backdrop-blur hover:text-foreground"
        :aria-expanded="activePanel === 'lsp'"
        title="LSP runtime status"
        aria-label="LSP runtime status"
        @click="togglePanel('lsp')"
      >
        LSP
        <span
          v-if="badge(lspDiagnosticsCount)"
          class="absolute -right-1 -top-1 min-w-4 rounded-full bg-primary px-1 text-[9px] leading-4 text-primary-foreground"
        >
          {{ badge(lspDiagnosticsCount) }}
        </span>
      </button>
      <button
        type="button"
        class="relative inline-flex h-8 min-w-8 items-center justify-center rounded-md border border-border/60 bg-background/85 px-1.5 text-[9px] font-semibold text-muted-foreground shadow-sm backdrop-blur hover:text-foreground"
        :aria-expanded="activePanel === 'mcp'"
        title="MCP runtime status"
        aria-label="MCP runtime status"
        @click="togglePanel('mcp')"
      >
        MCP
        <span
          v-if="badge(mcpItems.length)"
          class="absolute -right-1 -top-1 min-w-4 rounded-full bg-primary px-1 text-[9px] leading-4 text-primary-foreground"
        >
          {{ badge(mcpItems.length) }}
        </span>
      </button>
    </div>
  </div>
</template>
