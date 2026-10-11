<script setup lang="ts">
import { computed } from 'vue'
import ApprovalModelPanel from '@/components/settings/ApprovalModelPanel.vue'
import { settingsText as st } from '@/i18n/settingsText'

type ApprovalScope = 'effective' | 'global' | 'workspace'

// The layer choice lives in the left navigation, so this page reports the layer
// it was opened with and hands it straight to the editor.
const props = defineProps<{ scope: string }>()

function normalizeApprovalScope(value: unknown): ApprovalScope {
  return value === 'global' || value === 'workspace' ? value : 'effective'
}

const scope = computed<ApprovalScope>(() => normalizeApprovalScope(props.scope))
const scopeOptions = computed(() => [
  { value: 'effective', label: st('Effective default'), description: st('Read-only merged default.') },
  { value: 'global', label: st('Global default'), description: st('Baseline for every workspace.') },
  {
    value: 'workspace',
    label: st('Workspace default'),
    description: st('Overrides the global default for this workspace.'),
  },
])
const activeScopeOption = computed(
  () => scopeOptions.value.find((option) => option.value === scope.value) || scopeOptions.value[0],
)
</script>

<template>
  <div class="grid gap-4">
    <div class="grid max-w-xl gap-1">
      <span class="text-xs font-medium">{{ $st('Model default source') }}</span>
      <span class="text-sm font-medium">{{ activeScopeOption.label }}</span>
      <span class="text-xs text-muted-foreground">{{ activeScopeOption.description }}</span>
    </div>
    <ApprovalModelPanel :scope="scope" />
  </div>
</template>
