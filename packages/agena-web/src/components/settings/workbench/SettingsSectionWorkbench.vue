<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { cn } from '@/lib/utils'
import { settingsQueryForDestination } from '../settingsDestination'
import type { SettingsTab } from '../sidebar/settingsSidebarNavigation'
import { buildSettingsScopes, findSettingsScope } from '../settingsScopeCatalog'
import { resolveSettingsSubpage, type SettingsSubpageDefinition } from './settingsSectionNavigation'
import SettingsProjectPicker from '../SettingsProjectPicker.vue'
import { useSettingsSessionTarget } from '../settingsSessionScope'
import { useDirectorySessionStore } from '@/stores/directorySessionStore'
import { useDirectoryStore } from '@/stores/directory'

const props = defineProps<{
  section: SettingsTab
  pages: SettingsSubpageDefinition[]
  defaultPage: string
  activePage: string
  // The route-backed configuration layer. The selector can sit beside the
  // content or in a compact toolbar above it.
  activeScope?: string
  scopeLayout?: 'sidebar' | 'toolbar'
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
// A session layer always names its conversation: settings edit the one this
// client used last, so there is nothing to pick here. A workspace layer names
// its project.
const sessionScope = computed(() => props.activeScope === 'session')
const sessionTarget = useSettingsSessionTarget()
const workspaceScope = computed(() => props.activeScope === 'workspace')
const directory = useDirectoryStore()
const directories = useDirectorySessionStore()

// Selecting a project makes that directory the active workspace, because the
// workspace configuration layer is always the composed runtime's project.
function selectProject(id: string) {
  const entry = directories.directoriesById[String(id || '').trim()]
  if (!entry?.path) return
  directory.setDirectory(entry.path)
}

// Sidebar tasks that contain several legacy pages expose their secondary
// destinations here rather than adding another sidebar navigation level.
const LOCAL_PAGE_GROUPS: Partial<Record<SettingsTab, string[][]>> = {
  'models-providers': [['model-settings', 'provider-studio', 'model-catalog']],
  'runtime-session': [['prompt-library', 'prompt-layers']],
  'plugins-tools': [['plugin-workbench', 'marketplace']],
  skills: [['library', 'install']],
}
const localPages = computed(() => {
  const ids = LOCAL_PAGE_GROUPS[props.section]?.find((group) => group.includes(activePage.value)) || []
  return ids.flatMap((id) => {
    const page = props.pages.find((candidate) => candidate.id === id)
    return page ? [page] : []
  })
})

function selectPage(view: string) {
  if (view === activePage.value) return
  const query = settingsQueryForDestination({ section: props.section, view }, props.section, route.query)
  void router.push({ path: route.path, query, hash: route.hash })
}

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
          scopes.length && scopeLayout !== 'toolbar'
            ? 'grid gap-4 lg:grid-cols-[14rem_minmax(0,1fr)] lg:gap-6'
            : fillsViewport
              ? 'flex h-full flex-col'
              : '',
        )
      "
    >
      <aside v-if="scopes.length && scopeLayout !== 'toolbar'" class="min-w-0">
        <div class="mb-2 px-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {{ $st('Configuration scope') }}
        </div>
        <nav
          class="flex gap-1 overflow-x-auto rounded-lg border border-border/60 bg-muted/10 p-1 lg:flex-col lg:overflow-visible"
          :class="scopes.length === 3 ? 'grid-cols-3' : 'grid-cols-2'"
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
            <span v-if="activeScopeLabel && scopeLayout !== 'toolbar'" class="font-normal text-muted-foreground"
              >· {{ activeScopeLabel }}</span
            >
          </h1>
          <p v-if="activeDefinition.description" class="max-w-3xl text-sm text-muted-foreground">
            {{ activeDefinition.description }}
          </p>
        </header>
        <nav
          v-if="localPages.length > 1"
          class="mb-4 flex flex-wrap gap-1 rounded-xl border border-border/60 bg-muted/20 p-1"
          :aria-label="$st('Related settings')"
        >
          <button
            v-for="page in localPages"
            :key="page.id"
            type="button"
            class="rounded-lg px-3 py-2 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :class="
              page.id === activePage
                ? 'bg-background text-foreground shadow-sm ring-1 ring-border/60'
                : 'text-muted-foreground hover:bg-muted/50 hover:text-foreground'
            "
            :aria-current="page.id === activePage ? 'page' : undefined"
            @click="selectPage(page.id)"
          >
            {{ page.label }}
          </button>
        </nav>
        <nav
          v-if="scopes.length && scopeLayout === 'toolbar'"
          class="mb-4 grid gap-1 rounded-xl border border-border/60 bg-muted/20 p-1 sm:flex sm:flex-wrap"
          :class="scopes.length === 3 ? 'grid-cols-3' : 'grid-cols-2'"
          :aria-label="$st('Configuration scope')"
        >
          <button
            v-for="scope in scopes"
            :key="scope.id"
            type="button"
            class="min-w-0 rounded-lg px-2 py-2 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:flex-1 sm:px-3"
            :class="
              scope.id === activeScope
                ? 'bg-background text-foreground shadow-sm ring-1 ring-border/60'
                : 'text-muted-foreground hover:bg-muted/50 hover:text-foreground'
            "
            :aria-current="scope.id === activeScope ? 'true' : undefined"
            :title="scope.description"
            @click="selectScope(scope.id)"
          >
            {{ scope.label }}
          </button>
        </nav>
        <!-- A session layer always edits the conversation this client used last;
             the page states which one so the layer selector is unambiguous. -->
        <div
          v-if="sessionScope"
          class="mb-4 flex min-w-0 flex-wrap items-center gap-2 rounded-xl border border-border/60 bg-muted/15 px-3 py-2"
          :aria-label="$st('Session for this setting')"
        >
          <span class="text-xs font-medium text-muted-foreground">{{ $st('Conversation') }}</span>
          <span class="min-w-0 truncate text-sm font-medium">
            {{ sessionTarget.selected.value?.title || $st('No conversation yet') }}
          </span>
          <span
            v-if="sessionTarget.selected.value?.project"
            class="min-w-0 truncate font-mono text-[10px] text-muted-foreground"
            :title="sessionTarget.selected.value.project"
          >
            {{ sessionTarget.selected.value.project }}
          </span>
          <span v-else class="min-w-0 text-xs text-muted-foreground">
            {{ $st('Open a conversation to set this layer.') }}
          </span>
        </div>
        <div
          v-else-if="workspaceScope"
          class="mb-4 flex min-w-0 flex-wrap items-center gap-2 rounded-xl border border-border/60 bg-muted/15 px-3 py-2"
        >
          <span class="text-xs font-medium text-muted-foreground">{{ $st('Project') }}</span>
          <SettingsProjectPicker class="min-w-0 flex-1" @select="selectProject" />
          <span
            v-if="directory.currentDirectory"
            class="min-w-0 truncate font-mono text-[10px] text-muted-foreground"
            :title="directory.currentDirectory"
            >{{ directory.currentDirectory }}</span
          >
        </div>
        <div :class="cn('min-w-0', fillsViewport ? 'min-h-0 flex-1' : '')">
          <slot :active-page="activePage" :active-definition="activeDefinition" />
        </div>
      </div>
    </div>
  </section>
</template>
