<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiBrainAi3Line,
  RiCheckLine,
  RiEqualizerLine,
  RiLoader4Line,
  RiSettings3Line,
  RiStackLine,
} from '@remixicon/vue'

import OptionMenu from '@/components/ui/OptionMenu.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import { sessionApiPath } from '@/lib/acpSession'
import { apiJson } from '@/lib/api'
import { optionOpenEdge } from '@/lib/keyboardNavigation'
import { useToastsStore } from '@/stores/toasts'
import { useUiStore } from '@/stores/ui'
import type { ExternalAgent, ExternalAgentOption, ExternalAgentOptions } from '@/types/chat'

type OptionRow = ExternalAgentOption & { selected: string }

const props = withDefaults(
  defineProps<{
    sessionId: string
    label: string
    agent?: ExternalAgent
    liveOptions?: ExternalAgentOptions
    executionReady?: boolean
    modelChipTextClass?: string
    modeChipTextClass?: string
  }>(),
  {
    executionReady: true,
  },
)

const { t } = useI18n()
const ui = useUiStore()
const toasts = useToastsStore()

const options = ref<ExternalAgentOptions>()
const loading = ref(false)
const savingKind = ref('')
const error = ref('')

const openKind = ref<string | null>(null)
const openAt = ref<'selected' | 'first' | 'last'>('selected')
const searchQuery = ref('')
const activeAnchorEl = ref<HTMLElement | null>(null)

const modelTriggerRef = ref<HTMLElement | null>(null)
const thinkingTriggerRef = ref<HTMLElement | null>(null)
const modeTriggerRef = ref<HTMLElement | null>(null)

let loadController: AbortController | null = null
let requestToken = 0

const rows = computed<OptionRow[]>(() => {
  if (!options.value) return []
  const { selection, options: available } = options.value
  const advertised: OptionRow[] = available.map((option) => ({
    ...option,
    selected: selection[option.kind] ?? '',
  }))
  const missing: OptionRow[] = Object.entries(selection).flatMap(([kind, selected]) => {
    if (!selected || advertised.some((option) => option.kind === kind)) return []
    return [
      {
        id: kind,
        kind,
        type: 'select' as const,
        selected,
        name: kind.startsWith('config:') ? kind.slice(7) : kind,
        description: undefined,
        current_value: '',
        choices: [],
      },
    ]
  })
  return [...advertised, ...missing]
})

const modelRow = computed(() => rows.value.find((row) => row.kind === 'model') ?? null)
const thoughtRow = computed(() => rows.value.find((row) => row.kind === 'thought_level') ?? null)
const modeRow = computed(() => rows.value.find((row) => row.kind === 'mode') ?? null)
const extraRows = computed(() =>
  rows.value.filter((row) => row.kind !== 'model' && row.kind !== 'thought_level' && row.kind !== 'mode'),
)

function isBooleanOption(row: OptionRow): boolean {
  if (row.type === 'boolean') return true
  if (row.choices.length === 2) {
    const values = new Set(row.choices.map((c) => c.value.toLowerCase()))
    return values.has('true') && values.has('false')
  }
  return false
}

function effectiveValue(row: OptionRow): string {
  return row.selected || row.current_value || ''
}

function booleanEffectiveValue(row: OptionRow): boolean {
  return effectiveValue(row).toLowerCase() === 'true'
}

function displayLabelForRow(row: OptionRow): string {
  const value = effectiveValue(row)
  if (!value) return String(t('common.default'))
  return row.choices.find((choice) => choice.value === value)?.name || value
}

const modelDisplayLabel = computed(() => {
  if (modelRow.value) return displayLabelForRow(modelRow.value)
  if (loading.value) return String(t('chat.composer.status.loading'))
  return String(t('chat.composer.picker.modelTitle'))
})

const activeRow = computed<OptionRow | null>(() => {
  if (!openKind.value) return null
  return rows.value.find((row) => row.kind === openKind.value) ?? null
})

const menuTitle = computed(() => {
  if (activeRow.value?.name) return activeRow.value.name
  if (openKind.value === 'model') return String(t('chat.composer.picker.modelTitle'))
  if (openKind.value === 'thought_level') return String(t('chat.composer.picker.thinkingTitle'))
  return String(t('chat.composer.picker.optionsTitle'))
})

const menuSearchable = computed(() => {
  if (openKind.value === 'model') return true
  return (activeRow.value?.choices.length ?? 0) > 6
})

