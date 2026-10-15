<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { cn } from '@/lib/utils'
import { buildSettingsScopes, findSettingsScope } from '../settingsScopeCatalog'
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

const route = useRoute()
const router = useRouter()

// SettingsPage owns the selected page and remembered destinations; the scope
// selector keeps its choice shareable by updating the route query.
const activePage = computed(() => resolveSettingsSubpage(props.activePage, '', props.pages, props.defaultPage))
const activeDefinition = computed(() => props.pages.find((page) => page.id === activePage.value))
const scopes = computed(() => buildSettingsScopes(props.section, activePage.value))
const activeScopeLabel = computed(
  () => findSettingsScope(props.section, activePage.value, props.activeScope)?.label || '',
)
// A viewport-filling page keeps its own scrolling panes, so the workbench must
// stop contributing page scroll and hand it the remaining height instead.
const fillsViewport = computed(() => activeDefinition.value?.fillsViewport === true)

function selectScope(scope: string) {
  if (!scopes.value.some((candidate) => candidate.id === scope) || props.activeScope === scope) return
  void router.replace({ path: route.path, query: { ...route.query, scope }, hash: route.hash })
}
</script>

<template>
  <section :class="cn('min-w-0', fillsViewport ? 'lg:h-full lg:min-h-0' : '')">
    <div
      :class="
        cn(
          'min-w-0',
          scopes.length
            ? 'grid gap-4 lg:grid-cols-[14rem_minmax(0,1fr)] lg:gap-6'
            : fillsViewport
              ? 'flex h-full flex-col'
              : '',
        )
      "
    >
      <aside v-if="scopes.length" class="min-w-0">
        <div class="mb-2 px-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {{ $st('Configuration scope') }}
        </div>
        <nav
          class="flex gap-1 overflow-x-auto rounded-lg border border-border/60 bg-muted/10 p-1 lg:flex-col lg:overflow-visible"
          :aria-label="$st('Configuration scope')"
        >
          <button
            v-for="scope in scopes"
            :key="scope.id"
            type="button"
            class="min-w-max rounded-md px-3 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring lg:min-w-0"
            :class="
              scope.id === activeScope
                ? 'bg-primary/10 text-foreground ring-1 ring-inset ring-primary/20'
                : 'text-foreground/80 hover:bg-muted/60'
            "
            :aria-current="scope.id === activeScope ? 'true' : undefined"
            @click="selectScope(scope.id)"
          >
            <span class="block text-sm font-medium">{{ scope.label }}</span>
            <span class="mt-0.5 hidden text-xs leading-relaxed text-muted-foreground lg:block">{{
              scope.description
            }}</span>
          </button>
        </nav>
      </aside>

      <div :class="cn('min-w-0', fillsViewport ? 'flex min-h-0 flex-1 flex-col' : '')">
        <header v-if="activeDefinition" class="mb-5 min-w-0 space-y-1">
          <h1 class="text-lg font-semibold">
            {{ activeDefinition.label }}
            <span v-if="activeScopeLabel" class="font-normal text-muted-foreground">· {{ activeScopeLabel }}</span>
          </h1>
          <p v-if="activeDefinition.description" class="max-w-3xl text-sm text-muted-foreground">
            {{ activeDefinition.description }}
          </p>
        </header>
        <div :class="cn('min-w-0', fillsViewport ? 'min-h-0 flex-1' : '')">
          <slot :active-page="activePage" :active-definition="activeDefinition" />
        </div>
      </div>
    </div>
  </section>
</template>
