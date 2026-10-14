<script setup lang="ts">
import { computed, nextTick, ref, useId, watch } from 'vue'
import { RiArrowDownSLine, RiArrowUpSLine, RiCloseLine, RiResetLeftLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { settingsText as st } from '@/i18n/settingsText'
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
    optional?: boolean
    isSet?: boolean
  }>(),
  { disabled: false, depth: 0, optional: false, isSet: true },
)

const emit = defineEmits<{
  'update:modelValue': [value: JsonValue]
  set: []
  unset: []
}>()

const fieldId = useId()
const editorElement = ref<HTMLFieldSetElement | null>(null)
const isScalar = computed(() => !isStructured(props.node))
const hasHeaderControl = computed(() => isScalar.value || props.node.kind === 'tagged_variant')
const unsetSummary = computed(() => {
  const value = props.node.default
  if (value === undefined) return st('Not configured')
  if (props.node.secret || props.node.sensitive) return st('Default')
  const label = props.node.options?.find((option) => enumKey(option.value) === enumKey(value))?.title
  const preview = label ?? (typeof value === 'boolean' ? (value ? st('On') : st('Off')) : value)
  return typeof preview === 'string' || typeof preview === 'number' ? `${st('Default')}: ${preview}` : st('Default')
})
const unsetLabel = computed(
  () =>
    `${props.node.default === undefined ? st('Unset') : st('Reset to defaults')}: ${props.node.title || props.node.id}`,
)

const record = computed<JsonRecord>(() => pluginJsonRecord(props.modelValue) || {})
const arrayValue = computed<JsonValue[]>(() => (Array.isArray(props.modelValue) ? props.modelValue : []))
const listItemNode = computed(() => props.node.item || null)
const recordValueNode = computed(() => props.node.value || null)
const fields = computed(() => (Array.isArray(props.node.fields) ? props.node.fields : []))
const options = computed(() => (Array.isArray(props.node.options) ? props.node.options : []))
const variants = computed(() => (Array.isArray(props.node.variants) ? props.node.variants : []))
const selectedVariant = computed(() => findVariant(props.node, props.modelValue) || variants.value[0] || null)
const recordKey = ref('')
const canAddRecordEntry = computed(
  () => Boolean(recordKey.value.trim()) && !Object.prototype.hasOwnProperty.call(record.value, recordKey.value.trim()),
)
const jsonDraft = ref('')
const jsonError = ref('')

function isStructured(node: PluginSettingsNode) {
  return ['object', 'list', 'record', 'tagged_variant', 'multi_choice', 'json'].includes(node.kind)
}

async function requestSet() {
  emit('set')
  await nextTick()
  editorElement.value?.querySelector<HTMLElement>('input, select, textarea, [data-add-item]')?.focus()
}