const menuSearchPlaceholder = computed(() => {
  if (openKind.value === 'model') return String(t('chat.composer.picker.searchModels'))
  if (openKind.value === 'thought_level') return String(t('chat.composer.picker.searchThinkingModes'))
  return String(t('chat.composer.picker.searchOptions'))
})

const menuEmptyText = computed(() => {
  if (error.value) return error.value
  if (openKind.value === 'model') return String(t('chat.composer.picker.emptyModels'))
  if (openKind.value === 'thought_level') return String(t('chat.composer.picker.emptyThinkingModes'))
  return String(t('chat.composer.picker.emptyOptions'))
})

const menuHelperText = computed(() => {
  if (error.value) return error.value
  if (activeRow.value?.description) return activeRow.value.description
  return ''
})

const menuGroups = computed<OptionMenuGroup[]>(() => {
  const row = activeRow.value
  if (!row) return []
  const query = searchQuery.value.trim().toLowerCase()
  const activeVal = effectiveValue(row)
  const items: OptionMenuItem[] = row.choices
    .filter((choice) => {
      if (!query) return true
      return `${choice.name} ${choice.value} ${choice.description || ''}`.toLowerCase().includes(query)
    })
    .map((choice) => ({
      id: `choice:${choice.value}`,
      label: choice.name,
      description: choice.description || (choice.value !== choice.name ? choice.value : undefined),
      checked: choice.value === activeVal,
      keywords: `${choice.name} ${choice.value} ${choice.description || ''}`,
      monospace: row.kind === 'model',
    }))

  if (row.selected && !row.choices.some((choice) => choice.value === row.selected)) {
    items.unshift({
      id: `choice:${row.selected}`,
      label: row.selected,
      checked: true,
      disabled: true,
      monospace: row.kind === 'model',
    })
  }

  const groups: OptionMenuGroup[] = []
  if (row.selected) {
    const defaultName = row.current_value
      ? row.choices.find((choice) => choice.value === row.current_value)?.name || row.current_value
      : ''
    const defaultLabel = String(t('chat.composer.picker.agentSettings'))
    if (!query || `${defaultLabel} ${defaultName} default`.toLowerCase().includes(query)) {
      groups.push({
        id: 'default',
        title: String(t('common.default')),
        items: [
          {
            id: 'reset',
            label: defaultLabel,
            description: defaultName || undefined,
            checked: false,
            keywords: `${defaultLabel} ${defaultName} default`,
          },
        ],
      })
    }
  }

  if (items.length) {
    groups.push({
      id: `options:${row.kind}`,
      title: row.name,
      subtitle: String(t('chat.composer.picker.availableCount', { count: items.length })),
      items,
    })
  }
  return groups
})

async function load(change?: { kind: string; value: string | null }) {
  const sid = props.sessionId
  if (!sid) return
  if (!change) {
    loadController?.abort()
  }
  const controller = new AbortController()
  if (!change) loadController = controller
  const token = ++requestToken
  loading.value = true
  if (change) savingKind.value = change.kind
  error.value = ''
  try {
    const next = await apiJson<ExternalAgentOptions>(`${sessionApiPath(sid)}/agent-options`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(change ? { change } : {}),
      signal: controller.signal,
    })
    if (controller.signal.aborted || props.sessionId !== sid) return
    if (token === requestToken || change) {
      options.value = next
    }
  } catch (reason) {
    if (controller.signal.aborted || props.sessionId !== sid) return
    const message = reason instanceof Error ? reason.message : String(reason)
    error.value = message
    if (change) {
      toasts.push('error', message)
    }
  } finally {
    if (!change && loadController === controller) loadController = null
    if (token === requestToken) {
      loading.value = false
      savingKind.value = ''
    }
  }
}

function closeMenu() {
  openKind.value = null
  searchQuery.value = ''
}

function setMenuOpen(next: boolean) {
  if (!next) closeMenu()
}

function anchorForKind(kind: string, event?: Event): HTMLElement | null {
  if (kind === 'model') return modelTriggerRef.value
  if (kind === 'thought_level') return thinkingTriggerRef.value
  if (kind === 'mode') return modeTriggerRef.value
  return event?.currentTarget instanceof HTMLElement ? event.currentTarget : null
}

