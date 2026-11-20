<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  RiDeleteBinLine,
  RiEditLine,
  RiFolderLine,
  RiGlobalLine,
  RiResetLeftLine,
  RiTerminalBoxLine,
  RiToolsLine,
} from '@remixicon/vue'
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
  scope?: string
}>()
const emit = defineEmits<{
  select: [row: PermissionRuleRow, selected: boolean]
  'select-visible': [rows: PermissionRuleRow[], selected: boolean]
  mode: [row: PermissionRuleRow, value: string]
  'rename-target': [row: PermissionRuleRow, event: Event]
  edit: [row: PermissionRuleRow]
  remove: [row: PermissionRuleRow]
  source: [row: PermissionRuleRow]
  reset: [row: PermissionRuleRow]
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
  props.kind === 'command'
    ? st('Command pattern')
    : props.kind === 'filesystem'
      ? st('Path')
      : props.kind === 'network'
        ? st('Rule target')
        : st('Tool name'),
)
const hasCondition = computed(() => props.kind === 'network')
const showSources = computed(() => props.scope !== 'global')
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
const hasActions = computed(() =>
  kindRows.value.some((row) => row.editAction || row.removable || row.sourceAction || row.resetAction),
)
const visibleSelectable = computed(() => visibleRows.value.filter((row) => row.selectable))
const allVisibleSelected = computed(
  () => visibleSelectable.value.length > 0 && visibleSelectable.value.every((row) => selected.value.has(row.key)),
)
const someVisibleSelected = computed(() => visibleSelectable.value.some((row) => selected.value.has(row.key)))
const selectedCount = computed(
  () => kindRows.value.filter((row) => row.selectable && selected.value.has(row.key)).length,
)

