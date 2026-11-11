<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import WorkspacePaneView from './WorkspacePaneView.vue'
import { useWorkspacePaneHosts } from './workspacePaneHosts'
import { useUiStore } from '@/stores/ui'
import { afterPaint } from '@/lib/afterPaint'

const props = defineProps<{
  suspended?: boolean
  // Compact layout shows one conversation surface instead of a split board, so
  // the same panes are teleported into a single host. Keeping this component
  // mounted across the breakpoint is what lets a window resize change the
  // layout without remounting (and re-reading) every page.
  compact?: boolean
  compactHost?: HTMLElement | null
}>()
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
const visibleIds = computed(() => {
  if (!props.compact) return new Set(ui.workspaceVisibleWindowIds)
  const active = String(
    ui.activeWorkspaceWindowId || ui.workspaceVisibleWindowIds[0] || ui.workspaceWindows[0]?.id || '',
  ).trim()
  return new Set(active ? [active] : [])
})

watch(
  () => [ui.workspaceWindows.map((tab) => tab.id), [...visibleIds.value]] as const,
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
  if (props.compact) return props.compactHost || parking.value
  return hosts?.get(groupsByWindow.value.get(id) || '') || parking.value
}

onBeforeUnmount(() => {
  for (const cancel of retiring.values()) cancel()
  retiring.clear()
})
</script>

<template>
  <div ref="parking" hidden aria-hidden="true" />
  <!-- One stable owner for each window. Moving between split groups, or between
       the split board and a narrow window, moves its DOM instead of destroying
       the router, transcript and editors. -->
  <template v-if="parking">
    <Teleport v-for="windowId in mountedWindowIds" :key="windowId" :to="target(windowId)!">
      <!-- `suspended` hides the pane while a gutter is dragged, but it must not
           report the pane as invisible: visibility pauses data streams and
           resumes them with a fresh read, so a resize would reload every page.
           Only a genuinely hidden tab changes `visible`. -->
      <WorkspacePaneView
        v-show="visibleIds.has(windowId) && !suspended"
        :visible="visibleIds.has(windowId)"
        :window-id="windowId"
        class="absolute inset-0"
      />
    </Teleport>
  </template>
</template>
