<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'

import WorkspacePreviewDockPanel from '@/features/workspacePreview/components/WorkspacePreviewDockPanel.vue'
import WorkspacePreviewPageSidebar from '@/features/workspacePreview/components/WorkspacePreviewPageSidebar.vue'
import WorkspacePageSidebar from '@/layout/WorkspacePageSidebar.vue'
import { useUiStore } from '@/stores/ui'
import { useWorkspacePreviewStore } from '@/stores/workspacePreview'
const InteractiveAppPreview = defineAsyncComponent(() => import('@/features/interactive/InteractiveAppPreview.vue'))

const ui = useUiStore()
const preview = useWorkspacePreviewStore()
const route = useRoute()
const { t } = useI18n()
const appTitle = ref('')
const isInteractiveApp = computed(
  () => typeof route.query.appOwner === 'string' && typeof route.query.appId === 'string',
)

function basenameFromPath(rawPath: string): string {
  const normalized = String(rawPath || '')
    .trim()
    .replace(/\\/g, '/')
    .replace(/\/+/g, '/')
    .replace(/\/+$/g, '')
  if (!normalized) return ''
  const parts = normalized.split('/').filter(Boolean)
  return parts[parts.length - 1] || ''
}

function hostFromUrl(rawUrl: string): string {
  const candidate = String(rawUrl || '').trim()
  if (!candidate) return ''
  try {
    const parsed = new URL(candidate)
    return String(parsed.hostname || parsed.host || '').trim()
  } catch {
    return ''
  }
}

function syncPreviewWindowTitle() {
  const base = String(t('nav.preview'))
  if (isInteractiveApp.value) {
    ui.setWorkspaceWindowTitleFromRoute(route.query, appTitle.value || String(route.query.appId))
    return
  }
  const active = preview.activeSession
  const detail = active
    ? basenameFromPath(active.runDirectory) || basenameFromPath(active.directory) || hostFromUrl(active.targetUrl)
    : ''
  const title = detail ? `${base} · ${detail}` : base

  ui.setWorkspaceWindowTitleFromRoute(route.query, title)
}

watch(
  () => [route.query.appOwner, route.query.appId],
  () => {
    appTitle.value = ''
  },
)
watch(
  () => [preview.activeSessionId, preview.activeSession?.id, route.query, appTitle.value],
  () => {
    syncPreviewWindowTitle()
  },
  { immediate: true },
)
</script>

<template>
  <div class="flex h-full min-h-0 overflow-hidden bg-background text-foreground">
    <WorkspacePageSidebar panel="preview">
      <WorkspacePreviewPageSidebar />
    </WorkspacePageSidebar>

    <div class="min-w-0 flex-1 overflow-hidden" v-show="!ui.isCompactLayout || !ui.isSessionSwitcherOpen">
      <InteractiveAppPreview
        v-if="isInteractiveApp"
        :owner="String(route.query.appOwner)"
        :app-id="String(route.query.appId)"
        @title="appTitle = $event"
      />
      <WorkspacePreviewDockPanel v-else controls-variant="viewport" viewer-mode="responsive" />
    </div>
  </div>
</template>
