<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import ModelSelectionSettings from './ModelSelectionSettings.vue'
import { confirmAction } from '@/lib/appConfirm'
import { settingsText as st } from '@/i18n/settingsText'

withDefaults(defineProps<{ scope?: string }>(), { scope: 'effective' })

type ModelEditor = {
  refresh: () => Promise<void>
  discard: () => void
  dirty: boolean
  saving: boolean
}

const defaultEditor = ref<ModelEditor | null>(null)
const approvalEditor = ref<ModelEditor | null>(null)
const auxiliaryEditor = ref<ModelEditor | null>(null)
const editors = computed(() => [defaultEditor.value, approvalEditor.value, auxiliaryEditor.value])
const dirty = computed(() => editors.value.some((editor) => editor?.dirty))
const saving = computed(() => editors.value.some((editor) => editor?.saving))

// One navigation decision covers all drafts. A cancelled navigation must not
// discard one card before the other card has been checked.
async function canLeave() {
  if (saving.value) return false
  if (!dirty.value) return true
  return confirmAction({
    title: st('Discard unsaved model changes?'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}

onBeforeRouteUpdate(async (to, from) => {
  if (to.path === from.path && to.query.scope === from.query.scope && to.query.view === from.query.view) return true
  if (!(await canLeave())) return false
  for (const editor of editors.value) editor?.discard()
  return true
})
onBeforeRouteLeave(canLeave)

function beforeUnload(event: BeforeUnloadEvent) {
  if (!dirty.value && !saving.value) return
  event.preventDefault()
  event.returnValue = ''
}
onMounted(() => window.addEventListener('beforeunload', beforeUnload))
onBeforeUnmount(() => window.removeEventListener('beforeunload', beforeUnload))

defineExpose({
  refresh: async () => {
    await Promise.all(editors.value.map((editor) => editor?.refresh()))
  },
})
</script>

<template>
  <div class="grid min-w-0 gap-5" data-model-settings>
    <ModelSelectionSettings ref="defaultEditor" purpose="default" :scope="scope" />
    <ModelSelectionSettings ref="approvalEditor" purpose="approval" :scope="scope" />
    <ModelSelectionSettings ref="auxiliaryEditor" purpose="auxiliary" :scope="scope" />
  </div>
</template>
