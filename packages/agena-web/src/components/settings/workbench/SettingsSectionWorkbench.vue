<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import {
  filterSettingsSubpages,
  resolveSettingsSubpage,
  settingsSubpageStorageKey,
  type SettingsSubpageDefinition,
} from './settingsSectionNavigation'

const props = withDefaults(
  defineProps<{
    section: string
    title: string
    description: string
    pages: SettingsSubpageDefinition[]
    defaultPage: string
  }>(),
  {
    pages: () => [],
  },
)

const route = useRoute()
const router = useRouter()
const query = ref('')

function rememberedPage(): string {
  try {
    return localStorage.getItem(settingsSubpageStorageKey(props.section)) || ''
  } catch {
    return ''
  }
}

const activePage = ref(resolveSettingsSubpage(route.query.view, rememberedPage(), props.pages, props.defaultPage))

const visiblePages = computed(() => filterSettingsSubpages(props.pages, query.value))
const activeDefinition = computed(
  () => props.pages.find((page) => page.id === activePage.value) || props.pages[0] || null,
)
const pickerOptions = computed(() =>
  props.pages.map((page) => ({
    value: page.id,
    label: page.label,
    description: page.description,
  })),
)

function remember(value: string) {
  try {
    localStorage.setItem(settingsSubpageStorageKey(props.section), value)
  } catch {
    // Browser storage can be unavailable in private or embedded contexts.
  }
}

function selectPage(value: string) {
  const resolved = resolveSettingsSubpage(value, '', props.pages, props.defaultPage)
  if (!resolved || resolved === activePage.value) return
  activePage.value = resolved
}

async function synchronizeRoute(value: string) {
  const current = String(route.query.view || '').trim()
  if (current === value) return
  await router.replace({
    path: route.path,
    query: { ...route.query, view: value },
    hash: route.hash,
  })
}

watch(
  () => route.query.view,
  (value) => {
    const resolved = resolveSettingsSubpage(value, rememberedPage(), props.pages, props.defaultPage)
    if (resolved && resolved !== activePage.value) activePage.value = resolved
  },
)

watch(
  () => [props.section, props.defaultPage, props.pages.map((page) => page.id).join('|')] as const,
  () => {
    const resolved = resolveSettingsSubpage(route.query.view, rememberedPage(), props.pages, props.defaultPage)
    if (resolved) activePage.value = resolved
  },
)

watch(
  activePage,
  (value) => {
    if (!value) return
    remember(value)
    void synchronizeRoute(value)
  },
  { immediate: true },
)

onMounted(() => {
  if (!activePage.value) {
    activePage.value = resolveSettingsSubpage('', rememberedPage(), props.pages, props.defaultPage)
  }
})
</script>

<template>
  <section class="grid min-w-0 gap-5">
    <header class="flex flex-wrap items-start justify-between gap-3 border-b border-border/60 pb-5">
      <div class="min-w-0">
        <h1 class="text-xl font-semibold tracking-tight">{{ title }}</h1>
        <p class="mt-1 max-w-3xl text-sm leading-6 text-muted-foreground">{{ description }}</p>
      </div>
      <slot name="actions" :active-page="activePage" />
    </header>

    <div class="grid min-w-0 gap-5 lg:grid-cols-[16rem_minmax(0,1fr)] lg:items-start">
      <aside class="hidden min-w-0 rounded-lg border border-border/60 bg-muted/10 p-2 lg:block">
        <SearchInput
          v-model="query"
          class="mb-2"
          input-class="h-8 text-xs"
          placeholder="Search this section"
          :show-search-button="false"
          input-aria-label="Search this settings section"
        />
        <nav class="grid gap-1" :aria-label="`${title} pages`">
          <button
            v-for="page in visiblePages"
            :key="page.id"
            type="button"
            class="grid min-w-0 gap-0.5 rounded-md px-3 py-2.5 text-left transition-colors"
            :class="
              activePage === page.id
                ? 'bg-primary/10 text-foreground ring-1 ring-primary/20'
                : 'text-foreground/80 hover:bg-muted/60 hover:text-foreground'
            "
            @click="selectPage(page.id)"
          >
            <span class="flex min-w-0 items-center justify-between gap-2">
              <span class="truncate text-sm font-medium">{{ page.label }}</span>
              <span
                v-if="page.badge"
                class="shrink-0 rounded-full bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground"
              >
                {{ page.badge }}
              </span>
            </span>
            <span class="line-clamp-2 text-[11px] leading-4 text-muted-foreground">{{ page.description }}</span>
          </button>
        </nav>
        <div v-if="visiblePages.length === 0" class="px-3 py-8 text-center text-xs text-muted-foreground">
          No matching settings pages.
        </div>
      </aside>

      <div class="grid min-w-0 gap-4">
        <div class="grid gap-2 lg:hidden">
          <OptionPicker
            :model-value="activePage"
            :options="pickerOptions"
            :title="title"
            :include-empty="false"
            search-placeholder="Search pages..."
            @update:model-value="selectPage"
          />
          <p v-if="activeDefinition" class="text-xs leading-5 text-muted-foreground">
            {{ activeDefinition.description }}
          </p>
        </div>

        <div class="min-w-0">
          <slot :active-page="activePage" :active-definition="activeDefinition" :select-page="selectPage" />
        </div>
      </div>
    </div>
  </section>
</template>
