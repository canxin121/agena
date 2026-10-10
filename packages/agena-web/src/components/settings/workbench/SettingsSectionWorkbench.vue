<script setup lang="ts">
import { computed } from 'vue'
import { resolveSettingsSubpage, type SettingsSubpageDefinition } from './settingsSectionNavigation'

const props = defineProps<{
  section: string
  pages: SettingsSubpageDefinition[]
  defaultPage: string
  activePage: string
}>()

// SettingsPage owns routing and remembered destinations for every section.
const activePage = computed(() => resolveSettingsSubpage(props.activePage, '', props.pages, props.defaultPage))
const activeDefinition = computed(() => props.pages.find((page) => page.id === activePage.value))
</script>

<template>
  <section class="grid min-w-0 gap-5">
    <header v-if="activeDefinition" class="min-w-0 space-y-1">
      <h1 class="text-lg font-semibold">{{ activeDefinition.label }}</h1>
      <p v-if="activeDefinition.description" class="max-w-3xl text-sm text-muted-foreground">
        {{ activeDefinition.description }}
      </p>
    </header>
    <div class="min-w-0">
      <slot :active-page="activePage" :active-definition="activeDefinition" />
    </div>
  </section>
</template>
