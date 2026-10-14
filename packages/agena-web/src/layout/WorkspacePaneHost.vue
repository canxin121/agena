<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import WorkspacePaneView from './WorkspacePaneView.vue'
import { useWorkspacePaneHosts } from './workspacePaneHosts'
import { useUiStore } from '@/stores/ui'
import { afterPaint } from '@/lib/afterPaint'

const props = defineProps<{ suspended?: boolean }>()
const ui = useUiStore()
const hosts = useWorkspacePaneHosts()
const parking = ref<HTMLElement | null>(null)
const mountedWindowIds = ref<string[]>([])
const retiring = new Map<string, () => void>()
const groupsByWindow = computed(() => {
  const groups = new Map<string, string>()
  for (const group of ui.workspaceGroups) for (const id of group.tabIds) groups.set(id, group.id)
  return groups
})
const visibleIds = computed(() => new Set(ui.workspaceVisibleWindowIds))

watch(
  () => [ui.workspaceWindows.map((tab) => tab.id), ui.workspaceVisibleWindowIds] as const,
  ([openIds, visible]) => {
    const open = new Set(openIds)
    for (const id of mountedWindowIds.value) {
      if (open.has(id)) {
        retiring.get(id)?.()
        retiring.delete(id)
      } else if (!retiring.has(id)) {
        // Hide the closed tab immediately; release its component tree after
        // the new active tab/tab strip has had a chance to paint.
        retiring.set(
          id,
          afterPaint(() => {
            retiring.delete(id)
            mountedWindowIds.value = mountedWindowIds.value.filter((item) => item !== id)
          }),
        )
      }
    }
    const mounted = new Set(mountedWindowIds.value)
    for (const id of visible) if (open.has(id)) mounted.add(id)
    if (mounted.size !== mountedWindowIds.value.length) mountedWindowIds.value = [...mounted]
  },
  { immediate: true },
)

function target(id: string) {
  return hosts?.get(groupsByWindow.value.get(id) || '') || parking.value
}

onBeforeUnmount(() => {
  for (const cancel of retiring.values()) cancel()
  retiring.clear()
})
</script>

<template>
  <div ref="parking" hidden aria-hidden="true" />
  <!-- One stable owner for each window. Moving between split groups moves
       its DOM instead of destroying the router, transcript and editors. -->
  <template v-if="parking">
    <Teleport v-for="windowId in mountedWindowIds" :key="windowId" :to="target(windowId)!">
      <WorkspacePaneView
        v-show="visibleIds.has(windowId) && !suspended"
        :visible="visibleIds.has(windowId) && !suspended"
        :window-id="windowId"
        class="absolute inset-0"
      />
    </Teleport>
  </template>
</template>
