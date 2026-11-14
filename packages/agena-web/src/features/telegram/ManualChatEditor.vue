<script setup lang="ts">
import { ref, watch } from 'vue'
import Button from '@/components/ui/Button.vue'
import Dialog from '@/components/ui/Dialog.vue'
import { useTelegramCopy } from './copy'

defineProps<{ busy: boolean; error: string }>()
const emit = defineEmits<{
  close: []
  add: [draft: { chatId: string; title: string; kind: 'direct' | 'group' | 'broadcast' }]
}>()
const { tx, kindLabel } = useTelegramCopy()
const chatId = ref(''),
  title = ref(''),
  kind = ref<'direct' | 'group' | 'broadcast'>('group')
const field =
  'mt-1.5 h-10 w-full min-w-0 rounded-lg border border-border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring'
watch(chatId, (value) => {
  if (/^[1-9]\d*$/.test(value.trim())) kind.value = 'direct'
  else if (/^-[1-9]\d*$/.test(value.trim()) && kind.value === 'direct') kind.value = 'group'
})
</script>

<template>
  <Dialog
    open
    :title="tx('manualTitle')"
    :description="tx('manualHint')"
    max-width="max-w-lg"
    @close="!busy && emit('close')"
  >
    <form
      class="space-y-5"
      data-testid="telegram-manual-chat"
      @submit.prevent="emit('add', { chatId: chatId.trim(), title: title.trim(), kind })"
    >
      <fieldset :disabled="busy" class="space-y-4">
        <label class="block text-sm font-medium"
          >{{ tx('manualId') }}
          <input
            v-model="chatId"
            :class="field"
            required
            autofocus
            autocomplete="off"
            spellcheck="false"
            placeholder="-1001234567890 / @my_group"
            data-testid="telegram-manual-id"
          />
        </label>
        <label class="block text-sm font-medium"
          >{{ tx('manualKind') }}
          <select v-model="kind" :class="field" data-testid="telegram-manual-kind">
            <option v-for="value in ['direct', 'group', 'broadcast'] as const" :key="value" :value="value">
              {{ kindLabel(value) }}
            </option>
          </select>
        </label>
        <label class="block text-sm font-medium"
          >{{ tx('manualName') }}
          <input v-model="title" :class="field" maxlength="120" data-testid="telegram-manual-name" />
        </label>
        <p class="text-xs leading-relaxed text-muted-foreground">{{ tx('manualNumericHint') }}</p>
      </fieldset>
      <p v-if="error" role="alert" class="rounded-lg bg-destructive/10 p-3 text-sm text-destructive break-words">
        {{ error }}
      </p>
      <div class="flex justify-end gap-2 border-t border-border pt-4">
        <Button variant="ghost" :disabled="busy" @click="emit('close')">{{ tx('cancel') }}</Button>
        <Button type="submit" :disabled="busy || !chatId.trim()" data-testid="telegram-manual-save">{{
          busy ? tx('adding') : tx('add')
        }}</Button>
      </div>
    </form>
  </Dialog>
</template>
