<script setup lang="ts">
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import { appConfirmRequest, resolveAppConfirm } from '@/lib/appConfirm'

function onOpenChange(open: boolean) {
  if (!open) resolveAppConfirm(false)
}
</script>

<template>
  <ConfirmPopover
    v-if="appConfirmRequest"
    :open="true"
    force-dialog
    :title="appConfirmRequest.title"
    :description="appConfirmRequest.description"
    :confirm-text="appConfirmRequest.confirmText"
    :cancel-text="appConfirmRequest.cancelText"
    :variant="appConfirmRequest.variant"
    @confirm="resolveAppConfirm(true)"
    @cancel="resolveAppConfirm(false)"
    @update:open="onOpenChange"
  />
</template>