function toggleOptionMenu(kind: string, event?: Event) {
  if (openKind.value === kind) {
    closeMenu()
    return
  }
  openAt.value = 'selected'
  searchQuery.value = ''
  activeAnchorEl.value = anchorForKind(kind, event)
  openKind.value = kind
  if (!options.value?.options.length) {
    void load()
  }
}

function handleChipKeydown(kind: string, event: KeyboardEvent) {
  const edge = optionOpenEdge(event)
  if (!edge) return
  event.preventDefault()
  event.stopPropagation()
  openAt.value = edge
  searchQuery.value = ''
  activeAnchorEl.value = anchorForKind(kind, event)
  openKind.value = kind
  if (!options.value?.options.length) {
    void load()
  }
}

function handleMenuSelect(item: OptionMenuItem) {
  const kind = openKind.value
  if (!kind) return
  closeMenu()
  if (item.id === 'reset') {
    void load({ kind, value: null })
    return
  }
  if (item.id.startsWith('choice:')) {
    const value = item.id.slice('choice:'.length)
    void load({ kind, value })
  }
}

function toggleBooleanOption(row: OptionRow) {
  if (loading.value) return
  const next = !booleanEffectiveValue(row)
  void load({ kind: row.kind, value: next ? 'true' : 'false' })
}

watch(
  () => props.liveOptions,
  (live) => {
    if (live) options.value = live
  },
  { immediate: true },
)

watch(
  () => [props.sessionId, props.executionReady] as const,
  ([sid, ready], [prevSid]) => {
    if (sid !== prevSid) {
      closeMenu()
      error.value = ''
      options.value = props.liveOptions
    }
    if (sid && ready && !options.value?.options?.length && !props.liveOptions?.options?.length) {
      void load()
    }
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  loadController?.abort()
  loadController = null
})
</script>

