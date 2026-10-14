<script setup lang="ts">
import { computed, onBeforeUnmount, provide, ref, watch } from 'vue'
import { afterPaint } from '@/lib/afterPaint'
import { useWorkspacePaneContext, workspacePaneContextKey } from '@/app/workspace/workspacePaneContext'
import PartLoadingIndicator from './PartLoadingIndicator.vue'

const props = defineProps<{ active: boolean }>()
const mounted = ref(props.active)
const pane = useWorkspacePaneContext()
if (pane)
  provide(workspacePaneContextKey, {
    ...pane,
    isVisible: computed(() => props.active && pane.isVisible.value),
  })
let cancel: (() => void) | undefined
watch([() => props.active, () => !pane || pane.isVisible.value], ([active, visible]) => {
  cancel?.()
  if (active && !visible) return
  // Paint the disclosure first. Closing hides the body immediately and
  // releases heavy editors/terminals afterwards; a rapid reopen reuses it.
  cancel = afterPaint(() => {
    mounted.value = active
  })
})
onBeforeUnmount(() => cancel?.())
</script>

<template>
  <div v-show="active" :aria-busy="active && !mounted">
    <slot v-if="mounted" />
    <PartLoadingIndicator v-else-if="active" class="min-h-8 py-1" />
  </div>
</template>
