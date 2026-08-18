<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RiAddLine, RiArrowDownLine, RiArrowUpLine, RiDeleteBinLine, RiFileCopyLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import type { JsonObject, JsonValue } from '@/types/json'
import {
  cloneJson,
  defaultValueForSchema,
  isJsonRecord,
  normalizeJsonSchema,
  schemaBranches,
  schemaDescription,
  schemaEnumOptions,
  schemaMatchesValue,
  schemaProperties,
  schemaRequired,
  schemaTitle,
  schemaType,
  stableJson,
} from './pluginConfigSchema'
import { settingsText as st } from '@/i18n/settingsText'

const props = withDefaults(
  defineProps<{
    modelValue: JsonValue
    schema: JsonValue
    rootSchema?: JsonValue
    label?: string
    description?: string
    required?: boolean
    disabled?: boolean
    path?: string
    nested?: boolean
  }>(),
  {
    rootSchema: null,
    label: '',
    description: '',
    required: false,
    disabled: false,
    path: '',
    nested: false,
  },
)

const emit = defineEmits<{ (event: 'update:modelValue', value: JsonValue): void }>()

const root = computed(() => props.rootSchema || props.schema)
const normalizedSchema = computed(() => normalizeJsonSchema(props.schema, root.value))
const kind = computed(() => schemaType(normalizedSchema.value, root.value))
const title = computed(() => props.label || schemaTitle(normalizedSchema.value, props.path.split('.').pop() || 'Value'))
const help = computed(() => props.description || schemaDescription(normalizedSchema.value))
const properties = computed(() => schemaProperties(normalizedSchema.value, root.value))
const requiredKeys = computed(() => schemaRequired(normalizedSchema.value, root.value))
const branches = computed(() => schemaBranches(normalizedSchema.value, root.value))
const enumOptions = computed(() => schemaEnumOptions(normalizedSchema.value, root.value))
const branchOptions = computed(() =>
  branches.value.map((branch, index) => ({
    value: String(index),
    label: schemaTitle(
      branch,
      st('{type} {index}', {
        type: schemaType(branch, root.value) || st('Variant'),
        index: index + 1,
      }),
    ),
    description: schemaDescription(branch) || undefined,
  })),
)
const selectedBranch = computed(() => {
  const index = branches.value.findIndex((branch) => schemaMatchesValue(branch, props.modelValue, root.value))
  return index >= 0 ? index : 0
})
const objectValue = computed<JsonObject>(() => (isJsonRecord(props.modelValue) ? props.modelValue : {}))
const defaultObjectValue = computed<JsonObject>(() => {
  const value = defaultValueForSchema(normalizedSchema.value, root.value)
  return isJsonRecord(value) ? value : {}
})
const arrayValue = computed<JsonValue[]>(() => (Array.isArray(props.modelValue) ? props.modelValue : []))
const itemSchema = computed(() => {
  const schema = normalizedSchema.value
  return isJsonRecord(schema) && schema.items !== undefined ? schema.items : {}
})
const rawText = ref('')
const rawError = ref('')

function syncRaw() {
  try {
    rawText.value = JSON.stringify(props.modelValue ?? null, null, 2)
  } catch {
    rawText.value = 'null'
  }
  rawError.value = ''
}

watch(() => props.modelValue, syncRaw, { immediate: true, deep: true })

function updateChild(key: string, value: JsonValue) {
  const next: JsonObject = cloneJson(objectValue.value)
  next[key] = value
  emit('update:modelValue', next)
}

function clearChild(key: string) {
  const next: JsonObject = cloneJson(objectValue.value)
  if (Object.prototype.hasOwnProperty.call(defaultObjectValue.value, key)) {
    next[key] = cloneJson(defaultObjectValue.value[key])
  } else {
    delete next[key]
  }
  emit('update:modelValue', next)
}

function clearChildLabel(key: string): string {
  return Object.prototype.hasOwnProperty.call(defaultObjectValue.value, key)
    ? st('Reset / inherit')
    : st('Inherit / remove')
}

function updateArray(index: number, value: JsonValue) {
  const next = cloneJson(arrayValue.value)
  next[index] = value
  emit('update:modelValue', next)
}

function addArrayItem() {
  emit('update:modelValue', [...cloneJson(arrayValue.value), defaultValueForSchema(itemSchema.value, root.value)])
}

function removeArrayItem(index: number) {
  const next = cloneJson(arrayValue.value)
  next.splice(index, 1)
  emit('update:modelValue', next)
}

