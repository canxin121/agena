<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import Button from '@/components/ui/Button.vue'
import type { JsonValue } from '@/types/json'
import {
  clonePluginJson,
  pluginJsonRecord,
  type JsonRecord,
  type PluginSettingsNode,
  type PluginSettingsOption,
  type PluginSettingsVariant,
} from '@/lib/pluginOperations'

defineOptions({ name: 'PluginContractEditor' })

const props = withDefaults(
  defineProps<{
    node: PluginSettingsNode
    modelValue: JsonValue
    disabled?: boolean
    depth?: number
  }>(),
  { disabled: false, depth: 0 },
)

const emit = defineEmits<{ 'update:modelValue': [value: JsonValue] }>()

const record = computed<JsonRecord>(() => pluginJsonRecord(props.modelValue) || {})
const arrayValue = computed<JsonValue[]>(() => (Array.isArray(props.modelValue) ? props.modelValue : []))
const listItemNode = computed(() => props.node.item || null)
const recordValueNode = computed(() => props.node.value || null)
const fields = computed(() => (Array.isArray(props.node.fields) ? props.node.fields : []))
const options = computed(() => (Array.isArray(props.node.options) ? props.node.options : []))
const variants = computed(() => (Array.isArray(props.node.variants) ? props.node.variants : []))
const selectedVariant = computed(() => findVariant(props.node, props.modelValue) || variants.value[0] || null)
const recordKey = ref('')
const jsonDraft = ref('')
const jsonError = ref('')

watch(
  () => props.modelValue,
  (value) => {
    if (props.node.kind !== 'json') return
    jsonDraft.value = JSON.stringify(value ?? null, null, 2)
    jsonError.value = ''
  },
  { immediate: true, deep: true },
)

function update(value: JsonValue) {
  emit('update:modelValue', value)
}

function requireNode(node: PluginSettingsNode | null | undefined): PluginSettingsNode {
  if (!node) throw new Error('The plugin settings contract is missing a recursive child node.')
  return node
}

function recordEntryValue(key: string): JsonValue {
  return record.value[key] ?? null
}

function objectFieldValue(field: PluginSettingsNode): JsonValue {
  return record.value[field.id] ?? seed(field)
}


function seed(node: PluginSettingsNode): JsonValue {
  if (node.default !== undefined) return clonePluginJson(node.default)
  switch (node.kind) {
    case 'boolean':
      return false
    case 'integer':
    case 'number':
      return 0
    case 'text':
    case 'secret_reference':
    case 'path':
    case 'url':
    case 'duration':
      return ''
    case 'choice':
      return node.options?.[0] ? clonePluginJson(node.options[0].value) : null
    case 'multi_choice':
    case 'list':
      return []
    case 'record':
      return {}
    case 'object': {
      const output: JsonRecord = {}
      for (const field of node.fields || []) {
        if (field.required || field.default !== undefined) output[field.id] = seed(field)
      }
      return output
    }
    case 'tagged_variant': {
      const variant = node.variants?.[0]
      if (!variant) return {}
      return seedVariant(node, variant)
    }
    case 'json':
      return null
  }
  return null
}

function seedVariant(node: PluginSettingsNode, variant: PluginSettingsVariant): JsonRecord {
  const output: JsonRecord = {}
  if (node.discriminator) output[node.discriminator] = clonePluginJson(variant.tag)
  for (const field of variant.fields || []) {
    if (field.required || field.default !== undefined) output[field.id] = seed(field)
  }
  return output
}

function findVariant(node: PluginSettingsNode, value: JsonValue): PluginSettingsVariant | null {
  const current = pluginJsonRecord(value)
  if (!current || !node.discriminator) return null
  return (
    (node.variants || []).find(
      (variant) => JSON.stringify(current[node.discriminator!]) === JSON.stringify(variant.tag),
    ) || null
  )
}

function setField(field: PluginSettingsNode, value: JsonValue) {
  update({ ...record.value, [field.id]: value })
}

function hasField(field: PluginSettingsNode) {
  return Object.prototype.hasOwnProperty.call(record.value, field.id)
}

function enableField(field: PluginSettingsNode) {
  setField(field, seed(field))
}

function removeField(field: PluginSettingsNode) {
  const next = { ...record.value }
  delete next[field.id]
  update(next)
}

function enumKey(value: JsonValue) {
  return JSON.stringify(value)
}

function selectOption(raw: string, candidates: PluginSettingsOption[]) {
  const option = candidates.find((candidate) => enumKey(candidate.value) === raw)
  if (option) update(clonePluginJson(option.value))
}

function toggleMulti(option: PluginSettingsOption, checked: boolean) {
  const current = [...arrayValue.value]
  const key = enumKey(option.value)
  const index = current.findIndex((value) => enumKey(value) === key)
  if (checked && index < 0) current.push(clonePluginJson(option.value))
  if (!checked && index >= 0) current.splice(index, 1)
  update(current)
}

function updateNumber(raw: string, integer: boolean) {
  if (!raw.trim()) return
  const value = integer ? Number.parseInt(raw, 10) : Number.parseFloat(raw)
  if (Number.isFinite(value)) update(value)
}

