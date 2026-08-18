<script setup lang="ts">
import PermissionStudioPanel from '@/components/settings/PermissionStudioPanel.vue'
import PermissionsPanel from '@/components/settings/PermissionsPanel.vue'
</script>

<template>
  <div class="space-y-6">
    <div>
      <div class="text-lg font-medium">Permissions</div>
      <div class="mt-1 max-w-3xl text-sm text-muted-foreground">
        Edit global, workspace, current-session, and effective permissions using the same Filesystem, Network, and Tool
        Access hierarchy as the TUI.
      </div>
    </div>
    <PermissionStudioPanel />
    <PermissionsPanel />
  </div>
</template>
