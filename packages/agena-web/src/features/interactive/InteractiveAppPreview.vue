<script setup lang="ts">
import { ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { apiJson } from '@/lib/api'
import { appApiPath, type AppSnapshot } from './api'
import InteractiveAppCard from './InteractiveAppCard.vue'
const props = defineProps<{ owner: string; appId: string }>()
const emit = defineEmits<{ title: [value: string] }>()
const { t } = useI18n()
const snapshot = shallowRef<AppSnapshot | null>(null)
const error = ref('')
watch(
  () => [props.owner, props.appId],
  async (_, __, onCleanup) => {
    const controller = new AbortController()
    onCleanup(() => controller.abort())
    snapshot.value = null
    error.value = ''
    try {
      const result = await apiJson<AppSnapshot>(appApiPath(props.owner, props.appId), { signal: controller.signal })
      if (!controller.signal.aborted) {
        snapshot.value = result
        emit('title', result.manifest.title)
      }
    } catch (err) {
      if (!controller.signal.aborted) error.value = err instanceof Error ? err.message : String(err)
    }
  },
  { immediate: true },
)
</script>
<template>
  <div class="h-full overflow-auto">
    <InteractiveAppCard
      v-if="snapshot"
      :manifest="snapshot.manifest"
      :owner="owner"
      :directory="snapshot.directory"
      standalone
    />
    <p v-else class="p-6 text-sm text-muted-foreground" role="status">{{ error || t('interactive.loading') }}</p>
  </div>
</template>
