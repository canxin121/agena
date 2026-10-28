<script setup lang="ts">
import { computed, defineAsyncComponent } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiInformationLine, RiAlertLine, RiArrowUpLine, RiArrowDownLine } from '@remixicon/vue'
import { containerKinds, uiDisplay, type UiResolvedNode } from './program'
import type { ChartSpec } from './charts'
const InlineChart = defineAsyncComponent(() => import('./InlineChart.vue'))
const props = defineProps<{ node: UiResolvedNode; scope: string }>()
const emit = defineEmits<{ input: [id: string, value: unknown]; activate: [id: string] }>()
const { t } = useI18n()
const p = computed(() => props.node.props)
const domId = computed(() => `ui-${props.scope.replace(/[^a-zA-Z0-9_-]/g, '-')}-${props.node.id}`)
const isContainer = computed(() => containerKinds.has(props.node.kind) && props.node.kind !== 'details')
const items = computed(() => (Array.isArray(p.value.items) ? (p.value.items as Record<string, unknown>[]) : []))
const columns = computed(() =>
  Array.isArray(p.value.columns) ? (p.value.columns as { key: string; label: string }[]) : [],
)
const rows = computed(() => (Array.isArray(p.value.rows) ? (p.value.rows as Record<string, unknown>[]) : []))
const disabled = computed(() => p.value.disabled === true)
const progress = computed(() => {
  const max = Number(p.value.max ?? 100)
  return max > 0 ? Math.max(0, Math.min(100, (Number(p.value.value || 0) / max) * 100)) : 0
})
const chart = computed<ChartSpec>(() => ({
  type: (p.value.chart_type || 'line') as ChartSpec['type'],
  title: String(p.value.title || ''),
  labels: (p.value.labels || []) as string[],
  series: (p.value.series || []) as ChartSpec['series'],
  xLabel: String(p.value.x_label || ''),
  yLabel: String(p.value.y_label || ''),
}))
const display = (key: string) => (p.value[key] == null ? '' : uiDisplay(p.value[key]))
function input(event: Event, numeric = false) {
  const element = event.target as HTMLInputElement
  if (numeric && element.value.trim() === '') return
  const value = numeric ? element.valueAsNumber : element.value
  if (typeof value !== 'number' || Number.isFinite(value)) emit('input', props.node.id, value)
}
function checked(id: unknown) {
  return Array.isArray(props.node.value) && props.node.value.includes(id)
}
function toggleTask(id: unknown) {
  const current = Array.isArray(props.node.value) ? props.node.value : []
  emit('input', props.node.id, checked(id) ? current.filter((v) => v !== id) : [...current, id])
}
function tabKey(event: KeyboardEvent) {
  const options = props.node.field?.options || []
  if (!options.length || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return
  event.preventDefault()
  const current = options.findIndex((o) => o.value === props.node.value)
  const index =
    event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? options.length - 1
        : (current + (event.key === 'ArrowRight' ? 1 : -1) + options.length) % options.length
  emit('input', props.node.id, options[index]!.value)
  ;(event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>('button')[index]?.focus()
}
</script>

<template>
  <section
    v-if="isContainer"
    class="ui-node ui-layout"
    :class="`ui-${node.kind}`"
    :data-ui-node="node.id"
    :data-tone="p.tone || 'neutral'"
    :data-gap="p.gap || 'md'"
    :data-columns="node.kind === 'grid' ? Number(p.columns || 2) : undefined"
    :data-metrics="
      node.kind === 'grid' && node.children.length > 0 && node.children.every((child) => child.kind === 'metric')
        ? 'true'
        : undefined
    "
    :style="node.kind === 'grid' ? { '--ui-columns': Math.max(1, Math.min(4, Number(p.columns || 2))) } : undefined"
  >
    <header v-if="p.title || p.description" class="ui-section-header">
      <h4 v-if="p.title">{{ display('title') }}</h4>
      <p v-if="p.description">{{ display('description') }}</p>
    </header>
    <ProgramNode
      v-for="child in node.children"
      :key="child.id"
      :node="child"
      :scope="scope"
      @input="(id, value) => emit('input', id, value)"
      @activate="(id) => emit('activate', id)"
    />
  </section>
  <h3 v-else-if="node.kind === 'heading'" class="ui-heading" :data-size="p.size || 'md'" :data-ui-node="node.id">
    {{ display('text') }}
  </h3>
  <p
    v-else-if="node.kind === 'text'"
    class="ui-text"
    :data-size="p.size || 'md'"
    :data-tone="p.tone || 'neutral'"
    :data-ui-node="node.id"
  >
    {{ display('text') }}
  </p>
  <div v-else-if="node.kind === 'metric'" class="ui-metric" :data-tone="p.tone || 'neutral'" :data-ui-node="node.id">
    <span class="ui-caption">{{ display('label') }}</span>
    <div class="ui-metric-value">
      <span v-if="p.prefix" class="ui-unit">{{ display('prefix') }}</span
      >{{ display('value') }}<span v-if="p.suffix" class="ui-unit">{{ display('suffix') }}</span>
    </div>
    <div v-if="p.detail || p.trend != null" class="ui-metric-detail">
      <span v-if="p.trend != null" class="ui-trend"
        ><component :is="Number(p.trend) >= 0 ? RiArrowUpLine : RiArrowDownLine" aria-hidden="true" />{{
          display('trend')
        }}</span
      >{{ display('detail') }}
    </div>
  </div>
  <span v-else-if="node.kind === 'badge'" class="ui-badge" :data-tone="p.tone || 'neutral'" :data-ui-node="node.id"
    ><span aria-hidden="true" class="ui-badge-dot" />{{ display('text') }}</span
  >
  <hr v-else-if="node.kind === 'divider'" class="ui-divider" :data-ui-node="node.id" />
  <div
    v-else-if="node.kind === 'tabs'"
    class="ui-tabs"
    role="tablist"
    :aria-label="display('label')"
    :data-ui-node="node.id"
    @keydown="tabKey"
  >
    <button
      v-for="option in node.field?.options || []"
      :key="option.value"
      type="button"
      role="tab"
      :aria-selected="node.value === option.value"
      :tabindex="node.value === option.value ? 0 : -1"
      :disabled="disabled"
      @click="emit('input', node.id, option.value)"
    >
      {{ option.label }}
    </button>
  </div>
  <label v-else-if="node.kind === 'select'" class="ui-field ui-select" :data-ui-node="node.id"
    ><span class="ui-field-label">{{ display('label') }}</span
    ><select
      :id="domId"
      :aria-label="display('label')"
      :value="node.value"
      :disabled="disabled"
      @change="input($event)"
    >
      <option v-for="option in node.field?.options || []" :key="option.value" :value="option.value">
        {{ option.label }}
      </option></select
    ><small v-if="p.description">{{ display('description') }}</small></label
  >
  <label v-else-if="node.kind === 'checkbox' || node.kind === 'switch'" class="ui-check" :data-ui-node="node.id"
    ><input
      :id="domId"
      :aria-label="display('label')"
      type="checkbox"
      :role="node.kind === 'switch' ? 'switch' : undefined"
      :checked="node.value === true"
      :disabled="disabled"
      @change="emit('input', node.id, ($event.target as HTMLInputElement).checked)"
    /><span
      ><span class="ui-field-label">{{ display('label') }}</span
      ><small v-if="p.description">{{ display('description') }}</small></span
    ></label
  >
  <label v-else-if="node.kind === 'slider' || node.kind === 'number'" class="ui-field" :data-ui-node="node.id"
    ><span class="ui-field-top"
      ><span class="ui-field-label">{{ display('label') }}</span
      ><output v-if="node.kind === 'slider'" :for="domId"
        >{{ display('prefix') }}{{ uiDisplay(node.value) }}{{ display('suffix') }}</output
      ></span
    ><input
      :id="domId"
      :aria-label="display('label')"
      :type="node.kind === 'slider' ? 'range' : 'number'"
      :value="node.value"
      :min="node.field?.min ?? undefined"
      :max="node.field?.max ?? undefined"
      :step="node.field?.step ?? 1"
      :disabled="disabled"
      @input="node.kind === 'slider' && input($event, true)"
      @change="node.kind === 'number' && input($event, true)"
    /><small v-if="p.description">{{ display('description') }}</small></label
  >
  <label v-else-if="node.kind === 'input'" class="ui-field" :data-ui-node="node.id"
    ><span class="ui-field-label">{{ display('label') }}</span
    ><input
      :id="domId"
      :aria-label="display('label')"
      type="text"
      :value="node.value"
      :maxlength="node.field?.max_length ?? 1024"
      :placeholder="display('placeholder')"
      :disabled="disabled"
      @input="input($event)"
    /><small v-if="p.description">{{ display('description') }}</small></label
  >
  <button
    v-else-if="node.kind === 'button'"
    type="button"
    class="ui-button"
    :data-tone="p.tone || 'neutral'"
    :data-ui-node="node.id"
    :disabled="disabled"
    @click="emit('activate', node.id)"
  >
    <span>{{ display('label') }}</span>
  </button>
  <div
    v-else-if="node.kind === 'progress'"
    class="ui-progress"
    :data-tone="p.tone || 'primary'"
    :data-ui-node="node.id"
  >
    <div class="ui-field-top">
      <span class="ui-field-label">{{ display('label') }}</span
      ><output>{{ Math.round(progress) }}%</output>
    </div>
    <div
      class="ui-progress-track"
      role="progressbar"
      :aria-label="display('label')"
      :aria-valuenow="Number(p.value || 0)"
      :aria-valuemin="0"
      :aria-valuemax="Number(p.max ?? 100)"
    >
      <span :style="{ width: `${progress}%` }" />
    </div>
    <small v-if="p.detail">{{ display('detail') }}</small>
  </div>
  <div v-else-if="node.kind === 'chart'" class="ui-chart" :data-ui-node="node.id">
    <InlineChart v-if="chart.labels.length" :spec="chart" source="" embedded />
    <p v-else class="ui-empty">{{ t('inlineUi.noData') }}</p>
  </div>
  <section v-else-if="node.kind === 'table'" class="ui-table-section" :data-ui-node="node.id">
    <h4 v-if="p.title">{{ display('title') }}</h4>
    <div class="ui-table-scroll" tabindex="0" role="region" :aria-label="display('title') || t('inlineUi.table')">
      <table>
        <thead>
          <tr>
            <th v-for="column in columns" :key="column.key" scope="col">{{ column.label }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, index) in rows" :key="index">
            <td v-for="column in columns" :key="column.key">{{ uiDisplay(row[column.key]) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-if="!rows.length" class="ui-empty">{{ display('empty') || t('inlineUi.noData') }}</p>
  </section>
  <section v-else-if="node.kind === 'tasks'" class="ui-tasks" :data-ui-node="node.id">
    <h4 v-if="p.title">{{ display('title') }}</h4>
    <label v-for="item in items" :key="String(item.id)" class="ui-task" :data-checked="checked(item.id)">
      <input type="checkbox" :checked="checked(item.id)" :disabled="disabled" @change="toggleTask(item.id)" />
      <span>
        <strong>{{ uiDisplay(item.title) }}</strong>
        <small v-if="item.detail">{{ uiDisplay(item.detail) }}</small>
      </span>
    </label>
    <p v-if="!items.length" class="ui-empty">{{ display('empty') || t('inlineUi.noData') }}</p>
  </section>
  <section v-else-if="node.kind === 'timeline'" class="ui-timeline" :data-ui-node="node.id">
    <h4 v-if="p.title">{{ display('title') }}</h4>
    <ol>
      <li v-for="item in items" :key="String(item.id)">
        <span class="ui-timeline-dot" aria-hidden="true" />
        <div>
          <strong>{{ uiDisplay(item.title) }}</strong
          ><small v-if="item.detail">{{ uiDisplay(item.detail) }}</small>
        </div>
      </li>
    </ol>
  </section>
  <aside v-else-if="node.kind === 'callout'" class="ui-callout" :data-tone="p.tone || 'info'" :data-ui-node="node.id">
    <component :is="p.tone === 'warning' || p.tone === 'danger' ? RiAlertLine : RiInformationLine" aria-hidden="true" />
    <div>
      <strong v-if="p.title">{{ display('title') }}</strong>
      <p>{{ display('text') }}</p>
    </div>
  </aside>
  <details v-else-if="node.kind === 'details'" class="ui-details" :data-ui-node="node.id">
    <summary>{{ display('title') }}</summary>
    <div class="ui-stack">
      <ProgramNode
        v-for="child in node.children"
        :key="child.id"
        :node="child"
        :scope="scope"
        @input="(id, value) => emit('input', id, value)"
        @activate="(id) => emit('activate', id)"
      />
    </div>
  </details>
</template>
