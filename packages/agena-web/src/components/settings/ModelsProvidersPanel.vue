<script setup lang="ts">
import ProvidersPanel from '@/components/settings/ProvidersPanel.vue'
import ProviderStudioPanel from '@/components/settings/ProviderStudioPanel.vue'
</script>

<template>
  <div class="space-y-6">
    <div>
      <div class="text-lg font-medium">Models &amp; Providers</div>
      <div class="mt-1 max-w-3xl text-sm text-muted-foreground">
        Configure provider authentication and adapters, select the runtime and automatic-approval models, and inspect
        the resolved model catalog.
      </div>
    </div>
    <ProviderStudioPanel />
    <ProvidersPanel />
  </div>
</template>