function addListItem() {
  const item = props.node.item
  if (!item) return
  update([...arrayValue.value, seed(item)])
}

function setListItem(index: number, value: JsonValue) {
  const next = [...arrayValue.value]
  next[index] = value
  update(next)
}

function removeListItem(index: number) {
  update(arrayValue.value.filter((_, itemIndex) => itemIndex !== index))
}

function moveListItem(index: number, delta: number) {
  const target = index + delta
  if (target < 0 || target >= arrayValue.value.length) return
  const next = [...arrayValue.value]
  const item = next[index]
  if (item === undefined) return
  next.splice(index, 1)
  next.splice(target, 0, item)
  update(next)
}

function addRecordEntry() {
  const key = recordKey.value.trim()
  const valueNode = props.node.value
  if (!key || !valueNode || Object.prototype.hasOwnProperty.call(record.value, key)) return
  update({ ...record.value, [key]: seed(valueNode) })
  recordKey.value = ''
}

function setRecordEntry(key: string, value: JsonValue) {
  update({ ...record.value, [key]: value })
}

function removeRecordEntry(key: string) {
  const next = { ...record.value }
  delete next[key]
  update(next)
}

function selectVariant(id: string) {
  const variant = variants.value.find((candidate) => candidate.id === id)
  if (variant) update(seedVariant(props.node, variant))
}

function commitJson() {
  try {
    update(JSON.parse(jsonDraft.value) as JsonValue)
    jsonError.value = ''
  } catch (error) {
    jsonError.value = error instanceof Error ? error.message : String(error)
  }
}

function textInputType() {
  if (props.node.kind === 'url') return 'url'
  return 'text'
}
</script>

