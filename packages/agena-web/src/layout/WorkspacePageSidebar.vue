<script setup lang="ts">
import { computed, type HTMLAttributes } from 'vue'
import { useRoute } from 'vue-router'
import { isEmbeddedWorkspacePaneContext } from '@/app/windowScope'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { useUiStore } from '@/stores/ui'
import { cn } from '@/lib/utils'
import DesktopSidebarResizeHandle from './DesktopSidebarResizeHandle.vue'
import { useWorkspaceSidebarHost } from './workspaceSidebarHost'

defineOptions({ inheritAttrs: false })
const props = withDefaults(
  defineProps<{
    panel: 'files' | 'terminal' | 'git' | 'preview' | 'settings'
    mode?: 'page' | 'embedded' | 'hosted-only' | 'local'
    visible?: boolean
    keepMounted?: boolean
    /** The standalone desktop view always has its own resizable sidebar. */
    localResizable?: boolean
    class?: HTMLAttributes['class']
  }>(),
  { mode: 'page', visible: true },
)

const route = useRoute()
const pane = useWorkspacePaneContext()
const ui = useUiStore()
const host = useWorkspaceSidebarHost()
const eligible = computed(
  () =>
    props.visible &&
    (!pane || pane.isFocused.value) &&
    (props.mode === 'embedded' || props.mode === 'local' || !isEmbeddedWorkspacePaneContext(route.query)),
)
const target = computed(() =>
  eligible.value && !ui.isCompactLayout && (props.mode === 'page' || props.mode === 'hosted-only')
    ? host?.value
    : undefined,
)
const localDesktop = computed(() => !target.value && !ui.isCompactLayout && props.mode !== 'embedded')
const shown = computed(() => {
  if (!eligible.value) return false
  if (props.mode === 'hosted-only') return Boolean(target.value)
  if (target.value || props.mode === 'embedded') return true
  return ui.isCompactLayout ? ui.isSessionSwitcherOpen : props.localResizable || ui.isSidebarOpen
})
</script>

<template>
  <Teleport :to="target ?? undefined" :disabled="!target">
    <aside
      v-if="shown || keepMounted"
      v-show="shown"
      v-bind="$attrs"
      :data-workspace-sidebar="panel"
      :class="
        cn(
          'relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-sidebar text-sidebar-foreground',
          localDesktop ? 'shrink-0 border-e border-border' : 'w-full',
          props.class,
        )
      "
      :style="localDesktop ? { width: `${ui.sidebarWidth}px` } : undefined"
    >
      <DesktopSidebarResizeHandle v-if="localDesktop && localResizable" />
      <slot />
    </aside>
  </Teleport>
</template>
