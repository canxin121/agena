<script setup lang="ts">
import { computed } from 'vue'
import ApprovalModelPanel from '@/components/settings/ApprovalModelPanel.vue'

type ApprovalScope = 'effective' | 'global' | 'workspace'

// The layer is chosen in the left navigation and named once in the section
// header, so this page only forwards the scope to the editor.
const props = defineProps<{ scope: string }>()

function normalizeApprovalScope(value: unknown): ApprovalScope {
  return value === 'global' || value === 'workspace' ? value : 'effective'
}

const scope = computed<ApprovalScope>(() => normalizeApprovalScope(props.scope))
</script>

<template>
  <ApprovalModelPanel :scope="scope" />
</template>
