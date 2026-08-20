<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import {
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

function rememberedPage(): string {
  try {
    return localStorage.getItem(settingsSubpageStorageKey(props.section)) || ''
  } catch {
    return ''
  }
}

const activePage = ref(resolveSettingsSubpage(route.query.view, rememberedPage(), props.pages, props.defaultPage))

const activeDefinition = computed(
  () => props.pages.find((page) => page.id === activePage.value) || props.pages[0] || null,
)

function remember(value: string) {
  try {
    localStorage.setItem(settingsSubpageStorageKey(props.section), value)
  } catch {
    // Browser storage can be unavailable in private or embedded contexts.
  }
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

    <div class="min-w-0">
      <slot :active-page="activePage" :active-definition="activeDefinition" />
    </div>
  </section>
</template>