function moveArrayItem(index: number, delta: -1 | 1) {
  const target = index + delta
  if (target < 0 || target >= arrayValue.value.length) return
  const next = cloneJson(arrayValue.value)
  const [item] = next.splice(index, 1)
  if (item === undefined) return
  next.splice(target, 0, item)
  emit('update:modelValue', next)
}

function duplicateArrayItem(index: number) {
  const item = arrayValue.value[index]
  if (item === undefined) return
  const next = cloneJson(arrayValue.value)
  next.splice(index + 1, 0, cloneJson(item))
  emit('update:modelValue', next)
}

function chooseEnum(serialized: string) {
  const option = enumOptions.value.find((item) => item.value === serialized)
  if (option) emit('update:modelValue', cloneJson(option.raw))
}

function chooseBranch(serialized: string) {
  const index = Number(serialized)
  const branch = Number.isInteger(index) ? branches.value[index] : null
  if (branch) emit('update:modelValue', defaultValueForSchema(branch, root.value))
}

function updateText(value: string | number) {
  emit('update:modelValue', String(value))
}

function updateNumber(value: string | number) {
  const numeric = Number(value)
  emit('update:modelValue', Number.isFinite(numeric) ? (kind.value === 'integer' ? Math.trunc(numeric) : numeric) : 0)
}

