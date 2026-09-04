<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import Button from '@/components/ui/Button.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import PathPicker from '@/components/ui/PathPicker.vue'
import { useUiStore } from '@/stores/ui'

const { t } = useI18n()
const ui = useUiStore()

const props = defineProps<{
  open: boolean
  path: string
  title: string
  description?: string
  placeholder?: string
  confirmLabel: string
  confirmDisabled?: boolean
  basePath?: string
  allowCreateDirectory?: boolean
}>()

const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'update:path', value: string): void
  (e: 'confirm'): void
}>()

const pathModel = computed({
  get: () => props.path,
  set: (value: string) => emit('update:path', value),
})

const confirmButtonDisabled = computed(() => {
  if (props.confirmDisabled !== undefined) return props.confirmDisabled
  return !pathModel.value.trim()
})

const browserLayoutClass = computed(() =>
  ui.isMobilePointer
    ? 'flex h-full min-h-0 flex-col'
    : 'flex h-[min(56dvh,34rem)] min-h-[14rem] flex-col',
)
</script>

<template>
  <FormDialog
    :open="open"
    :title="title"
    :description="description"
    mobile-fill-viewport
    @update:open="(value) => emit('update:open', value)"
  >
    <div class="flex h-full min-h-0 flex-col gap-3">
      <div class="min-h-0 flex-1">
        <PathPicker
          v-model="pathModel"
          :placeholder="placeholder"
          view="browser"
          mode="directory"
          :resolve-to-absolute="true"
          :base-path="basePath || ''"
          :show-options="true"
          :show-gitignored="true"
          :allow-create-directory="allowCreateDirectory ?? true"
          input-class="h-9 font-mono"
          :browser-class="browserLayoutClass"
        />
      </div>
      <div class="flex flex-none items-center justify-end gap-2">
        <Button variant="ghost" @click="emit('update:open', false)">
          {{ t('common.cancel') }}
        </Button>
        <Button :disabled="confirmButtonDisabled" @click="emit('confirm')">
          {{ confirmLabel }}
        </Button>
      </div>
    </div>
  </FormDialog>
</template>
