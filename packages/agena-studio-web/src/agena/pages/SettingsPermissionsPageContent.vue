<script setup lang="ts">
import type { useSettingsPageState } from './useSettingsPageState'

import PermissionsSettingsPanel from './PermissionsSettingsPanel.vue'
import PermissionRulesManagerPanel from './PermissionRulesManagerPanel.vue'

const props = defineProps<{
  load: () => Promise<void>
  loading: boolean
  permissions: ReturnType<typeof useSettingsPageState>['panels']['permissions']
}>()

function clearActionStatus() {
  props.permissions.actionError.value = ''
  props.permissions.actionMessage.value = ''
}

function setActionError(message: string) {
  props.permissions.actionError.value = message
  props.permissions.actionMessage.value = ''
}

function setActionMessage(message: string) {
  props.permissions.actionError.value = ''
  props.permissions.actionMessage.value = message
}
</script>

<template>
  <PermissionRulesManagerPanel :permissions="props.permissions" />
  <PermissionsSettingsPanel
    :clear-action-status="clearActionStatus"
    :load="props.permissions.load"
    :loading="props.loading"
    :permission-config="props.permissions.permissionConfig"
    :set-action-error="setActionError"
    :set-action-message="setActionMessage"
  />
</template>
