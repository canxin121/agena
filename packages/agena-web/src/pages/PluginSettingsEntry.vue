<script setup lang="ts">
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'
import Button from '@/components/ui/Button.vue'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { pluginSettingsLocation } from '@/features/pluginWeb/settingsEntry'
const router = useRouter()
const pane = useWorkspacePaneContext()
function openSettings() {
  const target = pluginSettingsLocation()
  return pane ? pane.navigate(target, true) : router.replace(target)
}
onMounted(() => void openSettings())
</script>
<template>
  <div class="flex h-full items-center justify-center p-6">
    <Button variant="outline" @click="openSettings">{{ $t('pluginWeb.manage') }}</Button>
  </div>
</template>