async function requestUnset() {
  emit('unset')
  await nextTick()
  editorElement.value?.querySelector<HTMLElement>('[data-enable-field]')?.focus()
}

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
  if (!node) throw new Error(st('The plugin settings contract is missing a recursive child node.'))
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
  <fieldset
    ref="editorElement"
    class="contract-editor"
    :class="{
      'contract-root': depth === 0,
      'contract-branch': node.kind === 'object' || node.kind === 'tagged_variant',
      'contract-unset': !isSet,
    }"
    :disabled="disabled"
    :aria-labelledby="hasHeaderControl ? undefined : `${fieldId}-label`"
    :data-setting-path="node.path"
  >
    <div class="contract-layout">
      <div
        class="contract-header"
        :class="{
          'contract-row': hasHeaderControl || !isSet,
          'contract-toggle': node.kind === 'boolean' && isSet,
        }"
      >
        <div class="contract-heading min-w-0">
          <component
            :is="
              hasHeaderControl && isSet ? 'label' : isScalar ? 'div' : depth === 0 ? 'h3' : depth === 1 ? 'h4' : 'h5'
            "
            :id="`${fieldId}-label`"
            :for="hasHeaderControl && isSet ? fieldId : undefined"
            class="contract-title flex flex-wrap items-center gap-1.5 text-sm font-medium"
          >
            <span class="break-words">{{ node.title || node.id }}</span>
            <span v-if="node.required && depth > 0" class="text-destructive" aria-hidden="true">*</span>
            <span v-if="node.secret" class="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
              {{ $st('secret reference') }}
            </span>
            <span v-else-if="node.sensitive" class="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
              {{ $st('sensitive') }}
            </span>
          </component>
          <p
            v-if="node.description"
            :id="`${fieldId}-description`"
            class="mt-1 break-words text-xs leading-relaxed text-muted-foreground"
          >
            {{ node.description }}
          </p>
        </div>

        <div v-if="!isSet" class="contract-control">
          <span class="min-w-0 break-words text-xs text-muted-foreground">{{ unsetSummary }}</span>
          <Button
            size="sm"
            variant="outline"
            type="button"
            class="shrink-0"
            :aria-label="`${$st('Set')}: ${node.title || node.id}`"
            data-enable-field
            @click="requestSet"
            >{{ $st('Set') }}</Button
          >
        </div>

        <div v-else-if="hasHeaderControl" class="contract-control" :class="`contract-control-${node.kind}`">
          <label v-if="node.kind === 'boolean'" class="inline-flex min-h-9 shrink-0 cursor-pointer items-center gap-2">
            <input
              :id="fieldId"
              class="peer sr-only"
              type="checkbox"
              role="switch"
              :aria-label="node.title || node.id"
              :aria-describedby="node.description ? `${fieldId}-description` : undefined"
              :checked="modelValue === true"
              @change="update(($event.target as HTMLInputElement).checked)"
            />
            <span
              class="relative h-5 w-9 rounded-full border border-input bg-muted transition-colors peer-checked:border-primary peer-checked:bg-primary peer-focus-visible:ring-2 peer-focus-visible:ring-ring peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-background peer-disabled:opacity-50"
              aria-hidden="true"
            >
              <span
                class="absolute start-0.5 top-0.5 h-3.5 w-3.5 rounded-full bg-muted-foreground transition-transform"
                :class="modelValue === true ? 'translate-x-4 bg-primary-foreground rtl:-translate-x-4' : ''"
              />
            </span>
            <span class="text-xs text-muted-foreground">{{ modelValue === true ? $st('On') : $st('Off') }}</span>
          </label>

          <input
            v-else-if="['text', 'secret_reference', 'path', 'url', 'duration'].includes(node.kind)"
            :id="fieldId"
            class="contract-input"
            :type="textInputType()"
            :aria-label="node.title || node.id"
            :aria-describedby="node.description ? `${fieldId}-description` : undefined"
            :value="typeof modelValue === 'string' ? modelValue : ''"
            :minlength="node.constraints?.min_length ?? undefined"
            :maxlength="node.constraints?.max_length ?? undefined"
            :pattern="node.constraints?.pattern || undefined"
            :placeholder="node.kind === 'secret_reference' ? $st('Host-managed secret reference') : ''"
            @input="update(($event.target as HTMLInputElement).value)"
          />

          <input
            v-else-if="node.kind === 'integer' || node.kind === 'number'"
            :id="fieldId"
            class="contract-input tabular-nums"
            type="number"
            :aria-label="node.title || node.id"
            :aria-describedby="node.description ? `${fieldId}-description` : undefined"
            :step="node.kind === 'integer' ? 1 : node.constraints?.multiple_of || 'any'"
            :min="node.constraints?.minimum ?? undefined"
            :max="node.constraints?.maximum ?? undefined"
            :value="typeof modelValue === 'number' ? modelValue : 0"
            @input="updateNumber(($event.target as HTMLInputElement).value, node.kind === 'integer')"
          />

          <select
            v-else-if="node.kind === 'choice'"
            :id="fieldId"
            class="contract-input"
            :aria-label="node.title || node.id"
            :aria-describedby="node.description ? `${fieldId}-description` : undefined"
            :value="enumKey(modelValue)"
            @change="selectOption(($event.target as HTMLSelectElement).value, options)"
          >
            <option v-for="option in options" :key="option.id" :value="enumKey(option.value)">
              {{ option.title }}
            </option>
          </select>

          <select
            v-else-if="node.kind === 'tagged_variant'"
            :id="fieldId"
            class="contract-input"
            :aria-label="node.title || node.id"
            :aria-describedby="node.description ? `${fieldId}-description` : undefined"
            :value="selectedVariant?.id || ''"
            @change="selectVariant(($event.target as HTMLSelectElement).value)"
          >
            <option v-for="variant in variants" :key="variant.id" :value="variant.id">{{ variant.title }}</option>
          </select>

          <IconButton
            v-if="optional"
            size="sm"
            class="text-muted-foreground"
            :tooltip="unsetLabel"
            :aria-label="unsetLabel"
            @click="requestUnset"
            ><RiResetLeftLine class="h-3.5 w-3.5"
          /></IconButton>
          <slot name="actions" />
        </div>

        <div v-else-if="optional || $slots.actions" class="contract-actions">
          <slot name="actions" />
          <IconButton
            v-if="optional"
            size="sm"
            class="text-muted-foreground"
            :tooltip="unsetLabel"
            :aria-label="unsetLabel"
            @click="requestUnset"
            ><RiResetLeftLine class="h-3.5 w-3.5"
          /></IconButton>
        </div>
      </div>

      <template v-if="isSet">
        <div v-if="node.kind === 'object'" class="contract-body contract-fields">
          <p v-if="fields.length === 0" class="text-xs text-muted-foreground">{{ $st('No fields.') }}</p>
          <PluginContractEditor
            v-for="field in fields"
            :key="field.id"
            class="contract-field"
            :class="{ 'contract-section': isStructured(field), 'contract-card': depth === 0 && isStructured(field) }"
            :node="field"
            :model-value="objectFieldValue(field)"
            :disabled="disabled"
            :depth="depth + 1"
            :optional="!field.required"
            :is-set="field.required || hasField(field)"
            @update:model-value="setField(field, $event)"
            @set="enableField(field)"
            @unset="removeField(field)"
          />
        </div>

        <div v-else-if="node.kind === 'multi_choice'" class="contract-body contract-options">
          <label
            v-for="option in options"
            :key="option.id"
            class="flex min-w-0 cursor-pointer items-start gap-2 rounded-md border border-border/60 px-3 py-2.5 text-sm has-checked:border-primary/40 has-checked:bg-primary/5"
          >
            <input
              class="mt-1 accent-primary"
              type="checkbox"
              :checked="arrayValue.some((value) => enumKey(value) === enumKey(option.value))"
              @change="toggleMulti(option, ($event.target as HTMLInputElement).checked)"
            />
            <span class="min-w-0 break-words">
              <span class="block font-medium">{{ option.title }}</span>
              <span v-if="option.description" class="mt-0.5 block text-xs text-muted-foreground">{{
                option.description
              }}</span>
            </span>
          </label>
        </div>

        <div v-else-if="node.kind === 'list' && listItemNode" class="contract-body space-y-2">
          <p v-if="arrayValue.length === 0" class="text-xs text-muted-foreground">{{ $st('No items.') }}</p>
          <div v-for="(item, index) in arrayValue" :key="index" class="min-w-0 rounded-lg border border-border/60 p-3">
            <div class="mb-2 flex items-center justify-between gap-2">
              <span class="text-[11px] tabular-nums text-muted-foreground">{{ $st('Item') }} {{ index + 1 }}</span>
              <div class="flex gap-1">
                <IconButton
                  size="sm"
                  :tooltip="$st('Up')"
                  :aria-label="`${$st('Up')}: ${index + 1}`"
                  :disabled="index === 0"
                  @click="moveListItem(index, -1)"
                >
                  <RiArrowUpSLine class="h-4 w-4" />
                </IconButton>
                <IconButton
                  size="sm"
                  :tooltip="$st('Down')"
                  :aria-label="`${$st('Down')}: ${index + 1}`"
                  :disabled="index + 1 === arrayValue.length"
                  @click="moveListItem(index, 1)"
                >
                  <RiArrowDownSLine class="h-4 w-4" />
                </IconButton>
                <IconButton
                  size="sm"
                  :tooltip="$st('Remove')"
                  :aria-label="`${$st('Remove')}: ${index + 1}`"
                  @click="removeListItem(index)"
                >
                  <RiCloseLine class="h-4 w-4" />
                </IconButton>
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
          <Button size="sm" variant="outline" type="button" data-add-item @click="addListItem">{{
            $st('Add item')
          }}</Button>
        </div>

        <div v-else-if="node.kind === 'record' && recordValueNode" class="contract-body space-y-2">
          <p v-if="Object.keys(record).length === 0" class="text-xs text-muted-foreground">{{ $st('No entries.') }}</p>
          <div
            v-for="key in Object.keys(record).sort()"
            :key="key"
            class="min-w-0 rounded-lg border border-border/60 p-3"
          >
            <div class="mb-2 flex items-center justify-between gap-2">
              <span class="min-w-0 break-all font-mono text-xs">{{ key }}</span>
              <IconButton
                size="sm"
                :tooltip="$st('Remove')"
                :aria-label="`${$st('Remove')}: ${key}`"
                @click="removeRecordEntry(key)"
              >
                <RiCloseLine class="h-4 w-4" />
              </IconButton>
            </div>
            <PluginContractEditor
              :node="requireNode(recordValueNode)"
              :model-value="recordEntryValue(key)"
              :disabled="disabled"
              :depth="depth + 1"
              @update:model-value="setRecordEntry(key, $event)"
            />
          </div>
          <div class="flex max-w-md items-center gap-2">
            <input
              v-model="recordKey"
              class="contract-input"
              :placeholder="$st('New entry name')"
              :aria-label="$st('New entry name')"
              @keydown.enter.prevent="addRecordEntry"
            />
            <Button size="sm" variant="outline" type="button" :disabled="!canAddRecordEntry" @click="addRecordEntry">{{
              $st('Add')
            }}</Button>
          </div>
        </div>

        <div v-else-if="node.kind === 'tagged_variant'" class="contract-body">
          <p v-if="selectedVariant?.description" class="mb-2 text-xs leading-relaxed text-muted-foreground">
            {{ selectedVariant.description }}
          </p>
          <div v-if="selectedVariant" class="contract-fields">
            <PluginContractEditor
              v-for="field in selectedVariant.fields || []"
              :key="`${selectedVariant.id}:${field.id}`"
              class="contract-field"
              :class="{ 'contract-section': isStructured(field), 'contract-card': depth === 0 && isStructured(field) }"
              :node="field"
              :model-value="objectFieldValue(field)"
              :disabled="disabled"
              :depth="depth + 1"
              :optional="!field.required"
              :is-set="field.required || hasField(field)"
              @update:model-value="setField(field, $event)"
              @set="enableField(field)"
              @unset="removeField(field)"
            />
          </div>
        </div>

        <div v-else-if="node.kind === 'json'" class="contract-body space-y-2">
          <textarea
            v-model="jsonDraft"
            class="contract-input min-h-36 resize-y py-2 font-mono text-xs"
            :aria-label="node.title || node.id"
            :aria-describedby="jsonError ? `${fieldId}-error` : node.description ? `${fieldId}-description` : undefined"
            :aria-invalid="Boolean(jsonError)"
            @blur="commitJson"
            @keydown.meta.enter.prevent="commitJson"
            @keydown.ctrl.enter.prevent="commitJson"
          />
          <div class="flex flex-wrap items-center justify-between gap-2">
            <span class="text-[10px] text-muted-foreground">{{ $st('Explicit bounded JSON field') }}</span>
            <Button size="sm" variant="outline" type="button" @click="commitJson">{{ $st('Apply JSON') }}</Button>
          </div>
          <p v-if="jsonError" :id="`${fieldId}-error`" role="alert" class="break-words text-xs text-destructive">
            {{ jsonError }}
          </p>
        </div>
      </template>
    </div>
  </fieldset>
