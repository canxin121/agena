<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'

import Button from '@/components/ui/Button.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import Input from '@/components/ui/Input.vue'
import ScrollArea from '@/components/ui/ScrollArea.vue'
import { handleOptionKeydown } from '@/lib/keyboardNavigation'

type BranchOption = { label: string; value: string }

const props = defineProps<{
  open: boolean
  title: string
  description: string
  actionLabel: string
  branch: string
  branches: BranchOption[]
  busy: boolean
}>()

const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'update:branch', value: string): void
  (e: 'submit'): void
}>()

const { t } = useI18n()
const branchChoices = ref<HTMLElement | null>(null)

function branchInput() {
  return branchChoices.value?.querySelector<HTMLInputElement>('input') || null
}

function onUpdateOpen(v: boolean) {
  emit('update:open', v)
}

function onUpdateBranch(v: string | number) {
  emit('update:branch', String(v))
}

function chooseBranch(value: string) {
  emit('update:branch', value)
}
</script>

<template>
  <FormDialog
    :open="open"
    :title="title"
    :description="description"
    maxWidth="max-w-md"
    initial-focus="[data-git-branch-choices]"
    @update:open="onUpdateOpen"
  >
    <div
      ref="branchChoices"
      data-git-branch-choices
      tabindex="-1"
      class="space-y-3 focus-visible:outline-2 focus-visible:outline-ring"
      @keydown="handleOptionKeydown($event, branchChoices, branchInput())"
    >
      <Input
        :model-value="branch"
        class="h-9 font-mono text-xs"
        :placeholder="t('git.ui.branchAction.branchNamePlaceholder')"
        :aria-label="t('git.ui.branchAction.branchNamePlaceholder')"
        @update:model-value="onUpdateBranch"
      />

      <ScrollArea v-if="branches.length" class="h-40 border rounded-md">
        <div class="p-1 space-y-0.5">
          <button
            v-for="b in branches"
            :key="b.value"
            type="button"
            data-keyboard-option
            class="w-full text-left px-3 py-2 rounded-sm text-xs font-mono hover:bg-muted/50 focus-visible:outline-2 focus-visible:outline-ring"
            @click="chooseBranch(b.value)"
          >
            {{ b.label }}
          </button>
        </div>
      </ScrollArea>

      <div class="flex justify-end gap-2">
        <Button variant="secondary" size="sm" @click="$emit('update:open', false)" :disabled="busy">{{
          t('common.cancel')
        }}</Button>
        <Button size="sm" :disabled="busy || !branch.trim()" @click="$emit('submit')">{{ actionLabel }}</Button>
      </div>
    </div>
  </FormDialog>
</template>