function applyRaw() {
  try {
    emit('update:modelValue', JSON.parse(rawText.value) as JsonValue)
    rawError.value = ''
  } catch (reason) {
    rawError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

const enumValue = computed(() => stableJson(props.modelValue))
const scalarType = computed(() => typeof props.modelValue)
const showMultiline = computed(() => {
  if (!isJsonRecord(normalizedSchema.value)) return false
  return normalizedSchema.value.format === 'multiline' || normalizedSchema.value['x-multiline'] === true
})
</script>

<template>
  <div :class="nested ? 'grid min-w-0 gap-2' : 'grid min-w-0 gap-3 rounded-lg border border-border/60 p-3'">
    <div v-if="title || help" class="min-w-0">
      <div v-if="title" class="flex min-w-0 items-center gap-2 text-sm font-medium">
        <span class="break-words">{{ title }}</span>
        <span v-if="required" class="text-destructive" :title="$st('Required')">*</span>
        <code v-if="path" class="ml-auto hidden truncate font-mono text-[10px] text-muted-foreground sm:block">{{
          path
        }}</code>
      </div>
      <p v-if="help" class="mt-1 text-xs leading-5 text-muted-foreground">{{ help }}</p>
    </div>

    <template v-if="branches.length > 0 && enumOptions.length === 0">
      <OptionPicker
        :model-value="String(selectedBranch)"
        :options="branchOptions"
        :title="$st('{title} variant', { title })"
        :include-empty="false"
        :disabled="disabled"
        @update:model-value="chooseBranch"
      />
      <JsonSchemaField
        :model-value="modelValue"
        :schema="branches[selectedBranch]"
        :root-schema="root"
        :disabled="disabled"
        :path="path"
        nested
        @update:model-value="emit('update:modelValue', $event)"
      />
    </template>

    <OptionPicker
      v-else-if="enumOptions.length > 0"
      :model-value="enumValue"
      :options="enumOptions"
      :title="title"
      :include-empty="false"
      :disabled="disabled"
      monospace
      @update:model-value="chooseEnum"
    />

    <label v-else-if="kind === 'boolean'" class="inline-flex min-h-9 items-center gap-2 text-sm">
      <input
        :checked="modelValue === true"
        type="checkbox"
        :disabled="disabled"
        @change="emit('update:modelValue', ($event.target as HTMLInputElement).checked)"
      />
      {{ modelValue === true ? $st('Enabled') : $st('Disabled') }}
    </label>

    <textarea
      v-else-if="kind === 'string' && showMultiline"
      :value="typeof modelValue === 'string' ? modelValue : ''"
      rows="5"
      :disabled="disabled"
      class="w-full rounded-md border border-input bg-transparent p-3 text-sm outline-none focus:border-ring"
      @input="updateText(($event.target as HTMLTextAreaElement).value)"
    />

    <Input
      v-else-if="kind === 'string'"
      :model-value="typeof modelValue === 'string' ? modelValue : ''"
      :disabled="disabled"
      @update:model-value="updateText"
    />

    <Input
      v-else-if="kind === 'integer' || kind === 'number'"
      type="number"
      :model-value="typeof modelValue === 'number' ? modelValue : 0"
      :disabled="disabled"
      @update:model-value="updateNumber"
    />

    <div v-else-if="kind === 'object' && properties.length > 0" class="grid min-w-0 gap-3">
      <div
        v-for="[key, childSchema] in properties"
        :key="key"
        class="grid min-w-0 gap-2 border-t border-border/50 pt-3 first:border-t-0 first:pt-0"
      >
        <div v-if="!requiredKeys.has(key)" class="flex justify-end">
          <Button
            v-if="Object.prototype.hasOwnProperty.call(objectValue, key)"
            variant="ghost"
            size="sm"
            :disabled="disabled"
            class="text-muted-foreground"
            @click="clearChild(key)"
          >
            <RiDeleteBinLine class="mr-1.5 h-4 w-4" /> {{ clearChildLabel(key) }}
          </Button>
          <Button
            v-else
            variant="outline"
            size="sm"
            :disabled="disabled"
            @click="updateChild(key, defaultValueForSchema(childSchema, root))"
          >
            <RiAddLine class="mr-1.5 h-4 w-4" /> {{ $st('Configure') }} {{ schemaTitle(childSchema, key) }}
          </Button>
        </div>
        <JsonSchemaField
          v-if="requiredKeys.has(key) || Object.prototype.hasOwnProperty.call(objectValue, key)"
          :model-value="objectValue[key]"
          :schema="childSchema"
          :root-schema="root"
          :label="schemaTitle(childSchema, key)"
          :description="schemaDescription(childSchema)"
          :required="requiredKeys.has(key)"
          :disabled="disabled"
          :path="path ? `${path}.${key}` : key"
          nested
          @update:model-value="updateChild(key, $event)"
        />
      </div>
    </div>

    <div v-else-if="kind === 'array'" class="grid gap-3">
      <div
        v-for="(item, index) in arrayValue"
        :key="index"
        class="grid gap-2 rounded-md border border-border/50 bg-muted/10 p-3"
      >
        <div class="flex items-center justify-between gap-2">
          <span class="text-xs font-medium text-muted-foreground">{{ $st('Item') }} {{ index + 1 }}</span>
          <div class="flex flex-wrap gap-1">
            <Button variant="ghost" size="sm" :disabled="disabled || index === 0" @click="moveArrayItem(index, -1)">
              <RiArrowUpLine class="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              :disabled="disabled || index >= arrayValue.length - 1"
              @click="moveArrayItem(index, 1)"
            >
              <RiArrowDownLine class="h-4 w-4" />
            </Button>
            <Button variant="ghost" size="sm" :disabled="disabled" @click="duplicateArrayItem(index)">
              <RiFileCopyLine class="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              :disabled="disabled"
              class="text-destructive"
              @click="removeArrayItem(index)"
            >
              <RiDeleteBinLine class="mr-1.5 h-4 w-4" /> {{ $st('Remove') }}
            </Button>
          </div>
        </div>
        <JsonSchemaField
          :model-value="item"
          :schema="itemSchema"
          :root-schema="root"
          :disabled="disabled"
          :path="`${path}[${index}]`"
          nested
          @update:model-value="updateArray(index, $event)"
        />
      </div>
      <Button variant="outline" size="sm" :disabled="disabled" class="justify-self-start" @click="addArrayItem">
        <RiAddLine class="mr-1.5 h-4 w-4" /> {{ $st('Add item') }}
      </Button>
    </div>

    <div v-else-if="kind === 'null'" class="text-xs text-muted-foreground">
      {{ $st('This value is explicitly null.') }}
    </div>

    <div v-else class="grid gap-2">
      <div class="text-xs text-muted-foreground">
        {{ $st('Structured controls are unavailable for this') }} {{ scalarType }}
        {{ $st('value. Edit its JSON directly.') }}
      </div>
      <textarea
        v-model="rawText"
        rows="6"
        spellcheck="false"
        :disabled="disabled"
        class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs outline-none focus:border-ring"
      />
      <div class="flex flex-wrap items-center justify-between gap-2">
        <span v-if="rawError" class="text-xs text-destructive">{{ rawError }}</span>
        <span v-else></span>
        <Button variant="outline" size="sm" :disabled="disabled" @click="applyRaw">{{ $st('Apply JSON') }}</Button>
      </div>
    </div>
  </div>
</template>
