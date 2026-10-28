<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import InlineReply from './InlineReply.vue'
import { readInlineReference } from './api'
import type { InlineDocument } from './model'
import type { InlineUiReferences } from './references'

const props = defineProps<{ reference: string; sessionId: string; documents?: InlineUiReferences | null }>()
const { t } = useI18n()
const fetched = shallowRef<InlineDocument | null>(null)
const error = ref(false),
  loading = ref(false)
const document = computed(() => props.documents?.[props.reference] || fetched.value)
let controller: AbortController | undefined
let generation = 0
async function load() {
  controller?.abort()
  const current = ++generation
  fetched.value = null
  error.value = false
  loading.value = false
  if (props.documents?.[props.reference]) return
  if (!props.sessionId) {
    error.value = true
    return
  }
  controller = new AbortController()
  loading.value = true
  try {
    const result = await readInlineReference(props.sessionId, props.reference, controller.signal)
    if (current === generation) fetched.value = result
  } catch {
    if (current === generation && !controller.signal.aborted) error.value = true
  } finally {
    if (current === generation) loading.value = false
  }
}
watch(
  () => [props.sessionId, props.reference, props.documents?.[props.reference]],
  () => {
    void load()
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  generation++
  controller?.abort()
})
</script>
<template>
  <div class="my-4 min-w-0" data-inline-embed :data-inline-reference="reference">
    <InlineReply v-if="document" :document="document" />
    <p v-else-if="loading" class="text-sm text-muted-foreground" role="status">{{ t('inlineUi.loading') }}</p>
    <div v-else-if="error" class="border-s-2 border-border ps-3 text-sm text-muted-foreground" role="status">
      <p>{{ t('inlineUi.referenceUnavailable') }}</p>
      <button
        v-if="sessionId"
        type="button"
        class="mt-1 text-xs underline underline-offset-4 focus-visible:outline-2"
        @click="load"
      >
        {{ t('inlineUi.retry') }}
      </button>
    </div>
  </div>
</template>
