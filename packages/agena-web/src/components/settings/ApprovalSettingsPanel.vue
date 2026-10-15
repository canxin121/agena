<script setup lang="ts">
import { computed, ref } from 'vue'
import ApprovalModelPanel from '@/components/settings/ApprovalModelPanel.vue'

type ApprovalScope = 'effective' | 'global' | 'workspace'

// The workbench owns the route-backed scope selector.
const props = defineProps<{ scope: string }>()

function normalizeApprovalScope(value: unknown): ApprovalScope {
  return value === 'global' || value === 'workspace' ? value : 'effective'
}

const scope = computed<ApprovalScope>(() => normalizeApprovalScope(props.scope))
const editor = ref<{ refresh: () => Promise<void> } | null>(null)
defineExpose({
  refresh: async () => {
    await editor.value?.refresh()
  },
})
</script>

<template>
  <ApprovalModelPanel ref="editor" :scope="scope" />
</template>
