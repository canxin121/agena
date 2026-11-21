<script setup lang="ts">
import { computed, defineAsyncComponent, inject, provide, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiExternalLinkLine, RiGitBranchLine } from '@remixicon/vue'
import type { Session } from '@/types/chat'
import { useChatStore } from '@/stores/chat'
import { openBrowserKey } from '@/features/browser/api'
import SessionChip from './SessionChip.vue'
import Button from '@/components/ui/Button.vue'

const AgentConversationPreview = defineAsyncComponent(() => import('./AgentConversationPreview.vue'))
const props = defineProps<{ agent: Session }>()
const { t } = useI18n()
const chat = useChatStore()
const open = ref(false)
const name = computed(() => String(props.agent.agent_name || props.agent.title || `#${props.agent.id}`))
const statusKeys: Record<string, string> = {
  created: 'chat.partStatus.pending',
  running: 'chat.partStatus.running',
  completed: 'chat.partStatus.completed',
  failed: 'chat.partStatus.failed',
  cancelled: 'chat.partStatus.cancelled',
  timed_out: 'chat.operations.status.timed_out',
  interrupted: 'chat.messageItem.interrupted',
}
const status = computed(() => statusKeys[String(props.agent.agent_status)])
const openBrowser = inject(openBrowserKey, undefined)
if (openBrowser)
  provide(openBrowserKey, (sid, page) => {
    open.value = false
    openBrowser(sid, page)
  })
function enter() {
  open.value = false
  void chat.selectSession(props.agent.id)
}
</script>

<template>
  <SessionChip
    class="composer-resource"
    :title="name"
    :hint="`${t('chat.operations.agent')} · ${name}`"
    :open="open"
    :data-agent-chip="agent.id"
    @update:open="open = $event"
  >
    <template #trigger>
      <RiGitBranchLine class="resource-icon" aria-hidden="true" />
      <span class="min-w-0 text-start"
        ><span class="resource-name">{{ name }}</span>
        <span class="resource-caption"
          >{{ t('chat.operations.agent') }}<template v-if="status"> · {{ t(status) }}</template></span
        >
      </span>
    </template>
    <template #actions>
      <Button size="sm" variant="ghost" @click="enter"
        ><RiExternalLinkLine aria-hidden="true" />{{ t('chat.operations.openAgentSession') }}</Button
      >
    </template>
    <AgentConversationPreview v-if="open" :session-id="agent.id" />
  </SessionChip>
</template>
