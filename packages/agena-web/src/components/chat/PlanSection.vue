<script setup lang="ts">
// Plan approval decisions belong to operation-owned interaction parts. This
// chip only inspects the durable plan and controls its autorun setting.
import { computed, defineAsyncComponent } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCheckboxMultipleLine, RiLoader4Line, RiRefreshLine } from '@remixicon/vue'
import IconButton from '@/components/ui/IconButton.vue'
import SessionChip from './SessionChip.vue'
import type { usePlanViewer } from '@/pages/chat/usePlanViewer'

const MarkdownRenderer = defineAsyncComponent(() => import('@/components/markdown/MarkdownRenderer.vue'))
const props = defineProps<{ state: ReturnType<typeof usePlanViewer>; label: string; open: boolean }>()
defineEmits<{ (event: 'update:open', value: boolean): void }>()
const { t } = useI18n()
const busy = computed(() => props.state.loading.value || props.state.toggling.value)
const hint = computed(() => {
  const plan = props.state.snapshot.value
  const step = plan ? plan.currentStep || plan.title : ''
  return [t('chat.planViewer.title'), props.label, step].filter(Boolean).join(' · ')
})
</script>

<template>
  <SessionChip
    data-plan-state-viewer="true"
    :title="t('chat.planViewer.title')"
    :label="label"
    :hint="hint"
    :open="open"
    :busy="state.loading.value && !state.snapshot.value"
    @update:open="$emit('update:open', $event)"
  >
    <template #icon>
      <RiCheckboxMultipleLine class="h-3 w-3 shrink-0" />
    </template>
    <template #actions>
      <IconButton
        class="part-action part-action-icon"
        :tooltip="t('chat.planViewer.refresh')"
        :aria-label="t('chat.planViewer.refresh')"
        :aria-disabled="busy"
        @click="!busy && state.refresh()"
      >
        <RiRefreshLine class="h-3.5 w-3.5" :class="state.loading.value ? 'animate-spin' : ''" />
      </IconButton>
    </template>
    <div class="min-w-0" :aria-busy="busy">
      <label v-if="state.autorun.value !== null" class="part-toggle mb-2" :aria-disabled="busy || undefined">
        <input
          type="checkbox"
          tabindex="0"
          :checked="state.autorun.value"
          :aria-disabled="busy"
          class="h-3.5 w-3.5 shrink-0 accent-primary"
          @click.prevent="!busy && state.toggleAutorun()"
        />
        <span class="interaction-choice-label">{{ t('chat.planViewer.autorun') }}</span>
        <RiLoader4Line v-if="state.toggling.value" class="ml-auto animate-spin text-primary" aria-hidden="true" />
      </label>
      <p v-if="state.error.value" role="alert" class="mb-2 text-sm text-destructive [overflow-wrap:anywhere]">
        {{ state.error.value }}
      </p>
      <MarkdownRenderer v-if="state.markdown.value" :content="state.markdown.value" mode="markdown" :stream="false" />
      <p v-else class="text-sm text-muted-foreground">
        {{ t(state.loading.value && !state.snapshot.value ? 'chat.planViewer.loading' : 'chat.planViewer.empty') }}
      </p>
    </div>
  </SessionChip>
</template>
