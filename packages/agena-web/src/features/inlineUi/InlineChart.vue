<script setup lang="ts">
import { computed, ref } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { useI18n } from 'vue-i18n'
import { RiTableLine, RiCodeLine, RiDownloadLine } from '@remixicon/vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import type { ChartSpec } from './charts'
const props = withDefaults(defineProps<{ spec: ChartSpec; source: string; embedded?: boolean }>(), { embedded: false })
const { t } = useI18n()
const hidden = ref(new Set<number>())
const hover = ref<number | null>(null)
const showData = ref(false)
const showSource = ref(false)
const svg = ref<SVGSVGElement>()
const container = ref<HTMLElement>()
const availableWidth = ref(640),
  axisFont = ref(13)
useResizeObserver(container, (entries) => {
  const entry = entries[0]
  if (!entry || entry.contentRect.width <= 0) return
  availableWidth.value = Math.max(160, Math.round(entry.contentRect.width))
  if (container.value) axisFont.value = Number.parseFloat(getComputedStyle(container.value).fontSize) || 13
})
// SVG coordinates follow the actual component width. Fixed 800px coordinates
// made mobile labels illegibly small and wasted vertical space on wide cards.
const palette = Array.from({ length: 8 }, (_, i) => `var(--inline-chart-${i + 1})`)
const W = computed(() => (props.spec.type === 'pie' ? Math.min(320, availableWidth.value) : availableWidth.value))
const H = computed(() =>
  props.spec.type === 'pie'
    ? Math.round(Math.max(160, Math.min(224, W.value * 0.72)))
    : Math.round(Math.max(180, Math.min(260, W.value * 0.32))),
)
const right = 12,
  top = 16
const left = computed(() =>
  Math.min(
    W.value * 0.42,
    Math.max(
      40,
      Math.max(...ticks.value.map((tick) => format(tick).length)) * axisFont.value * 0.6 +
        (props.spec.yLabel ? 28 : 12),
    ),
  ),
)
const bottom = computed(() => (props.spec.xLabel ? axisFont.value * 3.4 : axisFont.value * 2))
const plotWidth = computed(() => W.value - left.value - right)
const plotHeight = computed(() => H.value - top - bottom.value)
const pieCenter = computed(() => ({
  x: W.value / 2,
  y: H.value / 2,
  radius: Math.max(24, Math.min(W.value / 2 - 12, H.value / 2 - 8)),
}))
const visible = computed(() =>
  props.spec.series.map((s, index) => ({ ...s, index })).filter((s) => !hidden.value.has(s.index)),
)
const bounds = computed(() => {
  const values = visible.value.flatMap((s) => s.values.filter((v): v is number => v !== null))
  let min = Math.min(0, ...values),
    max = Math.max(0, ...values)
  if (min === max) {
    min = -1
    max = 1
  }
  return { min, max }
})
const y = (value: number) =>
  top + ((bounds.value.max - value) / (bounds.value.max - bounds.value.min)) * plotHeight.value
const x = (index: number) => left.value + ((index + 0.5) / props.spec.labels.length) * plotWidth.value
const format = (value: number | null) =>
  value === null
    ? '—'
    : new Intl.NumberFormat(undefined, {
        notation: Math.abs(value) >= 1e12 ? 'scientific' : Math.abs(value) >= 1e6 ? 'compact' : 'standard',
        maximumFractionDigits: 3,
      }).format(value)