<template>
  <span class="inline-flex items-center gap-1 whitespace-nowrap">
    <!-- Model chip (matches Agena Chat model chip) -->
    <button
      ref="modelTriggerRef"
      type="button"
      data-oc-keyboard-tap="blur"
      class="pointer-events-auto flex min-h-7 touch-manipulation items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
      :class="openKind === 'model' ? 'bg-secondary/60 text-foreground' : ''"
      :title="modelRow ? `${modelRow.name}: ${modelDisplayLabel}` : t('chat.composer.picker.modelTitle')"
      :aria-label="t('chat.composer.picker.modelTitle')"
      aria-haspopup="dialog"
      :aria-expanded="openKind === 'model'"
      @mousedown.prevent
      @click.stop="toggleOptionMenu('model', $event)"
      @keydown="handleChipKeydown('model', $event)"
    >
      <RiLoader4Line v-if="savingKind === 'model' || (loading && !modelRow)" class="h-3 w-3 shrink-0 animate-spin" />
      <RiStackLine v-else class="h-3 w-3 shrink-0" />
      <span :class="modelChipTextClass || 'max-w-52 truncate font-mono text-[11px] font-medium'">
        {{ modelDisplayLabel }}
      </span>
    </button>

    <!-- Thinking level chip (matches Agena Chat thinking chip) -->
    <template v-if="thoughtRow">
      <span class="text-muted-foreground/50">|</span>
      <button
        ref="thinkingTriggerRef"
        type="button"
        data-oc-keyboard-tap="blur"
        class="pointer-events-auto flex min-h-7 touch-manipulation items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
        :class="openKind === 'thought_level' ? 'bg-secondary/60 text-foreground' : ''"
        :title="`${thoughtRow.name}: ${displayLabelForRow(thoughtRow)}`"
        :aria-label="thoughtRow.name || t('chat.composer.picker.thinkingTitle')"
        aria-haspopup="dialog"
        :aria-expanded="openKind === 'thought_level'"
        @mousedown.prevent
        @click.stop="toggleOptionMenu('thought_level', $event)"
        @keydown="handleChipKeydown('thought_level', $event)"
      >
        <RiLoader4Line v-if="savingKind === 'thought_level'" class="h-3 w-3 shrink-0 animate-spin" />
        <RiBrainAi3Line v-else class="h-3 w-3 shrink-0" />
        <span :class="modeChipTextClass || 'max-w-32 truncate font-mono text-[11px] font-medium'">
          {{ displayLabelForRow(thoughtRow) }}
        </span>
      </button>
    </template>

    <!-- Mode chip (matches Agena Chat mode/speed chip) -->
    <template v-if="modeRow">
      <span class="text-muted-foreground/50">|</span>
      <button
        ref="modeTriggerRef"
        type="button"
        data-oc-keyboard-tap="blur"
        class="pointer-events-auto flex min-h-7 touch-manipulation items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
        :class="openKind === 'mode' ? 'bg-secondary/60 text-foreground' : ''"
        :title="`${modeRow.name}: ${displayLabelForRow(modeRow)}`"
        :aria-label="modeRow.name"
        aria-haspopup="dialog"
        :aria-expanded="openKind === 'mode'"
        @mousedown.prevent
        @click.stop="toggleOptionMenu('mode', $event)"
        @keydown="handleChipKeydown('mode', $event)"
      >
        <RiLoader4Line v-if="savingKind === 'mode'" class="h-3 w-3 shrink-0 animate-spin" />
        <RiEqualizerLine v-else class="h-3 w-3 shrink-0" />
        <span :class="modeChipTextClass || 'max-w-32 truncate font-mono text-[11px] font-medium'">
          {{ displayLabelForRow(modeRow) }}
        </span>
      </button>
    </template>

    <!-- Extra agent-specific config options (only rendered if advertised beyond model/thought_level/mode) -->
    <template v-for="row in extraRows" :key="row.kind">
      <span class="text-muted-foreground/50">|</span>
      <button
        v-if="isBooleanOption(row)"
        type="button"
        data-oc-keyboard-tap="blur"
        class="pointer-events-auto flex min-h-7 touch-manipulation items-center gap-1 rounded px-1 py-0.5 hover:bg-secondary/50"
        :class="booleanEffectiveValue(row) ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'"
        :title="row.description || row.name"
        :aria-label="row.name"
        :aria-pressed="booleanEffectiveValue(row)"
        :disabled="loading"
        @mousedown.prevent
        @click.stop="toggleBooleanOption(row)"
      >
        <RiLoader4Line v-if="savingKind === row.kind" class="h-3 w-3 shrink-0 animate-spin" />
        <RiCheckLine v-else-if="booleanEffectiveValue(row)" class="h-3 w-3 shrink-0 text-primary" />
        <RiSettings3Line v-else class="h-3 w-3 shrink-0" />
        <span :class="modeChipTextClass || 'max-w-32 truncate font-mono text-[11px] font-medium'">
          {{ row.name }}
        </span>
      </button>

      <button
        v-else
        type="button"
        data-oc-keyboard-tap="blur"
        class="pointer-events-auto flex min-h-7 touch-manipulation items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
        :class="openKind === row.kind ? 'bg-secondary/60 text-foreground' : ''"
        :title="`${row.name}: ${displayLabelForRow(row)}`"
        :aria-label="row.name"
        aria-haspopup="dialog"
        :aria-expanded="openKind === row.kind"
        @mousedown.prevent
        @click.stop="toggleOptionMenu(row.kind, $event)"
        @keydown="handleChipKeydown(row.kind, $event)"
      >
        <RiLoader4Line v-if="savingKind === row.kind" class="h-3 w-3 shrink-0 animate-spin" />
        <RiSettings3Line v-else class="h-3 w-3 shrink-0" />
        <span :class="modeChipTextClass || 'max-w-32 truncate font-mono text-[11px] font-medium'">
          {{ displayLabelForRow(row) }}
        </span>
      </button>
    </template>

    <!-- Single-level direct OptionMenu matching Agena Chat composer picker -->
    <OptionMenu
      :open="Boolean(openKind)"
      :open-at="openAt"
      :close-on-select="true"
      :query="searchQuery"
      :groups="menuGroups"
      :title="menuTitle"
      :mobile-title="menuTitle"
      :searchable="menuSearchable"
      :search-placeholder="menuSearchPlaceholder"
      :empty-text="menuEmptyText"
      :helper-text="menuHelperText"
      :loading="loading && !activeRow"
      :refreshable="true"
      :on-refresh="() => load()"
      :is-compact-touch="ui.isCompactTouch"
      :desktop-fixed="true"
      :desktop-anchor-el="activeAnchorEl"
      :desktop-gap-px="10"
      :desktop-viewport-margin-px="8"
      desktop-placement="top-start"
      desktop-class="w-[min(420px,calc(100vw-1rem))]"
      filter-mode="external"
      @update:open="setMenuOpen"
      @update:query="(v) => (searchQuery = v)"
      @select="handleMenuSelect"
    />
  </span>
</template>
