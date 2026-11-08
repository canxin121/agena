<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useChatStore } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import AssistantPanel from '@/features/assistant/AssistantPanel.vue'
const route = useRoute(),
  router = useRouter(),
  chat = useChatStore(),
  toasts = useToastsStore(),
  { t } = useI18n()
const source = computed(() => {
  const id = Number(route.query.source || chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? id : null
})
async function openChat(id: number) {
  try {
    await chat.selectSession(String(id))
    await router.push({ path: '/chat', query: { sessionId: String(id) } })
  } catch (e) {
    toasts.push('error', e instanceof Error ? e.message : String(e))
  }
}
</script>
<template>
  <div class="flex justify-end border-b border-border p-2">
    <button
      class="text-sm underline"
      @click="router.push({ path: '/integrations', query: source ? { source: String(source) } : {} })"
      data-testid="assistant-integrations"
    >
      {{ t('integrations.title') }}
    </button>
  </div>
  <AssistantPanel v-if="source" :key="source" :source="source" @open-chat="openChat" />
  <div v-else class="p-8 text-sm text-muted-foreground">
    {{ t('assistant.noSession') }}
    <button class="underline" @click="router.push('/chat')">{{ t('assistant.openChat') }}</button>
  </div>
</template>
