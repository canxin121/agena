<script setup lang="ts">
import { computed, ref } from 'vue'
import { RiDeleteBinLine, RiEditLine, RiFolderLine, RiGlobalLine, RiTerminalBoxLine, RiToolsLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { settingsText as st } from '@/i18n/settingsText'
import type { PermissionRuleKind, PermissionRuleRow } from './permissionRuleTable'

const props = defineProps<{
  kind: PermissionRuleKind
  rows: PermissionRuleRow[]
  selectedKeys: string[]
  disabled?: boolean
}>()
const emit = defineEmits<{
  select: [row: PermissionRuleRow, selected: boolean]
  'select-visible': [rows: PermissionRuleRow[], selected: boolean]
  mode: [row: PermissionRuleRow, value: string]
  'rename-target': [row: PermissionRuleRow, event: Event]
  'rename-condition': [row: PermissionRuleRow, event: Event]
  edit: [row: PermissionRuleRow]
  remove: [row: PermissionRuleRow]
}>()

const query = ref('')
const kindLabels = computed<Record<PermissionRuleKind, string>>(() => ({
  filesystem: st('Path Rules'),
  network: st('Network Rules'),
  'tool-name': st('Name Rules'),
  command: st('Command Rules'),
}))
const kindIcons = {
  filesystem: RiFolderLine,
  network: RiGlobalLine,
  'tool-name': RiToolsLine,
  command: RiTerminalBoxLine,
}
const heading = computed(() => kindLabels.value[props.kind])
const targetLabel = computed(() =>
  props.kind === 'filesystem' ? st('Path') : props.kind === 'network' ? st('Rule target') : st('Tool name'),
)
const hasCondition = computed(() => props.kind === 'network' || props.kind === 'command')
const conditionLabel = computed(() => (props.kind === 'network' ? st('Protocol / ports') : st('Command pattern')))
// The table has a fixed rule kind. A caller cannot accidentally render or select
// another domain by passing a combined array or a key selected in another table.
const kindRows = computed(() => props.rows.filter((row) => row.kind === props.kind))
const visibleRows = computed(() => {
  const search = query.value.trim().toLowerCase()
  return kindRows.value.filter(
    (row) =>
      !search ||
      [row.target, row.condition, row.modeLabel, row.sourceLabel].some((value) => value.toLowerCase().includes(search)),
  )
})
const selected = computed(() => new Set(props.selectedKeys))
const hasSelectable = computed(() => kindRows.value.some((row) => row.selectable))
const hasActions = computed(() => kindRows.value.some((row) => row.editAction || row.removable))
const visibleSelectable = computed(() => visibleRows.value.filter((row) => row.selectable))
const allVisibleSelected = computed(
  () => visibleSelectable.value.length > 0 && visibleSelectable.value.every((row) => selected.value.has(row.key)),
)
const someVisibleSelected = computed(() => visibleSelectable.value.some((row) => selected.value.has(row.key)))
const selectedCount = computed(
  () => kindRows.value.filter((row) => row.selectable && selected.value.has(row.key)).length,
)

function selectRow(row: PermissionRuleRow, event: Event) {
  if (props.disabled || row.kind !== props.kind || !row.selectable) return
  emit('select', row, (event.target as HTMLInputElement).checked)
}
function selectVisible() {
  if (props.disabled || !visibleSelectable.value.length) return
  emit('select-visible', visibleSelectable.value, !allVisibleSelected.value)
}
function clearSearch() {
  query.value = ''
}
</script>

<template>
  <section
    class="permission-rule-table"
    :class="{ 'has-selection': hasSelectable, 'has-condition': hasCondition }"
    :data-permission-rule-table="kind"
    :aria-labelledby="'permission-rule-title-' + kind"
  >
    <header class="rule-table-toolbar">
      <h2 :id="'permission-rule-title-' + kind" class="rule-table-title">
        <component :is="kindIcons[kind]" class="h-4 w-4 shrink-0" aria-hidden="true" />
        {{ heading }}<span class="rule-table-count">{{ kindRows.length }}</span>
      </h2>
      <SearchInput
        v-model="query"
        class="rule-table-search"
        input-class="h-8 text-[13px]"
        :placeholder="$st('Search rules…')"
        :input-aria-label="heading + ' · ' + $st('Search rules…')"
        :input-title="heading"
        :show-search-button="false"
        :clear-aria-label="$st('Clear search')"
        :clear-title="$st('Clear search')"
      />
      <div v-if="$slots.actions" class="rule-table-actions"><slot name="actions" /></div>
    </header>
    <div v-if="selectedCount" class="rule-table-bulk">
      <span class="text-[13px] font-medium">{{ $st('{count} selected', { count: selectedCount }) }}</span>
      <slot name="bulk" />
    </div>
    <slot name="create" />
    <table v-if="visibleRows.length" class="rule-table" :aria-label="heading">
      <colgroup>
        <col v-if="hasSelectable" class="rule-table-selection-column" />
        <col />
        <col v-if="hasCondition" />
        <col class="rule-table-mode-column" />
        <col class="rule-table-source-column" />
        <col v-if="hasActions" class="rule-table-action-column" />
      </colgroup>
      <thead>
        <tr>
          <th v-if="hasSelectable" scope="col" class="rule-table-selection">
            <input
              type="checkbox"
              class="accent-primary"
              :checked="allVisibleSelected"
              :indeterminate="someVisibleSelected && !allVisibleSelected"
              :disabled="disabled || !visibleSelectable.length"
              :aria-label="$st('Select all permission rules')"
              @change="selectVisible"
            />
          </th>
          <th scope="col">{{ targetLabel }}</th>
          <th v-if="hasCondition" scope="col">{{ conditionLabel }}</th>
          <th scope="col">{{ $st('Mode') }}</th>
          <th scope="col">{{ $st('Source') }}</th>
          <th v-if="hasActions" scope="col" class="text-right">{{ $st('Actions') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="row in visibleRows"
          :key="row.key"
          class="rule-table-row"
          :class="{ 'is-selected': row.selectable && selected.has(row.key) }"
          :data-permission-rule="row.kind"
          :data-rule-scope="row.source"
          :data-rule-action="row.networkAction"
          :data-rule-key="encodeURIComponent(row.key)"
          :data-rule-target="row.target"
          :data-rule-condition="row.condition"
        >
          <td v-if="hasSelectable" class="rule-table-selection">
            <input
              v-if="row.selectable"
              type="checkbox"
              data-permission-row-action="select"
              class="accent-primary"
              :checked="selected.has(row.key)"
              :disabled="disabled"
              :aria-label="`${$st('Select')} ${row.target}${row.condition ? ` · ${row.condition}` : ''}`"
              @change="selectRow(row, $event)"
            />
          </td>
          <td class="rule-table-target">
            <div class="rule-table-target-content">
              <input
                v-if="row.targetEditable"
                data-permission-row-action="rename-target"
                :value="row.target"
                type="text"
                class="rule-table-input"
                :title="row.target"
                :aria-label="`${targetLabel}: ${row.target}`"
                :disabled="disabled"
                @change="emit('rename-target', row, $event)"
              />
              <code v-else class="rule-table-value" :title="row.target">{{ row.target }}</code>
            </div>
          </td>
          <td v-if="hasCondition" class="rule-table-condition">
            <div class="rule-table-condition-content">
              <input
                v-if="row.conditionEditable"
                data-permission-row-action="rename-condition"
                :value="row.condition"
                type="text"
                class="rule-table-input"
                :title="row.condition"
                :aria-label="`${conditionLabel}: ${row.target}`"
                :disabled="disabled"
                @change="emit('rename-condition', row, $event)"
              />
              <code v-else-if="row.condition" class="rule-table-value" :title="row.condition">{{ row.condition }}</code>
              <span v-else class="rule-table-empty-condition text-muted-foreground">—</span>
              <span class="rule-table-mobile-source permission-rule-source" :data-source="row.source">{{
                row.sourceLabel
              }}</span>
            </div>
          </td>
          <td class="rule-table-mode" data-permission-mode>
            <OptionPicker
              v-if="row.modeOptions?.length"
              :model-value="row.mode"
              :options="row.modeOptions"
              :include-empty="false"
              :title="`${$st('Mode')}: ${row.target}`"
              :disabled="disabled"
              size="sm"
              @update:model-value="emit('mode', row, $event)"
            />
            <span v-else class="permission-rule-mode" :data-mode="row.mode">{{ row.modeLabel }}</span>
          </td>
          <td class="rule-table-source">
            <span class="permission-rule-source" :data-source="row.source">{{ row.sourceLabel }}</span>
          </td>
          <td v-if="hasActions" class="rule-table-row-actions">
            <div class="flex justify-end gap-0.5">
              <IconButton
                v-if="row.editAction"
                data-permission-row-action="edit"
                variant="ghost"
                size="sm"
                class="h-8 w-8"
                :tooltip="$st('Edit network rule')"
                :aria-label="$st('Edit network rule')"
                :disabled="disabled"
                @click="emit('edit', row)"
                ><RiEditLine class="h-4 w-4"
              /></IconButton>
              <IconButton
                v-if="row.removable"
                data-permission-row-action="remove"
                variant="ghost"
                size="sm"
                class="h-8 w-8"
                :tooltip="$st('Remove rule')"
                :aria-label="$st('Remove rule')"
                :disabled="disabled"
                @click="emit('remove', row)"
                ><RiDeleteBinLine class="h-4 w-4"
              /></IconButton>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
    <div v-else class="rule-table-empty">
      <span>{{
        kindRows.length ? $st('No matching rules.') : $st('No exceptions in this scope. Default policies apply.')
      }}</span>
      <Button v-if="query" type="button" variant="ghost" size="sm" @click="clearSearch">{{
        $st('Clear search')
      }}</Button>
    </div>
  </section>
</template>

<style scoped>
@reference '../../../style.css';
.permission-rule-table {
  @apply border-t border-border/70;
  min-width: 0;
  container: permission-rules / inline-size;
}
.rule-table-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.375rem 0.75rem;
  padding-block: 0.375rem;
}
.rule-table-title {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  font-size: 0.875rem;
  font-weight: 600;
}
.rule-table-count {
  @apply text-muted-foreground;
  font-size: 0.8125rem;
  font-weight: 400;
  font-variant-numeric: tabular-nums;
}
.rule-table-search {
  flex: 1 1 12rem;
  min-width: 0;
  max-width: 22rem;
  margin-inline-start: auto;
}
.rule-table-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.25rem;
}
.rule-table-bulk {
  @apply border-y border-primary/20 bg-primary/5;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.375rem;
  padding: 0.25rem 0.375rem;
}
.rule-table {
  width: 100%;
  table-layout: fixed;
  border-collapse: collapse;
  font-size: 0.8125rem;
}
.rule-table-selection-column {
  width: 1.75rem;
}
.rule-table-mode-column {
  width: 10rem;
}
.rule-table-source-column {
  width: 6rem;
}
.rule-table-action-column {
  width: 4.75rem;
}
.rule-table th {
  @apply text-muted-foreground;
  height: 1.75rem;
  text-align: start;
  font-size: 0.8125rem;
  font-weight: 500;
}
.rule-table :is(th, td) {
  padding: 0.0625rem 0.375rem;
  vertical-align: middle;
}
.rule-table-row {
  @apply border-t border-border/50 hover:bg-muted/20;
  height: 2.25rem;
}
.rule-table-row.is-selected {
  @apply bg-primary/5;
}
.rule-table-selection {
  text-align: center;
}
.rule-table-target-content,
.rule-table-condition-content {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  min-width: 0;
}
.rule-table-value {
  display: block;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 0.8125rem;
}
.rule-table-input {
  @apply rounded border border-transparent bg-transparent focus:border-input focus:outline-none focus:ring-1 focus:ring-ring disabled:opacity-60;
  width: 100%;
  min-width: 0;
  height: 2rem;
  padding-inline: 0.25rem;
  font-family: var(--font-mono);
  font-size: 0.8125rem;
}
.rule-table-mode :deep(button[aria-haspopup]) {
  height: auto;
  min-height: 2rem;
  padding-block: 0.25rem;
  font-size: 0.8125rem;
}
.rule-table-mode :deep(.truncate) {
  overflow: visible;
  text-overflow: clip;
  white-space: normal;
  line-height: 1.2;
}
.permission-rule-mode,
.permission-rule-source {
  display: inline-block;
  max-width: 100%;
  font-size: 0.8125rem;
  line-height: 1.3;
  white-space: normal;
}
.permission-rule-mode[data-mode='deny'] {
  @apply text-destructive;
}
.permission-rule-source {
  @apply text-muted-foreground;
}
.rule-table-mobile-source {
  display: none;
}
.rule-table-empty {
  @apply border-t border-border/50 text-muted-foreground;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.375rem;
  padding: 0.5rem 0;
  font-size: 0.8125rem;
}
@container permission-rules (max-width: 49rem) {
  .rule-table-toolbar {
    gap: 0.375rem 0.5rem;
  }
  .rule-table-search {
    max-width: none;
  }
  .rule-table,
  .rule-table tbody {
    display: block;
  }
  .rule-table colgroup,
  .rule-table thead {
    display: none;
  }
  .rule-table-row {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(7.5rem, max-content);
    grid-template-rows: auto auto;
    align-items: center;
    gap: 0.125rem 0.5rem;
    height: auto;
    min-height: 4.25rem;
    padding-block: 0.25rem;
  }
  .has-selection .rule-table-row {
    padding-inline-start: 1.5rem;
  }
  .rule-table td {
    display: block;
    min-width: 0;
    padding: 0;
  }
  .rule-table td.rule-table-selection {
    position: absolute;
    inset-inline-start: 0;
    top: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
  }
  .has-condition .rule-table td.rule-table-source {
    display: none;
  }
  .rule-table-target {
    grid-column: 1;
    grid-row: 1;
  }
  .rule-table-condition {
    grid-column: 1;
    grid-row: 2;
  }
  .rule-table-source {
    grid-column: 1;
    grid-row: 2;
  }
  .rule-table-mode {
    grid-column: 2;
    grid-row: 1;
  }
  .rule-table-row-actions {
    grid-column: 2;
    grid-row: 2;
  }
  .rule-table-mobile-source {
    display: inline-block;
    flex-shrink: 0;
  }
  .rule-table-empty-condition {
    display: none;
  }
  .rule-table-condition-content {
    gap: 0.375rem;
  }
  .rule-table-condition-content .rule-table-value {
    flex: 1 1 auto;
  }
  .rule-table-mode {
    max-width: 11rem;
  }
}
@media (pointer: coarse) {
  .rule-table-row-actions :deep(button),
  .rule-table-mode :deep(button[aria-haspopup]) {
    min-height: 2.5rem;
  }
}
</style>
