<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import InlineReply from './InlineReply.vue'
import type { InlineUiReferences } from './references'
import { inlineScope, watchInlineReference, type InlineSnapshot } from './live'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { useNearViewport } from '@/composables/useNearViewport'
const props = defineProps<{ reference: string; sessionId: string; documents?: InlineUiReferences | null }>()
const { t } = useI18n(),
  pane = useWorkspacePaneContext(),
  root = ref<HTMLElement | null>(null),
  near = useNearViewport(root)
const snapshot = shallowRef<InlineSnapshot | null>(null),
  status = ref<'connected' | 'reconnecting' | 'unavailable'>('reconnecting')
const doc = computed(() =>
  status.value === 'unavailable' ? null : snapshot.value?.document || props.documents?.[props.reference],
)
const scope = computed(() => inlineScope(props.sessionId, props.reference))
let stop: (() => void) | undefined,
  generation = 0,
  mounted = false
function connect() {
  stop?.()
  stop = undefined
  const current = ++generation
  if (!mounted || document.hidden || !near.value || (pane && !pane.isVisible.value)) return
  if (!props.sessionId) {
    status.value = 'unavailable'
    return
  }
  stop = watchInlineReference(props.sessionId, props.reference, {
    snapshot: (value) => {
      if (current === generation) snapshot.value = value
    },
    status: (value) => {
      if (current === generation) {
        status.value = value
        if (value === 'unavailable') snapshot.value = null
      }
    },
  })
}
watch(
  () => [props.sessionId, props.reference],
  () => {
    snapshot.value = null
    status.value = 'reconnecting'
    connect()
  },
)
watch(() => [near.value, pane?.isVisible.value], connect)
onMounted(() => {
  mounted = true
  document.addEventListener('visibilitychange', connect)
  connect()
})
onBeforeUnmount(() => {
  mounted = false
  generation++
  stop?.()
  document.removeEventListener('visibilitychange', connect)
})
</script>
<template>
  <div
    ref="root"
    class="inline-embed min-w-0"
    data-inline-embed
    :data-inline-reference="reference"
    :data-inline-revision="snapshot?.revision"
  >
    <div v-if="snapshot" class="inline-live-header" aria-live="polite">
      <span class="inline-live-dot" :data-phase="snapshot.phase" aria-hidden="true" /><span>{{
        t(`inlineUi.${snapshot.phase}`)
      }}</span
      ><span class="inline-live-revision">{{ t('inlineUi.revision') }} {{ snapshot.revision }}</span
      ><span v-if="status === 'reconnecting'">{{ t('inlineUi.reconnecting') }}</span>
    </div>
    <InlineReply v-if="doc" :document="doc" :scope="scope" />
    <p v-else-if="status !== 'unavailable'" class="text-sm text-muted-foreground" role="status">
      {{ t('inlineUi.loading') }}
    </p>
    <div v-else class="border-s-2 border-border ps-3 text-sm text-muted-foreground" role="status">
      <p>{{ t('inlineUi.referenceUnavailable') }}</p>
      <button
        v-if="sessionId"
        type="button"
        class="mt-1 text-xs underline underline-offset-4 focus-visible:outline-2"
        @click="connect"
      >
        {{ t('inlineUi.retry') }}
      </button>
    </div>
  </div>
</template>
<style scoped>
.inline-embed {
  margin-block: calc(0.5rem * var(--padding-scale, 1));
}
.inline-live-header {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  margin: 0 0 0.375rem;
  font-size: var(--text-micro, 0.8125rem);
  color: var(--color-muted-foreground);
  min-height: 1rem;
}
.inline-live-dot {
  width: 0.375rem;
  height: 0.375rem;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--color-primary);
}
.inline-live-dot[data-phase='complete'] {
  background: var(--color-chart-2, var(--color-primary));
}
.inline-live-dot[data-phase='error'] {
  background: var(--color-destructive);
}
.inline-live-revision {
  margin-inline-start: auto;
  font-variant-numeric: tabular-nums;
}
</style>