<template>
  <fieldset class="min-w-0 space-y-2" :disabled="disabled">
    <legend class="flex flex-wrap items-center gap-2 text-sm font-medium">
      <span>{{ node.title || node.id }}</span>
      <span v-if="node.required" class="text-destructive">*</span>
      <span v-if="node.secret" class="rounded bg-muted px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
        secret reference
      </span>
      <span v-else-if="node.sensitive" class="rounded bg-muted px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
        sensitive
      </span>
    </legend>
    <p v-if="node.description" class="text-xs leading-relaxed text-muted-foreground">{{ node.description }}</p>

    <label v-if="node.kind === 'boolean'" class="inline-flex items-center gap-2 text-sm">
      <input
        type="checkbox"
        :checked="modelValue === true"
        @change="update(($event.target as HTMLInputElement).checked)"
      />
      {{ modelValue === true ? 'On' : 'Off' }}
    </label>

    <input
      v-else-if="['text', 'secret_reference', 'path', 'url', 'duration'].includes(node.kind)"
      class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
      :type="textInputType()"
      :value="typeof modelValue === 'string' ? modelValue : ''"
      :minlength="node.constraints?.min_length || undefined"
      :maxlength="node.constraints?.max_length || undefined"
      :pattern="node.constraints?.pattern || undefined"
      :placeholder="node.kind === 'secret_reference' ? 'Host-managed secret reference' : ''"
      @input="update(($event.target as HTMLInputElement).value)"
    />

    <input
      v-else-if="node.kind === 'integer' || node.kind === 'number'"
      class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
      type="number"
      :step="node.kind === 'integer' ? 1 : node.constraints?.multiple_of || 'any'"
      :min="node.constraints?.minimum ?? undefined"
      :max="node.constraints?.maximum ?? undefined"
      :value="typeof modelValue === 'number' ? modelValue : 0"
      @input="updateNumber(($event.target as HTMLInputElement).value, node.kind === 'integer')"
    />

    <select
      v-else-if="node.kind === 'choice'"
      class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
      :value="enumKey(modelValue)"
      @change="selectOption(($event.target as HTMLSelectElement).value, options)"
    >
      <option v-for="option in options" :key="option.id" :value="enumKey(option.value)">
        {{ option.title }}
      </option>
    </select>

    <div v-else-if="node.kind === 'multi_choice'" class="grid gap-2 sm:grid-cols-2">
      <label v-for="option in options" :key="option.id" class="flex items-start gap-2 rounded-md border border-border/60 p-2 text-sm">
        <input
          class="mt-1"
          type="checkbox"
          :checked="arrayValue.some((value) => enumKey(value) === enumKey(option.value))"
          @change="toggleMulti(option, ($event.target as HTMLInputElement).checked)"
        />
        <span>
          <span class="block font-medium">{{ option.title }}</span>
          <span v-if="option.description" class="block text-xs text-muted-foreground">{{ option.description }}</span>
        </span>
      </label>
    </div>

    <div v-else-if="node.kind === 'object'" class="space-y-4 rounded-md border border-border/70 p-3">
      <div v-if="fields.length === 0" class="text-xs text-muted-foreground">No fields.</div>
      <div v-for="field in fields" :key="field.id" class="space-y-2 border-b border-border/50 pb-4 last:border-b-0 last:pb-0">
        <PluginContractEditor
          v-if="field.required || hasField(field)"
          :node="field"
          :model-value="objectFieldValue(field)"
          :disabled="disabled"
          :depth="depth + 1"
          @update:model-value="setField(field, $event)"
        />
        <div v-if="!field.required" class="flex justify-end">
          <Button v-if="hasField(field)" size="sm" variant="ghost" type="button" @click="removeField(field)">
            Use default / unset
          </Button>
          <Button v-else size="sm" variant="outline" type="button" @click="enableField(field)">
            Set {{ field.title || field.id }}
          </Button>
        </div>
      </div>
    </div>

    <div v-else-if="node.kind === 'list' && listItemNode" class="space-y-3 rounded-md border border-border/70 p-3">
      <div v-if="arrayValue.length === 0" class="text-xs text-muted-foreground">No items.</div>
      <div v-for="(item, index) in arrayValue" :key="index" class="space-y-2 rounded-md border border-border/60 p-3">
        <div class="flex items-center justify-between gap-2">
          <span class="font-mono text-[10px] text-muted-foreground">Item {{ index + 1 }}</span>
          <div class="flex gap-1">
            <Button size="sm" variant="ghost" type="button" :disabled="index === 0" @click="moveListItem(index, -1)">Up</Button>
            <Button size="sm" variant="ghost" type="button" :disabled="index + 1 === arrayValue.length" @click="moveListItem(index, 1)">Down</Button>
            <Button size="sm" variant="ghost" type="button" @click="removeListItem(index)">Remove</Button>
          </div>
        </div>
        <PluginContractEditor
          :node="requireNode(listItemNode)"
          :model-value="item"
          :disabled="disabled"
          :depth="depth + 1"
          @update:model-value="setListItem(index, $event)"
        />
      </div>
      <Button size="sm" variant="outline" type="button" @click="addListItem">Add item</Button>
    </div>

    <div v-else-if="node.kind === 'record' && recordValueNode" class="space-y-3 rounded-md border border-border/70 p-3">
      <div v-if="Object.keys(record).length === 0" class="text-xs text-muted-foreground">No entries.</div>
      <div v-for="key in Object.keys(record).sort()" :key="key" class="space-y-2 rounded-md border border-border/60 p-3">
        <div class="flex items-center justify-between gap-2">
          <span class="font-mono text-xs">{{ key }}</span>
          <Button size="sm" variant="ghost" type="button" @click="removeRecordEntry(key)">Remove</Button>
        </div>
        <PluginContractEditor
          :node="requireNode(recordValueNode)"
          :model-value="recordEntryValue(key)"
          :disabled="disabled"
          :depth="depth + 1"
          @update:model-value="setRecordEntry(key, $event)"
        />
      </div>
      <div class="flex items-center gap-2">
        <input
          v-model="recordKey"
          class="h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm"
          placeholder="New entry name"
          @keydown.enter.prevent="addRecordEntry"
        />
        <Button size="sm" variant="outline" type="button" :disabled="!recordKey.trim()" @click="addRecordEntry">Add</Button>
      </div>
    </div>

    <div v-else-if="node.kind === 'tagged_variant'" class="space-y-4 rounded-md border border-border/70 p-3">
      <select
        class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
        :value="selectedVariant?.id || ''"
        @change="selectVariant(($event.target as HTMLSelectElement).value)"
      >
        <option v-for="variant in variants" :key="variant.id" :value="variant.id">{{ variant.title }}</option>
      </select>
      <p v-if="selectedVariant?.description" class="text-xs text-muted-foreground">{{ selectedVariant.description }}</p>
      <div v-if="selectedVariant" class="space-y-4">
        <div v-for="field in selectedVariant.fields || []" :key="field.id" class="space-y-2">
          <PluginContractEditor
            v-if="field.required || hasField(field)"
            :node="field"
            :model-value="objectFieldValue(field)"
            :disabled="disabled"
            :depth="depth + 1"
            @update:model-value="setField(field, $event)"
          />
          <div v-if="!field.required" class="flex justify-end">
            <Button v-if="hasField(field)" size="sm" variant="ghost" type="button" @click="removeField(field)">Unset</Button>
            <Button v-else size="sm" variant="outline" type="button" @click="enableField(field)">Set {{ field.title }}</Button>
          </div>
        </div>
      </div>
    </div>

    <div v-else-if="node.kind === 'json'" class="space-y-2">
      <textarea
        v-model="jsonDraft"
        class="min-h-36 w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-xs"
        @blur="commitJson"
        @keydown.meta.enter.prevent="commitJson"
        @keydown.ctrl.enter.prevent="commitJson"
      />
      <div class="flex items-center justify-between gap-2">
        <span class="text-[10px] text-muted-foreground">Explicit bounded JSON field</span>
        <Button size="sm" variant="outline" type="button" @click="commitJson">Apply JSON</Button>
      </div>
      <p v-if="jsonError" class="text-xs text-destructive">{{ jsonError }}</p>
    </div>
  </fieldset>
</template>
