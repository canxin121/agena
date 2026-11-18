<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { RiFolderLine } from '@remixicon/vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SettingsSessionPicker from '../SettingsSessionPicker.vue'
import { buildSettingsScopes } from '../settingsScopeCatalog'
import { useSettingsSessionTarget } from '../settingsSessionScope'
import { PERMISSION_ACCESS_VIEW } from './permissionNavigation'

const props = defineProps<{ scope: string; sessionId?: string; configPath?: string; disabled?: boolean }>()
const route = useRoute()
const router = useRouter()
const target = useSettingsSessionTarget()
const targetId = computed(() => props.sessionId || target.sessionId.value)
const sessionScope = computed(() => props.scope === 'session' || props.scope === 'effective')
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
</script>

<template>
  <header class="permission-access-toolbar" data-permission-toolbar>
    <div class="permission-toolbar-heading">
      <h1 class="whitespace-nowrap text-base font-semibold">{{ $st('Access permissions') }}</h1>
      <slot name="status" />
    </div>
    <div class="permission-toolbar-context">
      <div class="permission-scope-picker" data-permission-scope>
        <OptionPicker
          :model-value="scope"
          :options="scopes"
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
      <div
        v-else-if="configPath && (scope === 'workspace' || scope === 'global')"
        class="permission-target"
        data-permission-target
        :title="configPath"
      >
        <RiFolderLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <code class="truncate text-[13px]">{{ configPath }}</code>
      </div>
    </div>
    <div class="permission-toolbar-actions"><slot name="actions" /></div>
  </header>
</template>

<style scoped>
@reference '../../../style.css';
.permission-access-toolbar {
  @apply sticky top-0 z-30 border-b border-border bg-background/95 backdrop-blur;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem 0.75rem;
  padding: 0.5rem 0;
}
.permission-toolbar-heading {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}
.permission-toolbar-context {
  display: flex;
  flex: 1 1 16rem;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}
.permission-scope-picker {
  width: 12rem;
  max-width: 100%;
  flex-shrink: 0;
}
.permission-target {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  min-width: 0;
  max-width: 16rem;
  flex: 1;
}
.permission-toolbar-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 0.375rem;
  margin-inline-start: auto;
}
@container permissions (max-width: 36rem) {
  .permission-toolbar-heading {
    flex: 1 1 auto;
  }
  .permission-toolbar-context {
    order: 3;
    flex-basis: 100%;
  }
  .permission-scope-picker {
    width: 11.5rem;
  }
  .permission-target {
    max-width: none;
  }
}
</style>
