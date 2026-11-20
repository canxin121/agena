<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { RiFolderLine } from '@remixicon/vue'
import { useRoute, useRouter } from 'vue-router'
import { settingsQueryForDestination } from '../settingsDestination'
import type { SettingsTab } from '../sidebar/settingsSidebarNavigation'
import { buildSettingsScopes, findSettingsScope } from '../settingsScopeCatalog'
import { resolveSettingsSubpage, type SettingsSubpageDefinition } from './settingsSectionNavigation'
import SettingsProjectPicker from '../SettingsProjectPicker.vue'
import SettingsSessionPicker from '../SettingsSessionPicker.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { useSettingsSessionTarget } from '../settingsSessionScope'
import { useDirectorySessionStore } from '@/stores/directorySessionStore'
import { useDirectoryStore } from '@/stores/directory'

const props = defineProps<{
  section: SettingsTab
  pages: SettingsSubpageDefinition[]
  defaultPage: string
  activePage: string
  activeScope?: string
  workspaceConfigPath?: string
}>()

const route = useRoute()
const router = useRouter()
const root = ref<HTMLElement | null>(null)

// SettingsPage owns the selected page and remembered destinations; the scope
// selector keeps its choice shareable by updating the route query.
const activePage = computed(() => resolveSettingsSubpage(props.activePage, '', props.pages, props.defaultPage))
const activeDefinition = computed(() => props.pages.find((page) => page.id === activePage.value))
const scopes = computed(() =>
  buildSettingsScopes(props.section, activePage.value).map(({ id, label, description }) => ({
    value: id,
    label,
    description,
  })),
)
const activeScopeLabel = computed(
  () => findSettingsScope(props.section, activePage.value, props.activeScope)?.label || '',
)
const fillsViewport = computed(() => activeDefinition.value?.fillsViewport === true)
const sessionTarget = useSettingsSessionTarget()
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
  void router.push({ path: route.path, query })
}

function selectScope(scope: string) {
  if (!scopes.value.some((candidate) => candidate.value === scope) || props.activeScope === scope) return
  void router.replace({ path: route.path, query: { ...route.query, scope }, hash: route.hash })
}

watch(
  [activePage, () => route.hash],
  async ([, hash]) => {
    if (!hash.startsWith('#settings-')) return
    await nextTick()
    const target = root.value?.querySelector<HTMLElement>('#' + CSS.escape(hash.slice(1)))
    target?.scrollIntoView({ block: 'start' })
  },
  { immediate: true, flush: 'post' },
)
</script>

<template>
  <section ref="root" class="settings-workbench" :class="{ 'fills-viewport': fillsViewport }" data-settings-workbench>
    <header v-if="activeDefinition" class="settings-workbench-toolbar" data-settings-toolbar>
      <h1>
        {{
          section === 'plugins-tools' ? $st('Plugins') : section === 'skills' ? $st('Skills') : activeDefinition.label
        }}
      </h1>
      <div v-if="scopes.length" class="settings-workbench-context">
        <OptionPicker
          v-if="scopes.length > 1"
          :model-value="activeScope"
          :options="scopes"
          :title="$st('Configuration scope')"
          :include-empty="false"
          size="sm"
          data-settings-scope
          @update:model-value="selectScope"
        />
        <span v-else class="text-xs text-muted-foreground">{{ activeScopeLabel }}</span>
        <SettingsSessionPicker
          v-if="activeScope === 'session'"
          :model-value="sessionTarget.sessionId.value"
          @select="sessionTarget.selectSession"
        />
        <span
          v-else-if="activeScope === 'workspace' && workspaceConfigPath !== undefined"
          class="settings-workbench-location flex items-center gap-1.5 text-xs text-muted-foreground"
          :title="workspaceConfigPath || undefined"
        >
          <RiFolderLine class="h-4 w-4 shrink-0" aria-hidden="true" />
          <span class="truncate" :class="workspaceConfigPath ? 'font-mono' : undefined">
            {{ workspaceConfigPath || $st('Server workspace') }}
          </span>
        </span>
        <SettingsProjectPicker
          v-else-if="activeScope === 'workspace'"
          class="min-w-0"
          :title="directory.currentDirectory"
          @select="selectProject"
        />
      </div>
      <nav v-if="localPages.length > 1" class="settings-workbench-tabs" :aria-label="$st('Related settings')">
        <button
          v-for="page in localPages"
          :key="page.id"
          type="button"
          :aria-current="page.id === activePage ? 'page' : undefined"
          @click="selectPage(page.id)"
        >
          {{ page.label }}
        </button>
      </nav>
    </header>
    <div class="settings-workbench-body">
      <slot :active-page="activePage" :active-definition="activeDefinition" />
    </div>
  </section>
</template>

<style scoped>
@reference '../../../style.css';

.settings-workbench {
  container: settings-workbench / inline-size;
  width: 100%;
  min-width: 0;
  max-width: 92rem;
  margin-inline: auto;
}
.settings-workbench-toolbar {
  @apply sticky top-0 z-20 border-b border-border/80 bg-background;
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  align-items: center;
  gap: 0.375rem 0.75rem;
  padding-block: 0.25rem 0.5rem;
}
.settings-workbench-toolbar h1 {
  font-size: clamp(1.125rem, 2.1cqi, 1.5rem);
  font-weight: 650;
  line-height: 1.25;
  overflow-wrap: anywhere;
}
.settings-workbench-context {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.375rem;
  min-width: 0;
}
.settings-workbench-context > * {
  max-width: 100%;
  min-width: 0;
}
.settings-workbench-context .settings-workbench-location {
  max-width: min(26rem, 100%);
}
.settings-workbench-tabs {
  display: flex;
  flex-wrap: wrap;
  gap: 0.25rem;
  min-width: 0;
}
.settings-workbench-tabs button {
  @apply rounded-md px-2.5 py-1.5 text-[13px] font-medium text-muted-foreground hover:bg-muted/50 hover:text-foreground
    focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring;
  min-height: 2rem;
  overflow-wrap: anywhere;
}
.settings-workbench-tabs button[aria-current='page'] {
  @apply bg-muted text-foreground;
}
.settings-workbench-body {
  min-width: 0;
  min-height: 0;
  padding-block-start: 0.75rem;
}
.settings-workbench-body :deep([id^='settings-']) {
  scroll-margin-top: 7.5rem;
}
@container settings-workbench (min-width: 46rem) {
  .settings-workbench-toolbar {
    grid-template-columns: minmax(0, 1fr) auto auto;
  }
  .settings-workbench-tabs {
    justify-content: end;
  }
}
@container settings-pane (min-width: 44rem) {
  .settings-workbench.fills-viewport {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    max-width: none;
  }
  .fills-viewport .settings-workbench-body {
    flex: 1;
  }
}
</style>
