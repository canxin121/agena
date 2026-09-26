<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { RiArrowLeftSLine, RiArrowRightSLine, RiBookOpenLine, RiCloseLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { apiJson } from '@/lib/api'
import type { CatalogPickerModel } from '@/components/settings/providerModelCatalogTypes'

const props = defineProps<{
  initialQuery: string
  selectedId?: string
  suggestedId?: string
  disabled?: boolean
}>()
const emit = defineEmits<{
  (e: 'select', model: CatalogPickerModel): void
  (e: 'close'): void
}>()

const PAGE_SIZE = 20
const query = ref(props.initialQuery)
const appliedQuery = ref(props.initialQuery)
const offset = ref(0)
const total = ref(0)
const items = ref<CatalogPickerModel[]>([])
const loading = ref(false)
const error = ref('')
let generation = 0

const page = computed(() => Math.floor(offset.value / PAGE_SIZE) + 1)
const pages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))

function formatLimit(value: number | null | undefined): string {
  return value ? Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(value) : '—'
}

async function load(nextOffset = 0) {
  const request = ++generation
  offset.value = nextOffset
  loading.value = true
  error.value = ''
  try {
    const params = new URLSearchParams({ q: appliedQuery.value, offset: String(nextOffset), limit: String(PAGE_SIZE) })
    const response = await apiJson<{ items?: CatalogPickerModel[]; total?: number }>(`/api/v1/model-catalog?${params}`)
    if (request !== generation) return
    items.value = Array.isArray(response.items) ? response.items : []
    total.value = Number(response.total) || 0
  } catch (reason) {
    if (request !== generation) return
    error.value = reason instanceof Error ? reason.message : String(reason)
    items.value = []
    total.value = 0
  } finally {
    if (request === generation) loading.value = false
  }
}

function search(value: string) {
  appliedQuery.value = value.trim()
  void load(0)
}

onMounted(() => void load())
onBeforeUnmount(() => ++generation)
</script>

<template>
  <section
    class="rounded-lg border border-primary/25 bg-background/90 shadow-sm"
    :aria-label="st('Choose catalog model')"
    :aria-busy="disabled"
  >
    <div class="flex items-start justify-between gap-3 border-b border-border/60 px-3 py-2.5">
      <div class="flex min-w-0 items-start gap-2.5">
        <span class="mt-0.5 rounded-md bg-primary/10 p-1.5 text-primary"><RiBookOpenLine class="h-4 w-4" /></span>
        <div>
          <div class="text-sm font-medium">{{ st('Choose catalog model') }}</div>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {{ st('Search by ID, name, origin, or description. Choosing a model fills the editable fields.') }}
          </p>
        </div>
      </div>
      <IconButton
        variant="ghost"
        size="sm"
        :tooltip="st('Close')"
        :aria-label="st('Close')"
        :disabled="disabled"
        @click="emit('close')"
      >
        <RiCloseLine class="h-4 w-4" />
      </IconButton>
    </div>

    <div class="space-y-2 p-3">
      <SearchInput
        v-model="query"
        :disabled="disabled"
        :placeholder="st('Search model catalog')"
        :input-aria-label="st('Search model catalog')"
        :input-title="st('Search model catalog')"
        :search-aria-label="st('Search')"
        :search-title="st('Search')"
        :clear-aria-label="st('Clear')"
        :clear-title="st('Clear')"
        @search="search"
        @clear="search('')"
      />
      <div v-if="error" class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">{{ error }}</div>
      <div v-else-if="loading" class="px-2 py-5 text-center text-xs text-muted-foreground">
        {{ st('Loading catalog models…') }}
      </div>
      <div v-else-if="items.length === 0" class="px-2 py-5 text-center text-xs text-muted-foreground">
        {{ st('No catalog models found. Try another search.') }}
      </div>
      <div v-else class="max-h-72 space-y-1 overflow-y-auto pr-1">
        <button
          v-for="item in items"
          :key="item.model_id"
          type="button"
          :disabled="disabled"
          class="flex w-full items-start justify-between gap-3 rounded-md border px-3 py-2 text-left transition-colors hover:border-primary/50 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :class="item.model_id === selectedId ? 'border-primary/55 bg-primary/5' : 'border-border/60'"
          @click="emit('select', item)"
        >
          <span class="min-w-0">
            <span class="block truncate text-sm font-medium">{{ item.display_name || item.model_id }}</span>
            <span class="block truncate font-mono text-[11px] text-muted-foreground">{{ item.model_id }}</span>
            <span v-if="item.description" class="mt-0.5 block truncate text-xs text-muted-foreground">{{
              item.description
            }}</span>
          </span>
          <span class="shrink-0 text-right text-[11px] text-muted-foreground">
            <span
              v-if="item.model_id === selectedId"
              class="mb-1 block rounded bg-primary/10 px-1.5 py-0.5 text-primary"
              >{{ st('Selected') }}</span
            >
            <span
              v-if="item.model_id === suggestedId"
              class="mb-1 block rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300"
              >{{ st('Suggested') }}</span
            >
            <span class="block">{{ item.origin || '—' }}</span>
            <span class="block">{{ formatLimit(item.context_window_tokens) }} {{ st('context') }}</span>
          </span>
        </button>
      </div>
      <div class="flex items-center justify-between border-t border-border/60 pt-2 text-xs text-muted-foreground">
        <span>{{ st('{total} catalog models', { total }) }}</span>
        <div class="flex items-center gap-2">
          <span>{{ page }} / {{ pages }}</span>
          <IconButton
            variant="ghost"
            size="sm"
            :tooltip="st('Previous')"
            :aria-label="st('Previous')"
            :disabled="disabled || loading || offset === 0"
            @click="load(Math.max(0, offset - PAGE_SIZE))"
            ><RiArrowLeftSLine class="h-4 w-4"
          /></IconButton>
          <IconButton
            variant="ghost"
            size="sm"
            :tooltip="st('Next')"
            :aria-label="st('Next')"
            :disabled="disabled || loading || offset + PAGE_SIZE >= total"
            @click="load(offset + PAGE_SIZE)"
            ><RiArrowRightSLine class="h-4 w-4"
          /></IconButton>
        </div>
      </div>
    </div>
  </section>
</template>
