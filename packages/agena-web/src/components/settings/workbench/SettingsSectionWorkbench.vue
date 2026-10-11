<script setup lang="ts">
import { computed } from 'vue'
import { cn } from '@/lib/utils'
import { findSettingsScope } from '../settingsScopeCatalog'
import { resolveSettingsSubpage, type SettingsSubpageDefinition } from './settingsSectionNavigation'

const props = defineProps<{
  section: string
  pages: SettingsSubpageDefinition[]
  defaultPage: string
  activePage: string
  // The configuration layer the page is showing. The left navigation owns the
  // choice, so the header names it once instead of every page repeating it.
  activeScope?: string
}>()

// SettingsPage owns routing and remembered destinations for every section.
const activePage = computed(() => resolveSettingsSubpage(props.activePage, '', props.pages, props.defaultPage))
const activeDefinition = computed(() => props.pages.find((page) => page.id === activePage.value))
const activeScopeLabel = computed(
  () => findSettingsScope(props.section, activePage.value, props.activeScope)?.label || '',
)
// A viewport-filling page keeps its own scrolling panes, so the workbench must
// stop contributing page scroll and hand it the remaining height instead.
const fillsViewport = computed(() => activeDefinition.value?.fillsViewport === true)
</script>

<template>
  <section :class="cn('min-w-0', fillsViewport ? 'flex flex-col gap-5 lg:h-full lg:min-h-0' : 'grid gap-5')">
    <header v-if="activeDefinition" class="min-w-0 space-y-1">
      <h1 class="text-lg font-semibold">
        {{ activeDefinition.label }}
        <span v-if="activeScopeLabel" class="font-normal text-muted-foreground">· {{ activeScopeLabel }}</span>
      </h1>
      <p v-if="activeDefinition.description" class="max-w-3xl text-sm text-muted-foreground">
        {{ activeDefinition.description }}
      </p>
    </header>
    <div :class="cn('min-w-0', fillsViewport ? 'lg:min-h-0 lg:flex-1' : '')">
      <slot :active-page="activePage" :active-definition="activeDefinition" />
    </div>
  </section>
</template>