</template>

<style scoped>
@reference "../../style.css";

.contract-editor {
  container: plugin-field / inline-size;
  min-width: 0;
}

.contract-layout {
  display: grid;
  min-width: 0;
  gap: 0.75rem;
}
.contract-header {
  display: grid;
  min-width: 0;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: start;
  gap: 0.75rem 1rem;
}

.contract-heading {
  grid-column: 1;
}
.contract-body {
  grid-column: 1 / -1;
  min-width: 0;
}
.contract-control {
  display: flex;
  grid-column: 1 / -1;
  align-items: center;
  justify-content: flex-end;
  gap: 0.5rem;
  min-width: 0;
}
.contract-actions {
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
}
.contract-root > .contract-layout > .contract-header > .contract-actions {
  grid-column: 1 / -1;
  justify-content: flex-end;
}
.contract-input {
  @apply h-9 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm outline-none transition-colors focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50;
}
.contract-control-integer > .contract-input,
.contract-control-number > .contract-input {
  max-width: 9rem;
}
.contract-control-choice > .contract-input {
  max-width: 20rem;
}
.contract-toggle {
  align-items: center;
}
.contract-toggle > .contract-control {
  grid-column: 2;
  grid-row: 1;
}
.contract-fields {
  display: grid;
  min-width: 0;
  column-gap: 2rem;
}
.contract-field {
  @apply border-t border-border/50;
  padding-block: 0.875rem;
}
.contract-section {
  grid-column: 1 / -1;
  padding-block: 1.25rem 0.875rem;
}
.contract-section.contract-branch:not(.contract-unset) {
  padding-block-end: 0;
}
.contract-section > .contract-layout > .contract-header > .contract-heading > .contract-title {
  @apply font-semibold;
}

