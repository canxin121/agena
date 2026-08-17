<script setup lang="ts">
import PluginsPanel from '@/components/settings/PluginsPanel.vue'
import HarnessSettingsPanel from '@/components/settings/HarnessSettingsPanel.vue'
</script>

<template>
  <div class="space-y-6">
    <div>
      <div class="text-lg font-medium">Plugins &amp; Tools</div>
      <div class="mt-1 max-w-3xl text-sm text-muted-foreground">{{ 'Plugin configuration, tools, commands, harnesses, and diagnostics.' }}</div>
    </div>
    <PluginsPanel />
    <HarnessSettingsPanel />
  </div>
</template>
