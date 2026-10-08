<script setup lang="ts">
// Plan approval decisions belong to operation-owned interaction parts. This
// section only inspects the durable plan and controls its autorun setting.
import { computed, defineAsyncComponent } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine } from '@remixicon/vue'
import IconButton from '@/components/ui/IconButton.vue'
import SessionSection from './SessionSection.vue'
import type { usePlanViewer } from '@/pages/chat/usePlanViewer'

const MarkdownRenderer = defineAsyncComponent(() => import('@/components/markdown/MarkdownRenderer.vue'))
const props = defineProps<{ state: ReturnType<typeof usePlanViewer>; expanded: boolean }>()
defineEmits<{ (event: 'update:expanded', value: boolean): void }>()
const { t } = useI18n()
const summary = computed(() => {
  const plan = props.state.snapshot.value
  return plan ? [plan.progress, plan.currentStep || plan.title].filter(Boolean).join(' · ') : ''
})
</script>

<template>
  <SessionSection
    data-plan-state-viewer="true"
    docked
    :title="t('chat.planViewer.title')"
    :expanded="expanded"
    :summary="summary"
    :busy="state.loading.value || state.toggling.value"
    @update:expanded="$emit('update:expanded', $event)"
  >
    <template #actions>
      <IconButton
        class="h-8 w-8"
        :tooltip="t('chat.planViewer.refresh')"
        :aria-label="t('chat.planViewer.refresh')"
        :disabled="state.loading.value || state.toggling.value"
        @click="state.refresh"
      >
        <RiRefreshLine class="h-3.5 w-3.5" />
      </IconButton>
    </template>
    <div
      class="max-h-[min(45dvh,28rem)] min-w-0 overflow-auto overscroll-contain"
      :aria-busy="state.loading.value || state.toggling.value"
    >
      <label
        v-if="state.autorun.value !== null"
        class="mb-1 inline-flex min-h-7 cursor-pointer items-center gap-2 text-xs"
      >
        <input
          type="checkbox"
          :checked="state.autorun.value"
          :disabled="state.loading.value || state.toggling.value"
          class="h-4 w-4"
          @change="state.toggleAutorun"
        />
        {{ t('chat.planViewer.autorun') }}
      </label>
      <p v-if="state.error.value" role="alert" class="mb-2 text-sm text-destructive [overflow-wrap:anywhere]">
        {{ state.error.value }}
      </p>
      <MarkdownRenderer v-if="state.markdown.value" :content="state.markdown.value" mode="markdown" :stream="false" />
      <p v-else class="text-sm text-muted-foreground">
        {{ t(state.loading.value ? 'chat.planViewer.loading' : 'chat.planViewer.empty') }}
      </p>
    </div>
  </SessionSection>
</template>
