<script setup lang="ts">
import type { useSettingsPageState } from './useSettingsPageState'

import PermissionsSettingsPanel from './PermissionsSettingsPanel.vue'

const props = defineProps<{
  load: () => Promise<void>
  loading: boolean
  permissions: ReturnType<typeof useSettingsPageState>['panels']['permissions']
}>()
</script>

<template>
  <PermissionsSettingsPanel
    :loading="props.loading"
    :permission-search="props.permissions.search"
    :permission-status-filter="props.permissions.statusFilter"
    :permission-scope-filter="props.permissions.scopeFilter"
    :permission-mode-filter="props.permissions.modeFilter"
    :permission-subject-filter="props.permissions.subjectFilter"
    :permission-draft="props.permissions.draft"
    :editing-permission-rule-id="props.permissions.editingRuleId"
    :filtered-permission-rules="props.permissions.filteredRules"
    :load="props.load"
    :save-permission-rule="props.permissions.saveRule"
    :reset-permission-draft="props.permissions.resetDraft"
    :edit-permission-rule="props.permissions.editRule"
    :revoke-permission-rule-action="props.permissions.revokeRuleAction"
    :delete-permission-rule-action="props.permissions.deleteRuleAction"
    :permission-rule-label="props.permissions.ruleLabel"
    :permission-rule-preview="props.permissions.rulePreview"
    :permission-rule-facts="props.permissions.ruleFacts"
    @update:permission-search="props.permissions.search = $event"
    @update:permission-status-filter="props.permissions.statusFilter = $event"
    @update:permission-scope-filter="props.permissions.scopeFilter = $event"
    @update:permission-mode-filter="props.permissions.modeFilter = $event"
    @update:permission-subject-filter="props.permissions.subjectFilter = $event"
  />
</template>
