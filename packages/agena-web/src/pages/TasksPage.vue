<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useChatStore } from '@/stores/chat'
import TaskBoard from '@/features/tasks/TaskBoard.vue'
import { useToastsStore } from '@/stores/toasts'

const route = useRoute(),
  router = useRouter(),
  chat = useChatStore(),
  toasts = useToastsStore()
const source = computed(() => {
  const id = Number(route.query.source || chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? id : null
})
async function openSession(id: number) {
  try {
    await chat.selectSession(String(id))
    await router.push({
      path: '/chat',
      query: {
        sessionId: String(id),
        ...(typeof route.query.task === 'string' ? { taskId: route.query.task } : {}),
        ...(source.value ? { taskSource: String(source.value) } : {}),
      },
    })
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  }
}
function selected(id: string) {
  void router.replace({
    path: '/tasks',
    query: { ...route.query, task: id, ...(source.value ? { source: String(source.value) } : {}) },
  })
}
</script>

<template>
  <TaskBoard
    :key="String(source)"
    :source-session-id="source"
    :selected-task-id="typeof route.query.task === 'string' ? route.query.task : undefined"
    :compose="route.query.compose === '1'"
    @open-session="openSession"
    @selected="selected"
    @leave="router.push('/chat')"
  />
</template>
