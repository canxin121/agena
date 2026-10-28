<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiTableLine, RiCodeLine, RiDownloadLine } from '@remixicon/vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import type { ChartSpec } from './charts'
const props = defineProps<{ spec: ChartSpec; source: string }>()
const { t } = useI18n()
const hidden = ref(new Set<number>())
const hover = ref<number | null>(null)
const showData = ref(false)
const showSource = ref(false)
const svg = ref<SVGSVGElement>()
const palette = ['#818cf8', '#34d399', '#fb923c', '#38bdf8', '#f472b6', '#a78bfa', '#facc15', '#2dd4bf']
const W = 800,
  H = 320,
  left = 72,
  right = 24,
  top = 22,
  bottom = 56
const plotWidth = W - left - right,
  plotHeight = H - top - bottom
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
const y = (value: number) => top + ((bounds.value.max - value) / (bounds.value.max - bounds.value.min)) * plotHeight
const x = (index: number) => left + ((index + 0.5) / props.spec.labels.length) * plotWidth
const format = (value: number | null) =>
  value === null
    ? '—'
    : new Intl.NumberFormat(undefined, {
        notation: Math.abs(value) >= 1e6 ? 'compact' : 'standard',
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
    angle += (value / total) * Math.PI * 2
    const end = angle,
      cx = 400,
      cy = 151,
      radius = 116
    const a = [cx + Math.cos(start) * radius, cy + Math.sin(start) * radius]
    const b = [
      cx + Math.cos(end - (end - start >= Math.PI * 2 ? 0.00001 : 0)) * radius,
      cy + Math.sin(end - (end - start >= Math.PI * 2 ? 0.00001 : 0)) * radius,
    ]
    return {
      index,
      value,
      percent: (value / total) * 100,
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
  const px = ((event.clientX - rect.left) / rect.width) * W
  hover.value = Math.max(
    0,
    Math.min(props.spec.labels.length - 1, Math.floor(((px - left) / plotWidth) * props.spec.labels.length)),
  )
}
function download() {
  if (!svg.value) return
  const copy = svg.value.cloneNode(true) as SVGSVGElement
  copy.setAttribute('xmlns', 'http://www.w3.org/2000/svg')
  copy.style.color = getComputedStyle(svg.value).color
  const url = URL.createObjectURL(new Blob([new XMLSerializer().serializeToString(copy)], { type: 'image/svg+xml' }))
  const link = document.createElement('a')
  link.href = url
  link.download = 'inline-ui-chart.svg'
  link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
</script>
<template>
  <figure class="my-4 overflow-hidden rounded-xl border border-border/70 bg-muted/10" data-reply-chart>
    <figcaption class="flex items-center gap-2 px-4 pt-3">
      <span class="min-w-0 flex-1 text-sm font-semibold">{{ spec.title || t('interactive.chart') }}</span>
      <button
        class="chart-action"
        :aria-label="t('interactive.data')"
        :title="t('interactive.data')"
        :aria-pressed="showData"
        @click="showData = !showData"
      >
        <RiTableLine class="h-4 w-4" />
      </button>
      <button
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
      class="block w-full text-foreground"
      role="img"
      :aria-label="spec.title || t('interactive.chart')"
      @pointermove="pointer"
      @pointerleave="hover = null"
    >
      <title>{{ spec.title || t('interactive.chart') }}</title>
      <template v-if="spec.type !== 'pie'">
        <g v-for="tick in ticks" :key="tick">
          <line :x1="left" :x2="W - right" :y1="y(tick)" :y2="y(tick)" stroke="currentColor" opacity=".1" />
          <text :x="left - 10" :y="y(tick) + 4" text-anchor="end" font-size="11" fill="currentColor" opacity=".6">
            {{ format(tick) }}
          </text>
        </g>
        <line :x1="left" :x2="W - right" :y1="y(0)" :y2="y(0)" stroke="currentColor" opacity=".25" />
        <text
          v-for="i in labelIndexes"
          :key="i"
          :x="x(i)"
          :y="H - bottom + 23"
          text-anchor="middle"
          font-size="11"
          fill="currentColor"
          opacity=".6"
        >
          {{ (spec.labels[i] || '').slice(0, 18) }}
        </text>
        <text
          v-if="spec.xLabel"
          :x="W / 2"
          :y="H - 6"
          text-anchor="middle"
          font-size="11"
          fill="currentColor"
          opacity=".65"
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
          opacity=".65"
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
        <circle cx="400" cy="151" r="64" fill="var(--color-background, #18181b)" />
      </template>
    </svg>
    <div class="flex min-h-8 flex-wrap items-center justify-center gap-x-4 gap-y-2 px-4 pb-3 text-xs">
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
        class="inline-flex items-center gap-1.5 rounded px-1 focus-visible:outline-2 focus-visible:outline-ring"
        :class="hidden.has(i) ? 'opacity-35' : ''"
        :aria-pressed="!hidden.has(i)"
        @click="toggle(i)"
      >
        <span class="h-2 w-2 rounded-full" :style="{ background: palette[i] }" />{{ series.name
        }}<strong v-if="hover !== null" class="font-mono">{{ format(series.values[hover] ?? null) }}</strong>
      </button>
      <span v-if="hover !== null" class="text-muted-foreground">{{ spec.labels[hover] }}</span>
    </div>
    <div v-if="showData" class="max-h-64 overflow-auto border-t border-border/60">
      <table class="w-full text-left text-xs">
        <caption class="sr-only">
          {{
            spec.title || t('interactive.data')
          }}
        </caption>
        <thead class="sticky top-0 bg-background">
          <tr>
            <th class="px-4 py-2">{{ spec.xLabel || t('interactive.label') }}</th>
            <th v-for="(series, i) in spec.series" :key="i" class="px-4 py-2">{{ series.name }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(label, index) in spec.labels" :key="index" class="border-t border-border/40">
            <th class="px-4 py-1.5 font-normal">{{ label }}</th>
            <td v-for="(series, i) in spec.series" :key="i" class="px-4 py-1.5 font-mono">
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
.chart-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: 5px;
  color: var(--color-muted-foreground);
}
.chart-action:hover {
  background: var(--color-muted);
  color: var(--color-foreground);
}
.chart-action:focus-visible {
  outline: 2px solid var(--color-ring);
}
</style>