function selectRow(row: PermissionRuleRow, event: Event) {
  if (props.disabled || row.readOnly || row.kind !== props.kind || !row.selectable) return
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
    :class="{
      'has-selection': hasSelectable,
      'has-condition': hasCondition,
      'without-source': !showSources,
    }"
    :data-permission-rule-table="kind"
    :aria-label="heading"
  >
    <header class="rule-table-toolbar">
      <div class="rule-table-heading">
        <label v-if="hasSelectable" class="rule-table-select-all" :title="$st('Select all permission rules')">
          <input
            type="checkbox"
            class="accent-primary"
            data-permission-select-all
            :checked="allVisibleSelected"
            :indeterminate="someVisibleSelected && !allVisibleSelected"
            :disabled="disabled || !visibleSelectable.length"
            :aria-label="$st('Select all permission rules')"
            @change="selectVisible"
          />
        </label>
        <h3 class="rule-table-title" data-permission-rule-heading>
          <component :is="kindIcons[kind]" class="h-4 w-4 shrink-0" aria-hidden="true" />
          {{ heading }}<span class="rule-table-count">{{ kindRows.length }}</span>
        </h3>
      </div>
      <SearchInput
        v-model="query"
        class="rule-table-search"
        input-class="h-8 text-[13px] shadow-none"
        :placeholder="$st('Search rules…')"
        :input-aria-label="heading + ' · ' + $st('Search rules…')"
        :input-title="heading"
        :show-search-button="false"
        :clear-aria-label="$st('Clear search')"
        :clear-title="$st('Clear search')"
      />
      <div v-if="$slots.actions" class="rule-table-actions"><slot name="actions" /></div>
    </header>
    <div v-if="selectedCount && $slots.bulk" class="rule-table-bulk">
      <slot name="bulk" :count="selectedCount" />
    </div>
    <slot name="create" />
    <table v-if="visibleRows.length" class="rule-table" role="table" :aria-label="heading">
      <colgroup>
        <col v-if="hasSelectable" class="rule-table-selection-column" />
        <col class="rule-table-target-column" />
        <col v-if="hasCondition" class="rule-table-condition-column" />
        <col class="rule-table-mode-column" />
        <col v-if="showSources" class="rule-table-source-column" />
        <col v-if="hasActions" class="rule-table-action-column" />
      </colgroup>
      <thead role="rowgroup">
        <tr role="row">
          <th v-if="hasSelectable" scope="col" class="rule-table-selection">
            <span class="sr-only">{{ $st('Select') }}</span>
          </th>
          <th scope="col">{{ targetLabel }}</th>
          <th v-if="hasCondition" scope="col">{{ $st('Protocol / ports') }}</th>
          <th scope="col">{{ $st('Mode') }}</th>
          <th v-if="showSources" scope="col">{{ $st('Source') }}</th>
          <th v-if="hasActions" scope="col" class="rule-table-action-heading">{{ $st('Actions') }}</th>
        </tr>
      </thead>
      <tbody role="rowgroup">
        <tr
          v-for="row in visibleRows"
          :key="row.key"
          role="row"
          class="rule-table-row"
          :class="{
            'is-selected': row.selectable && selected.has(row.key),
            'is-inherited': row.readOnly,
          }"
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
              :disabled="disabled || row.readOnly"
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
                :size="Math.max(8, Math.min(row.target.length, 40))"
                type="text"
                class="rule-table-input"
                :title="row.target"
                :aria-label="`${targetLabel}: ${row.target}`"
                :disabled="disabled || row.readOnly"
                @change="emit('rename-target', row, $event)"
              />
              <code v-else class="rule-table-value" :title="row.target">{{ row.target }}</code>
            </div>
          </td>
          <td v-if="hasCondition" class="rule-table-condition">
            <div class="rule-table-condition-content">
              <code v-if="row.condition" class="rule-table-value" :title="row.condition">{{ row.condition }}</code>
              <span v-else class="rule-table-empty-condition text-muted-foreground">—</span>
            </div>
          </td>
          <td class="rule-table-mode" data-permission-mode>
            <OptionPicker
              v-if="row.modeOptions?.length"
              :model-value="row.mode"
              :options="row.modeOptions"
              :include-empty="false"
              :title="`${$st('Mode')}: ${row.target}`"
              :disabled="disabled || row.readOnly"
              size="sm"
              @update:model-value="emit('mode', row, $event)"
            />
            <span v-else class="permission-rule-mode" :data-mode="row.mode">{{ row.modeLabel }}</span>
          </td>
          <td v-if="showSources" class="rule-table-source">
            <span class="permission-rule-source" :data-source="row.source">{{ row.sourceLabel }}</span>
          </td>
          <td v-if="hasActions" class="rule-table-row-actions">
            <div class="rule-table-row-controls">
              <IconButton
                v-if="row.resetAction"
                data-permission-row-action="reset"
                variant="ghost"
                size="sm"
                class="h-8 w-8"
                :tooltip="row.resetActionLabel"
                :aria-label="row.resetActionLabel"
                :disabled="disabled"
                @click="emit('reset', row)"
                ><RiResetLeftLine class="h-4 w-4"
              /></IconButton>
              <IconButton
                v-if="row.sourceAction"
                :data-permission-row-action="'source-' + row.sourceAction"
                variant="ghost"
                size="sm"
                class="h-8 w-8"
                :tooltip="row.sourceActionLabel"
                :aria-label="row.sourceActionLabel"
                :disabled="disabled"
                @click="emit('source', row)"
                ><RiEditLine v-if="row.sourceAction === 'local'" class="h-4 w-4" />
                <RiResetLeftLine v-else class="h-4 w-4" />
              </IconButton>
              <IconButton
                v-if="row.editAction"
                data-permission-row-action="edit"
                variant="ghost"
                size="sm"
                class="h-8 w-8"
                :tooltip="$st('Edit network rule')"
                :aria-label="$st('Edit network rule')"
                :disabled="disabled || row.readOnly"
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
                :disabled="disabled || row.readOnly"
                @click="emit('remove', row)"
                ><RiDeleteBinLine class="h-4 w-4"
              /></IconButton>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
    <div v-else-if="kind !== 'network' || query" class="rule-table-empty">
      <span>{{
        query ? $st('No matching rules.') : $st('No exceptions in this scope. Default policies apply.')
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
  container: permission-rules / inline-size;
  min-width: 0;
  padding: 0.5625rem 0.75rem 0.625rem;
}
.permission-rule-table + .permission-rule-table {
  @apply border-t border-border/70;
}
.rule-table-toolbar {
  display: grid;
  grid-template-columns: minmax(0, max-content) minmax(5rem, 1fr) minmax(0, max-content);
  grid-template-areas: 'heading search actions';
  align-items: center;
  gap: 0.375rem 0.5625rem;
  min-width: 0;
  padding-bottom: 0.5rem;
}
.rule-table-heading {
  grid-area: heading;
  display: flex;
  align-items: center;
  gap: 0.375rem;
  min-width: 0;
}
.rule-table-title {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  min-width: 0;
  font-size: 0.8125rem;
  font-weight: 650;
  line-height: 1.3;
  overflow-wrap: anywhere;
}
.rule-table-title svg {
  @apply text-muted-foreground;
}
.rule-table-select-all {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 1.25rem;
  min-height: 2rem;
  cursor: pointer;
}
.rule-table-count {
  @apply rounded-md bg-muted/45 px-1.5 text-xs font-medium tabular-nums text-muted-foreground;
}
.rule-table-search {
  grid-area: search;
  min-width: 0;
  width: min(100%, 13rem);
  justify-self: end;
}
.rule-table-actions {
  grid-area: actions;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.25rem;
  min-width: 0;
}
.rule-table-actions :deep(button) {
  max-width: 100%;
  height: auto;
  min-height: 2rem;
  white-space: normal;
  line-height: 1.25;
}
.rule-table-actions :deep(button svg) {
  flex-shrink: 0;
}
.rule-table-bulk {
  margin-block-end: 0.375rem;
}
.rule-table {
  width: 100%;
  table-layout: fixed;
  border-collapse: collapse;
  font-size: 0.8125rem;
}
.rule-table-selection-column {
  width: 1.625rem;
}
.rule-table-mode-column {
  width: 6.25rem;
}
[data-permission-rule-table='filesystem'] .rule-table-mode-column {
  width: 7.75rem;
}
.rule-table-source-column {
  width: 6.25rem;
}
.rule-table-action-column {
  width: 5rem;
}
.has-condition .rule-table-target-column {
  width: 26%;
}
.rule-table thead {
  @apply bg-muted/30 text-muted-foreground;
}
.rule-table th {
  height: 1.875rem;
  text-align: start;
  font-size: 0.75rem;
  font-weight: 550;
  line-height: 1.2;
  overflow-wrap: anywhere;
}
.rule-table th:first-child {
  border-start-start-radius: 0.25rem;
}
.rule-table th:last-child {
  border-start-end-radius: 0.25rem;
}
.rule-table th.rule-table-action-heading {
  text-align: end;
}
.rule-table :is(th, td) {
  padding: 0.1875rem 0.375rem;
  vertical-align: middle;
}
.rule-table-row {
  @apply border-b border-border/45 hover:bg-muted/20;
  height: 2.375rem;
}
.rule-table-row:last-child {
  border-bottom: 0;
}
.rule-table-row.is-selected {
  @apply bg-primary/6;
}
.rule-table-selection {
  text-align: center;
}
.rule-table-target-content,
.rule-table-condition-content {
  display: flex;
  align-items: center;
  min-width: 0;
}
.rule-table-row.is-inherited :is(.rule-table-target-content, .rule-table-condition-content) {
  @apply text-muted-foreground;
}
.rule-table-value {
  display: block;
  max-width: 100%;
  min-width: 0;
  font-size: 0.8125rem;
  line-height: 1.4;
  overflow-wrap: anywhere;
}
.rule-table-input {
  @apply rounded border border-transparent bg-transparent focus:border-input focus:outline-none focus:ring-1
    focus:ring-ring disabled:opacity-60;
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
  padding-block: 0.1875rem;
  padding-inline: 0.375rem;
  gap: 0.25rem;
  font-size: 0.8125rem;
  text-align: start;
}
.rule-table-mode > :deep(div) {
  min-width: 0;
  max-width: 8rem;
}
.rule-table-mode :deep(.truncate) {
  overflow: visible;
  overflow-wrap: anywhere;
  text-overflow: clip;
  white-space: normal;
  line-height: 1.25;
}
.permission-rule-mode {
  @apply inline-block rounded-md bg-muted/55 px-1.5 py-0.5;
  max-width: 100%;
  font-size: 0.75rem;
  font-weight: 550;
  line-height: 1.3;
  overflow-wrap: anywhere;
}
.permission-rule-mode[data-mode='allow'] {
  @apply bg-primary/10;
}
.permission-rule-mode[data-mode='deny'] {
  @apply bg-destructive/10 text-destructive;
}
.permission-rule-source {
  @apply text-xs text-muted-foreground;
  line-height: 1.3;
  overflow-wrap: anywhere;
}
.rule-table-row-controls {
  display: flex;
  justify-content: flex-end;
  gap: 0.125rem;
}
.rule-table-empty {
  @apply flex flex-wrap items-center gap-2 rounded-md bg-muted/20 px-2.5 py-2 text-muted-foreground;
}
@container permission-rules (max-width: 32rem) {
  .rule-table-toolbar {
    grid-template-columns: minmax(0, 1fr) minmax(0, max-content);
    grid-template-areas: 'heading actions' 'search search';
  }
  .rule-table-search {
    width: 100%;
  }
  .rule-table-actions :deep(button) {
    max-width: 9rem;
  }
}
@container permission-rules (max-width: 33rem) {
  .rule-table,
  .rule-table tbody {
    display: block;
    width: 100%;
  }
  .rule-table colgroup {
    display: none;
  }
  .rule-table thead {
    @apply sr-only;
  }
  .rule-table-row {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, max-content) auto;
    grid-template-areas:
      'target mode actions'
      'source mode actions';
    align-items: center;
    gap: 0 0.5rem;
    width: 100%;
    min-height: 2.375rem;
    height: auto;
    padding: 0.25rem 0;
  }
  .has-condition .rule-table-row {
    grid-template-areas:
      'target mode actions'
      'condition mode actions'
      'source mode actions';
  }
  .without-source .rule-table-row {
    grid-template-areas: 'target mode actions';
  }
  .without-source.has-condition .rule-table-row {
    grid-template-areas:
      'target mode actions'
      'condition mode actions';
  }
  .has-selection .rule-table-row {
    padding-inline-start: 1.375rem;
  }
  .rule-table td {
    display: block;
    min-width: 0;
    padding: 0;
  }
  .rule-table .rule-table-selection {
    position: absolute;
    inset-inline-start: 0.125rem;
    inset-block-start: 0.625rem;
  }
  .rule-table-target {
    grid-area: target;
  }
  .rule-table-condition {
    grid-area: condition;
  }
  .rule-table-source {
    grid-area: source;
  }
  .rule-table-mode {
    grid-area: mode;
    min-width: 0;
    max-width: min(7rem, 29cqi);
  }
  .rule-table-row-actions {
    grid-area: actions;
  }
  .rule-table-empty-condition {
    display: none;
  }
}
@media (pointer: coarse) {
  .rule-table-row-actions :deep(button),
  .rule-table-mode :deep(button[aria-haspopup]) {
    min-height: 2.375rem;
  }
}
</style>