/* One card per top-level settings group; deeper groups stay inside its body. */
.contract-card {
  @apply rounded-xl border border-border/70 bg-card/30;
  margin-block: 0.5rem;
  padding: 0;
}
.contract-card > .contract-layout {
  gap: 0;
}
.contract-card > .contract-layout > .contract-header {
  @apply rounded-t-xl border-b border-border/60 bg-muted/20;
  align-items: center;
  padding: 0.875rem 1rem;
}
.contract-card.contract-unset > .contract-layout > .contract-header {
  @apply rounded-xl border-b-0;
}
.contract-card > .contract-layout > .contract-header > .contract-heading > .contract-title {
  font-size: 0.9375rem;
}
.contract-card > .contract-layout > .contract-body {
  padding: 0.75rem 1rem 1rem;
}
.contract-card > .contract-layout > .contract-fields {
  padding-block: 0.25rem;
}
.contract-card > .contract-layout > .contract-fields > .contract-field:first-child {
  border-block-start-width: 0;
}
.contract-options {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr));
  gap: 0.5rem;
}

/* Adapt to the actual editor pane, including recursive list/record entries. */
@container plugin-field (min-width: 32rem) {
  .contract-row {
    grid-template-columns: minmax(0, 1fr) minmax(10rem, 38%);
    align-items: center;
  }
  .contract-row > .contract-control {
    grid-column: 2;
    grid-row: 1;
  }
  .contract-root > .contract-layout > .contract-header > .contract-actions {
    grid-column: 2;
    grid-row: 1;
  }
}

@container plugin-field (min-width: 68rem) {
  .contract-fields {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .contract-card
    > .contract-layout
    > .contract-fields
    > .contract-field:first-child:not(.contract-section)
    + .contract-field:not(.contract-section) {
    border-block-start-width: 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .contract-editor :deep(*) {
    transition: none;
  }
}
</style>