const ticks = computed(() =>
  Array.from({ length: 5 }, (_, i) => bounds.value.min + ((bounds.value.max - bounds.value.min) * i) / 4),
)
const labelIndexes = computed(() => [
  ...new Set([0, Math.floor((props.spec.labels.length - 1) / 2), props.spec.labels.length - 1]),
])
function paths(values: (number | null)[], area = false): string[] {
  const result: string[] = []
  let points: [number, number][] = []
  const flush = () => {
    if (!points.length) return
    let d = points.map(([px, py], i) => `${i ? 'L' : 'M'}${px},${py}`).join(' ')
    if (area) d += ` L${points[points.length - 1]![0]},${y(0)} L${points[0]![0]},${y(0)} Z`
    result.push(d)
    points = []
  }
  values.forEach((value, i) => {
    if (value === null) flush()
    else points.push([x(i), y(value)])
  })
  flush()
  return result
}
const pie = computed(() => {
  const values = props.spec.series[0]?.values || []
  const total = values.reduce<number>((sum, v) => sum + (v || 0), 0)
  let angle = -Math.PI / 2
  return values.map((v, index) => {
    const value = v || 0,
      start = angle
    angle += (total > 0 ? value / total : 0) * Math.PI * 2
    const end = angle,
      cx = pieCenter.value.x,
      cy = pieCenter.value.y,
      radius = pieCenter.value.radius
    const a = [cx + Math.cos(start) * radius, cy + Math.sin(start) * radius]
    const b = [
      cx + Math.cos(end - (end - start >= Math.PI * 2 ? 0.00001 : 0)) * radius,
      cy + Math.sin(end - (end - start >= Math.PI * 2 ? 0.00001 : 0)) * radius,
    ]
    return {
      index,
      value,
      percent: total > 0 ? (value / total) * 100 : 0,
      path: `M${cx},${cy} L${a.join(',')} A${radius},${radius} 0 ${end - start > Math.PI ? 1 : 0} 1 ${b.join(',')} Z`,
    }
  })
})
function toggle(index: number) {
  const next = new Set(hidden.value)
  next.has(index) ? next.delete(index) : next.add(index)
  hidden.value = next
}
function pointer(event: PointerEvent) {
  if (props.spec.type === 'pie') return
  const rect = svg.value?.getBoundingClientRect()
  if (!rect) return
  const px = ((event.clientX - rect.left) / rect.width) * W.value
  hover.value = Math.max(
    0,
    Math.min(
      props.spec.labels.length - 1,
      Math.floor(((px - left.value) / plotWidth.value) * props.spec.labels.length),
    ),
  )
}
function download() {
  if (!svg.value) return
  const copy = svg.value.cloneNode(true) as SVGSVGElement
  copy.setAttribute('xmlns', 'http://www.w3.org/2000/svg')
  copy.setAttribute('width', String(W.value))
  copy.setAttribute('height', String(H.value))
  // Embed resolved colors/fonts: downloaded SVGs do not inherit app CSS vars.
  const originals = [svg.value, ...svg.value.querySelectorAll<SVGElement>('*')]
  const clones = [copy, ...copy.querySelectorAll<SVGElement>('*')]
  for (const [index, original] of originals.entries()) {
    const clone = clones[index]!,
      style = getComputedStyle(original)
    for (const property of ['fill', 'stroke', 'color', 'font-family', 'font-size'])
      clone.style.setProperty(property, style.getPropertyValue(property))
  }
  const url = URL.createObjectURL(new Blob([new XMLSerializer().serializeToString(copy)], { type: 'image/svg+xml' }))
  const link = document.createElement('a')
  link.href = url
  link.download = 'inline-ui-chart.svg'
  link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
</script>
<template>
  <figure
    ref="container"
    class="inline-chart"
    :data-embedded="embedded ? 'true' : undefined"
    :data-chart-type="spec.type"
    data-reply-chart
  >
    <figcaption class="chart-heading">
      <span class="chart-title">{{ spec.title || t('interactive.chart') }}</span>
      <button
        type="button"
        class="chart-action"
        :aria-label="t('interactive.data')"
        :title="t('interactive.data')"
        :aria-pressed="showData"
        @click="showData = !showData"
      >
        <RiTableLine class="h-4 w-4" />
      </button>
      <button
        type="button"
        class="chart-action"
        v-if="source"
        :aria-label="t('interactive.source')"
        :title="t('interactive.source')"
        :aria-pressed="showSource"
        @click="showSource = !showSource"
      >
        <RiCodeLine class="h-4 w-4" />
      </button>
      <button
        type="button"
        class="chart-action"
        :aria-label="t('interactive.download')"
        :title="t('interactive.download')"
        @click="download"
      >
        <RiDownloadLine class="h-4 w-4" />
      </button>
    </figcaption>
    <svg
      ref="svg"
      :viewBox="`0 0 ${W} ${H}`"
      class="chart-plot"
      :style="{ aspectRatio: `${W} / ${H}`, maxWidth: spec.type === 'pie' ? '320px' : undefined }"
      role="img"
      :aria-label="spec.title || t('interactive.chart')"
      @pointermove="pointer"
      @pointerleave="hover = null"
    >
      <title>{{ spec.title || t('interactive.chart') }}</title>
      <template v-if="spec.type !== 'pie'">
        <g v-for="tick in ticks" :key="tick">
          <line :x1="left" :x2="W - right" :y1="y(tick)" :y2="y(tick)" stroke="currentColor" opacity=".1" />
          <text :x="left - 10" :y="y(tick) + 4" text-anchor="end" font-size="11" fill="currentColor" opacity=".8">
            {{ format(tick) }}
          </text>
        </g>
        <line :x1="left" :x2="W - right" :y1="y(0)" :y2="y(0)" stroke="currentColor" opacity=".25" />
        <text
          v-for="i in labelIndexes"
          :key="i"
          :x="x(i)"
          :y="H - bottom + 23"
          :text-anchor="i === 0 ? 'start' : i === spec.labels.length - 1 ? 'end' : 'middle'"
          font-size="11"
          fill="currentColor"
          opacity=".8"
        >
          <title>{{ spec.labels[i] }}</title>
          {{
            Array.from(spec.labels[i] || '')
              .slice(0, Math.max(3, Math.floor(plotWidth / (labelIndexes.length * axisFont))))
              .join('')
          }}
        </text>
        <text
          v-if="spec.xLabel"
          :x="W / 2"
          :y="H - 6"
          text-anchor="middle"
          font-size="11"
          fill="currentColor"
          opacity=".8"
        >
          {{ spec.xLabel }}
        </text>
        <text
          v-if="spec.yLabel"
          x="16"
          :y="H / 2"
          :transform="`rotate(-90 16 ${H / 2})`"
          text-anchor="middle"
          font-size="11"
          fill="currentColor"
          opacity=".8"
        >
          {{ spec.yLabel }}
        </text>
        <g
          v-for="(series, si) in visible"
          :key="series.index"
          :fill="palette[series.index]"
          :stroke="palette[series.index]"
        >
          <template v-if="spec.type === 'bar'">
            <template v-for="(value, i) in series.values" :key="i">
              <rect
                v-if="value !== null"
                :x="
                  x(i) -
                  (plotWidth / spec.labels.length) * 0.4 +
                  (((si * plotWidth) / spec.labels.length) * 0.8) / visible.length
                "
                :width="Math.max(0.5, ((plotWidth / spec.labels.length) * 0.8) / visible.length - 2)"
                :y="Math.min(y(0), y(value))"
                :height="Math.max(0.7, Math.abs(y(value) - y(0)))"
                rx="2"
                stroke="none"
                opacity=".9"
              >
                <title>{{ spec.labels[i] }} · {{ series.name }}: {{ format(value) }}</title>
              </rect>
            </template>
          </template>
          <template v-else>
            <path
              v-for="(path, i) in paths(series.values, spec.type === 'area')"
              v-if="spec.type !== 'scatter'"
              :key="i"
              :d="path"
              :fill="spec.type === 'area' ? palette[series.index] : 'none'"
              :fill-opacity="spec.type === 'area' ? 0.15 : 1"
              stroke-width="2"
              stroke-linejoin="round"
            />
            <template v-for="(value, i) in series.values" :key="i">
              <circle
                v-if="value !== null && (spec.type === 'scatter' || series.values.length <= 30 || i === hover)"
                :cx="x(i)"
                :cy="y(value)"
                :r="i === hover ? 4 : 2.5"
                stroke-width="1"
              >
                <title>{{ spec.labels[i] }} · {{ series.name }}: {{ format(value) }}</title>
              </circle>
            </template>
          </template>
        </g>
        <line
          v-if="hover !== null"
          :x1="x(hover)"
          :x2="x(hover)"
          :y1="top"
          :y2="H - bottom"
          stroke="currentColor"
          stroke-dasharray="3 4"
          opacity=".35"
        />
      </template>
      <template v-else>
        <path
          v-for="slice in pie.filter((s) => s.value > 0)"
          :key="slice.index"
          :d="slice.path"
          :fill="palette[slice.index % palette.length]"
          stroke="currentColor"
          stroke-opacity=".1"
          stroke-width="2"
        >
          <title>{{ spec.labels[slice.index] }}: {{ format(slice.value) }} ({{ slice.percent.toFixed(1) }}%)</title>
        </path>
        <circle
          :cx="pieCenter.x"
          :cy="pieCenter.y"
          :r="pieCenter.radius * 0.56"
          fill="var(--color-card, var(--color-background))"
        />
      </template>
    </svg>
    <div class="chart-legend">
      <template v-if="spec.type === 'pie'"
        ><span v-for="slice in pie" :key="slice.index" class="inline-flex items-center gap-1.5"
          ><span class="h-2 w-2 rounded-full" :style="{ background: palette[slice.index % palette.length] }" />{{
            spec.labels[slice.index]
          }}
          <span class="text-muted-foreground">{{ slice.percent.toFixed(1) }}%</span></span
        ></template
      >
      <button
        v-for="(series, i) in spec.type === 'pie' ? [] : spec.series"
        :key="i"
        type="button"
        class="chart-legend-button"
        :class="hidden.has(i) ? 'opacity-35' : ''"
        :aria-pressed="!hidden.has(i)"
        @click="toggle(i)"
      >
        <span class="h-2 w-2 rounded-full" :style="{ background: palette[i] }" />{{ series.name
        }}<strong v-if="hover !== null" class="font-mono">{{ format(series.values[hover] ?? null) }}</strong>
      </button>
      <span v-if="hover !== null" class="text-muted-foreground">{{ spec.labels[hover] }}</span>
    </div>
    <div v-if="showData" class="chart-data" tabindex="0" role="region" :aria-label="t('interactive.data')">
      <table class="chart-data-table">
        <caption class="sr-only">
          {{
            spec.title || t('interactive.data')
          }}
        </caption>
        <thead class="sticky top-0 bg-background">
          <tr>
            <th class="chart-cell">{{ spec.xLabel || t('interactive.label') }}</th>
            <th v-for="(series, i) in spec.series" :key="i" class="chart-cell">{{ series.name }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(label, index) in spec.labels" :key="index" class="border-t border-border/40">
            <th class="chart-cell font-normal">{{ label }}</th>
            <td v-for="(series, i) in spec.series" :key="i" class="chart-cell font-mono">
              {{ format(series.values[index] ?? null) }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <CodeBlock v-if="showSource" :code="source" lang="json" />
  </figure>
</template>
<style scoped>
.inline-chart {
  --inline-chart-1: var(--color-primary);
  --inline-chart-2: #0f766e;
  --inline-chart-3: #6366f1;
  --inline-chart-4: #0284c7;
  --inline-chart-5: #be185d;
  --inline-chart-6: #7c3aed;
  --inline-chart-7: #a16207;
  --inline-chart-8: #15803d;
  margin: 0;
  min-width: 0;
  max-width: 100%;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 0.5rem);
  background: var(--color-card, var(--color-background));
  color: var(--color-foreground);
  font-size: var(--text-micro, 0.8125rem);
  line-height: 1.4;
}
:global(.dark) .inline-chart {
  --inline-chart-2: #2dd4bf;
  --inline-chart-3: #a5b4fc;
  --inline-chart-4: #38bdf8;
  --inline-chart-5: #f472b6;
  --inline-chart-6: #c4b5fd;
  --inline-chart-7: #facc15;
  --inline-chart-8: #4ade80;
}
.inline-chart[data-embedded='true'] {
  border: 0;
  background: transparent;
  border-radius: 0;
}
.chart-heading {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0.375rem 0.5rem 0;
}
.inline-chart[data-embedded='true'] .chart-heading {
  padding: 0;
}
.chart-title {
  flex: 1;
  min-width: 0;
  font-size: var(--text-ui-label, 0.875rem);
  font-weight: 600;
  overflow-wrap: anywhere;
}
.chart-plot {
  display: block;
  width: 100%;
  height: auto;
  margin-inline: auto;
}
.chart-plot text {
  font-family: inherit;
  font-size: var(--text-micro, 0.8125rem);
}
.chart-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border-radius: var(--radius-sm, 0.375rem);
  color: var(--color-muted-foreground);
  cursor: pointer;
}
.chart-action:hover {
  background: var(--color-accent);
  color: var(--color-foreground);
}
.chart-legend {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 0.25rem 0.75rem;
  padding: 0.125rem 0.5rem 0.375rem;
  min-width: 0;
}
.chart-legend > span {
  min-width: 0;
  overflow-wrap: anywhere;
}
.chart-legend-button {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  padding: 0.125rem 0.25rem;
  min-height: 1.75rem;
  max-width: 100%;
  border-radius: var(--radius-sm, 0.375rem);
  overflow-wrap: anywhere;
  cursor: pointer;
}
.chart-data {
  max-height: 16rem;
  overflow: auto;
  border-top: 1px solid var(--color-border);
  overscroll-behavior: contain;
}
.chart-data-table {
  width: 100%;
  border-collapse: collapse;
  text-align: start;
  font-size: inherit;
}
.chart-cell {
  text-align: start;
  padding: 0.375rem 0.5rem;
  overflow-wrap: anywhere;
}
.chart-action:focus-visible,
.chart-legend-button:focus-visible,
.chart-data:focus-visible {
  outline: 2px solid var(--color-ring);
  outline-offset: -2px;
}
@media (pointer: coarse) {
  .chart-action {
    width: 36px;
    height: 36px;
  }
  .chart-legend-button {
    min-height: 36px;
  }
}
</style>
