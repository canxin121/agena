<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { RiFolderLine } from '@remixicon/vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { settingsText as st } from '@/i18n/settingsText'
import SettingsSessionPicker from '../SettingsSessionPicker.vue'
import PermissionWorkspacePicker from './PermissionWorkspacePicker.vue'
import { buildSettingsScopes } from '../settingsScopeCatalog'
import { useSettingsSessionTarget } from '../settingsSessionScope'
import { PERMISSION_ACCESS_VIEW } from './permissionNavigation'

const props = defineProps<{
  scope: string
  sessionId?: string
  workspaceTarget: string
  configPath?: string
  disabled?: boolean
}>()
const route = useRoute()
const router = useRouter()
const target = useSettingsSessionTarget()
const targetId = computed(() => props.sessionId || target.sessionId.value)
const sessionScope = computed(() => props.scope === 'session' || props.scope === 'effective')
const scopeLabels = computed<Record<string, string>>(() => ({
  effective: st('Effective'),
  global: st('Global'),
  workspace: st('Workspace'),
  session: st('Current session'),
}))
const scopes = computed(() =>
  buildSettingsScopes('permissions', PERMISSION_ACCESS_VIEW).map((scope) => ({
    value: scope.id,
    label: scope.label,
    description: scope.description,
  })),
)
function selectScope(scope: string) {
  if (props.disabled || scope === props.scope || !scopes.value.some((item) => item.value === scope)) return
  void router.replace({ path: route.path, query: { ...route.query, scope }, hash: route.hash })
}

function selectWorkspace(id: string) {
  if (props.disabled || id === props.workspaceTarget) return
  const { targetWorkspaceId: _previous, ...query } = route.query
  if (id) query.targetWorkspaceId = id
  void router.replace({ path: route.path, query, hash: route.hash })
}
</script>

<template>
  <header class="permission-access-toolbar" data-permission-toolbar>
    <div class="permission-toolbar-heading">
      <h1>{{ $st('Access permissions') }}</h1>
      <slot name="status" />
    </div>
    <div class="permission-toolbar-context">
      <div class="permission-scope-picker" data-permission-scope>
        <OptionPicker
          :model-value="scope"
          :options="scopes"
          :display-label="scopeLabels[scope]"
          :include-empty="false"
          :title="$st('Configuration scope')"
          :disabled="disabled"
          size="sm"
          @update:model-value="selectScope"
        />
      </div>
      <div
        v-if="sessionScope"
        class="permission-target"
        data-permission-target
        :aria-label="$st('Session for this setting')"
      >
        <SettingsSessionPicker :model-value="targetId" :disabled="disabled" @select="target.selectSession" />
      </div>
      <div v-else-if="scope === 'workspace'" class="permission-target" data-permission-target :title="configPath">
        <PermissionWorkspacePicker
          :model-value="workspaceTarget"
          :config-path="configPath"
          :disabled="disabled"
          @select="selectWorkspace"
        />
      </div>
      <div
        v-else-if="configPath && scope === 'global'"
        class="permission-target permission-config-target"
        data-permission-target
        :title="configPath"
      >
        <RiFolderLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <code dir="ltr" class="truncate text-[13px]">{{ configPath }}</code>
      </div>
    </div>
    <div class="permission-toolbar-actions"><slot name="actions" /></div>
  </header>
</template>

<style scoped>
@reference '../../../style.css';
.permission-access-toolbar {
  @apply sticky top-0 z-30 border-b border-border/80 bg-background;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  grid-template-areas:
    'heading actions'
    'context context';
  align-items: center;
  gap: 0.5rem 0.875rem;
  padding: 0.25rem 0 0.625rem;
}
.permission-toolbar-heading {
  grid-area: heading;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.25rem 0.625rem;
  min-width: 0;
}
.permission-toolbar-heading h1 {
  font-size: clamp(1.125rem, 2.1cqi, 1.5rem);
  font-weight: 650;
  line-height: 1.25;
  letter-spacing: -0.02em;
}
.permission-toolbar-context {
  grid-area: context;
  display: grid;
  grid-template-columns: minmax(0, min(12rem, 49%)) minmax(0, 1fr);
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}
.permission-scope-picker {
  min-width: 0;
}
.permission-target {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  min-width: 0;
}
.permission-config-target {
  @apply rounded-md bg-muted/30 px-2 py-1.5 text-muted-foreground;
}
.permission-toolbar-actions {
  grid-area: actions;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 0.25rem;
  min-width: 0;
}
@container permissions (min-width: 62rem) {
  .permission-access-toolbar {
    grid-template-columns: minmax(0, max-content) minmax(0, 1fr) auto;
    grid-template-areas: 'heading context actions';
    column-gap: 1rem;
  }
}
@container permissions (max-width: 37rem) {
  .permission-access-toolbar:has([data-permission-savebar]) {
    grid-template-areas:
      'heading heading'
      'context context'
      'actions actions';
    row-gap: 0.375rem;
  }
}
</style>
