<script setup lang="ts">
import { computed, ref, useId } from 'vue'
import { RiAddLine, RiDeleteBinLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { changeMcpValue, mcpPointerKey, mcpRecord, mcpValueAt, type McpDraft } from '@/lib/mcpConnections'

const props = defineProps<{ modelValue: McpDraft; pointer: string; title: string; kind: 'env' | 'headers'; disabled?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: McpDraft] }>()
const fieldId = useId()
const error = ref('')
const entries = computed(() => Object.entries(mcpRecord(mcpValueAt(props.modelValue.config, props.pointer))))
const pointerFor = (key: string) => props.pointer + '/' + mcpPointerKey(key)
const preserved = (key: string) => props.modelValue.preserveSecrets.includes(pointerFor(key))
function update(key: string, value: unknown, remove = false) {
  error.value = ''
  emit('update:modelValue', changeMcpValue(props.modelValue, pointerFor(key), value, remove))
}
function rename(key: string, input: HTMLInputElement) {
  const next = input.value.trim()
  if (next === key) return
  if (!next || entries.value.some(([name]) => name === next)) {
    input.value = key
    error.value = st('Use a unique, nonempty field name.')
    return
  }
  const existing = mcpValueAt(props.modelValue.config, pointerFor(key))
  const draft = changeMcpValue(props.modelValue, pointerFor(key), undefined, true)
  emit('update:modelValue', changeMcpValue(draft, pointerFor(next), existing))
  error.value = ''
}
function add() {
  const base = props.kind === 'env' ? 'NEW_VARIABLE' : 'Header-Name'
  let name = base
  for (let index = 2; entries.value.some(([key]) => key === name); index++) name = base + '_' + index
  update(name, '')
}
</script>

<template>
  <fieldset class="min-w-0 space-y-2" :disabled="disabled">
    <legend class="sr-only">{{ title }}</legend>
    <div class="flex flex-wrap items-center justify-between gap-2">
      <span class="text-xs font-medium">{{ title }}</span>
      <Button variant="ghost" size="sm" type="button" :disabled="disabled" @click="add">
        <RiAddLine class="mr-1 h-3.5 w-3.5" />{{ st('Add field') }}
      </Button>
    </div>
    <p v-if="!entries.length" class="rounded-lg border border-dashed border-border/70 p-3 text-xs text-muted-foreground">{{ st('No fields configured.') }}</p>
    <div v-for="([key, value], index) in entries" :key="key" class="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-end gap-2 rounded-lg border border-border/60 p-2.5 sm:grid-cols-[minmax(7rem,1fr)_minmax(8rem,1.2fr)_auto]">
      <label class="min-w-0 space-y-1" :for="fieldId + '-key-' + index">
        <span class="text-[11px] text-muted-foreground">{{ kind === 'env' ? st('Variable name') : st('Header name') }}</span>
        <input :id="fieldId + '-key-' + index" :value="key" :disabled="disabled || preserved(key)" class="w-full min-w-0 rounded-md border border-input bg-background px-2.5 py-2 font-mono text-xs disabled:opacity-70" autocomplete="off" spellcheck="false" @change="rename(key, $event.target as HTMLInputElement)" />
      </label>
      <label class="col-start-1 min-w-0 space-y-1 sm:col-start-auto" :for="fieldId + '-value-' + index">
        <span class="text-[11px] text-muted-foreground">{{ st('Field value') }} · {{ key }}</span>
        <input :id="fieldId + '-value-' + index" type="password" :value="preserved(key) ? '' : String(value ?? '')" :placeholder="preserved(key) ? st('Stored value') : ''" :disabled="disabled || preserved(key)" class="w-full min-w-0 rounded-md border border-input bg-background px-2.5 py-2 font-mono text-xs disabled:opacity-70" autocomplete="new-password" @input="update(key, ($event.target as HTMLInputElement).value)" />
      </label>
      <div class="col-start-2 row-span-2 row-start-1 flex items-center gap-1 self-end sm:col-start-auto sm:row-span-1 sm:row-start-auto">
        <Button v-if="preserved(key)" type="button" variant="outline" size="sm" :disabled="disabled" @click="update(key, '')">{{ st('Replace value') }}</Button>
        <IconButton type="button" size="sm" :disabled="disabled" :aria-label="st('Remove field') + ': ' + key" :tooltip="st('Remove field')" @click="update(key, undefined, true)"><RiDeleteBinLine class="h-4 w-4" /></IconButton>
      </div>
    </div>
    <p v-if="error" role="alert" class="text-xs text-destructive">{{ error }}</p>
    <p v-if="entries.some(([key]) => preserved(key))" class="text-xs leading-relaxed text-muted-foreground">{{ st('Stored values are preserved until you replace or remove them.') }}</p>
  </fieldset>
</template>
