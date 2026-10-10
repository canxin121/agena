<script setup lang="ts">
import { computed, ref } from 'vue'
import ApprovalModelPanel from '@/components/settings/ApprovalModelPanel.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { settingsText as st } from '@/i18n/settingsText'

type ApprovalScope = 'effective' | 'global' | 'workspace'
const scope = ref<ApprovalScope>('effective')
const scopeOptions = computed(() => [
  { value: 'effective', label: st('Effective default'), description: st('Read-only merged default.') },
  { value: 'global', label: st('Global default'), description: st('Baseline for every workspace.') },
  {
    value: 'workspace',
    label: st('Workspace default'),
    description: st('Overrides the global default for this workspace.'),
  },
])
</script>

<template>
  <div class="grid gap-4">
    <label class="grid max-w-xl gap-1.5">
      <span class="text-xs font-medium">{{ $st('Model default source') }}</span>
      <OptionPicker
        :model-value="scope"
        :options="scopeOptions"
        :title="$st('Model default source')"
        :include-empty="false"
        @update:model-value="scope = $event as ApprovalScope"
      />
    </label>
    <ApprovalModelPanel :scope="scope" />
  </div>
</template>
