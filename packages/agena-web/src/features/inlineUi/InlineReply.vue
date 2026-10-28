<script setup lang="ts">
import InteractivePanel from './InteractivePanel.vue'
import { useI18n } from 'vue-i18n'
import {
  RiShieldCheckLine,
  RiStackLine,
  RiSparkling2Line,
  RiLinksLine,
  RiCheckboxCircleLine,
  RiSearchLine,
  RiCodeLine,
  RiInformationLine,
  RiLightbulbLine,
  RiAlertLine,
} from '@remixicon/vue'
import { defineAsyncComponent } from 'vue'
const InlineChart = defineAsyncComponent(() => import('./InlineChart.vue'))
import { inlineChartSpec, type InlineDocument } from './model'

defineProps<{ document: InlineDocument; scope?: string }>()
const { t } = useI18n()
const icons = {
  shield: RiShieldCheckLine,
  layers: RiStackLine,
  sparkles: RiSparkling2Line,
  link: RiLinksLine,
  check: RiCheckboxCircleLine,
  search: RiSearchLine,
  code: RiCodeLine,
}
const tones = { note: RiInformationLine, tip: RiLightbulbLine, warning: RiAlertLine }
</script>

<template>
  <article class="inline-reply" data-inline-reply :aria-label="document.title || t('inlineUi.label')">
    <h3 v-if="document.title" class="reply-title">{{ document.title }}</h3>
    <div class="reply-blocks">
      <section v-for="block in document.blocks" :key="block.id" class="reply-block" :data-inline-kind="block.type">
        <h4
          v-if="'title' in block && block.title && !['callout', 'details', 'chart'].includes(block.type)"
          class="block-title"
        >
          {{ block.title }}
        </h4>
        <InteractivePanel
          v-if="block.type === 'panel'"
          :program="block.program"
          :scope="scope ? `${scope}/${block.id}` : ''"
        />
        <p v-else-if="block.type === 'paragraph'" class="prose-text">{{ block.text }}</p>
        <aside v-else-if="block.type === 'callout'" class="reply-callout" :data-tone="block.tone || 'note'">
          <component :is="tones[block.tone || 'note']" class="reply-icon" aria-hidden="true" />
          <div class="min-w-0">
            <h4 v-if="block.title" class="callout-title">{{ block.title }}</h4>
            <p class="prose-text">{{ block.text }}</p>
          </div>
        </aside>
        <ol v-else-if="block.type === 'steps'" class="reply-steps">
          <li v-for="(item, index) in block.items" :key="index" class="reply-step">
            <span class="step-marker" aria-hidden="true">
              <component :is="icons[item.icon]" v-if="item.icon && item.icon !== 'number'" class="reply-icon" />
              <span v-else class="step-number">{{ index + 1 }}</span>
            </span>
            <div class="min-w-0">
              <h5 class="step-title">
                <span v-if="item.label" class="step-label">{{ item.label }}</span
                >{{ item.title }}
              </h5>
              <p class="prose-text step-text">{{ item.text }}</p>
            </div>
          </li>
        </ol>
        <div v-else-if="block.type === 'metrics'" class="reply-metrics">
          <dl v-for="(item, index) in block.items" :key="index" class="reply-metric">
            <dt>{{ item.label }}</dt>
            <dd class="metric-value">{{ item.value }}</dd>
            <dd v-if="item.detail" class="metric-detail">{{ item.detail }}</dd>
          </dl>
        </div>
        <div
          v-else-if="block.type === 'comparison'"
          class="table-scroll"
          tabindex="0"
          role="region"
          :aria-label="block.title || t('inlineUi.comparison')"
        >
          <table class="reply-table comparison-table">
            <thead>
              <tr>
                <th scope="col">
                  <span class="sr-only">{{ t('inlineUi.dimension') }}</span>
                </th>
                <th v-for="(column, index) in block.columns" :key="index" scope="col">{{ column }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(row, index) in block.rows" :key="index">
                <th scope="row">{{ row.label }}</th>
                <td v-for="(value, i) in row.values" :key="i">{{ value }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <div
          v-else-if="block.type === 'table'"
          class="table-scroll"
          tabindex="0"
          role="region"
          :aria-label="block.title || t('inlineUi.table')"
        >
          <table class="reply-table">
            <thead>
              <tr>
                <th v-for="(column, index) in block.columns" :key="index" scope="col">{{ column }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(row, index) in block.rows" :key="index">
                <td v-for="(value, i) in row" :key="i">{{ value }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <details v-else-if="block.type === 'details'" class="reply-details">
          <summary>{{ block.title }}</summary>
          <p class="prose-text">{{ block.text }}</p>
        </details>
        <InlineChart v-else-if="block.type === 'chart'" :spec="inlineChartSpec(block)" :source="''" />
      </section>
    </div>
  </article>
</template>

<style scoped>
.inline-reply {
  min-width: 0;
  max-width: 100%;
  color: var(--color-foreground);
  font-size: 0.9375rem;
  line-height: 1.75;
  overflow-wrap: anywhere;
  container-type: inline-size;
}
.reply-title {
  margin: 0 0 1.25rem;
  font-size: 1.125rem;
  font-weight: 650;
  letter-spacing: -0.015em;
  line-height: 1.5;
}
.reply-blocks {
  display: grid;
  gap: 1.5rem;
}
.reply-block {
  min-width: 0;
}
.block-title {
  margin: 0 0 0.8rem;
  font-size: 1rem;
  font-weight: 600;
  line-height: 1.6;
}
.prose-text {
  margin: 0;
  white-space: pre-wrap;
}
.reply-icon {
  height: 1.2rem;
  width: 1.2rem;
  flex-shrink: 0;
}
.reply-steps {
  margin: 0;
  padding: 0;
  list-style: none;
}
.reply-step {
  display: grid;
  grid-template-columns: 1.5rem minmax(0, 1fr);
  column-gap: 0.9rem;
  padding: 1.1rem 0;
  border-bottom: 1px solid color-mix(in oklab, var(--color-border) 65%, transparent);
}
.reply-step:first-child {
  padding-top: 0.15rem;
}
.reply-step:last-child {
  border-bottom: 0;
  padding-bottom: 0.15rem;
}
.step-marker {
  display: flex;
  justify-content: center;
  padding-top: 0.28rem;
  color: var(--color-muted-foreground);
}
.step-number {
  font-size: 0.8rem;
  font-variant-numeric: tabular-nums;
}
.step-title {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.55rem;
  margin: 0 0 0.35rem;
  font-size: 1rem;
  font-weight: 600;
  line-height: 1.6;
}
.step-label {
  font-size: 0.8rem;
  letter-spacing: 0.02em;
  font-weight: 500;
  color: var(--color-muted-foreground);
}
.step-text {
  color: color-mix(in oklab, var(--color-foreground) 85%, var(--color-muted-foreground));
}
.reply-callout {
  display: flex;
  gap: 0.75rem;
  padding: 0.85rem 1rem;
  border-inline-start: 3px solid var(--color-border);
  background: color-mix(in oklab, var(--color-muted) 28%, transparent);
  border-radius: 0 0.5rem 0.5rem 0;
}
.reply-callout > .reply-icon {
  margin-top: 0.28rem;
  color: var(--color-muted-foreground);
}
.reply-callout[data-tone='tip'] {
  border-inline-start-color: var(--color-primary);
}
.reply-callout[data-tone='warning'] {
  border-inline-start-color: var(--color-destructive);
}
.callout-title {
  margin: 0 0 0.3rem;
  font-size: 0.9375rem;
  font-weight: 600;
}
.reply-metrics {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 10rem), 1fr));
  gap: 1rem;
}
.reply-metric {
  margin: 0;
  padding: 0.15rem 0 0.15rem 1rem;
  border-inline-start: 2px solid var(--color-border);
  min-width: 0;
}
.reply-metric dt {
  font-size: 0.8125rem;
  color: var(--color-muted-foreground);
}
.reply-metric dd {
  margin: 0;
}
.metric-value {
  font-size: 1.5rem;
  font-weight: 600;
  letter-spacing: -0.025em;
  line-height: 1.55;
  font-variant-numeric: tabular-nums;
}
.metric-detail {
  font-size: 0.8rem;
  color: var(--color-muted-foreground);
}
.table-scroll {
  max-width: 100%;
  overflow-x: auto;
  overscroll-behavior-inline: contain;
  border-block: 1px solid var(--color-border);
  border-radius: 0.3rem;
}
.reply-table {
  width: 100%;
  border-collapse: collapse;
  text-align: start;
  font-size: 0.875rem;
}
.reply-table th,
.reply-table td {
  padding: 0.7rem 0.85rem;
  min-width: 6rem;
  max-width: 24rem;
  text-align: start;
  vertical-align: top;
  white-space: pre-wrap;
  border-bottom: 1px solid color-mix(in oklab, var(--color-border) 65%, transparent);
}
.reply-table thead th {
  background: color-mix(in oklab, var(--color-muted) 35%, transparent);
  font-weight: 600;
}
.reply-table tbody th {
  font-weight: 500;
  color: var(--color-muted-foreground);
}
.reply-table tbody tr:last-child > * {
  border-bottom: 0;
}
.comparison-table td {
  min-width: 10rem;
}
.reply-details {
  border-block: 1px solid color-mix(in oklab, var(--color-border) 65%, transparent);
  padding: 0.6rem 0;
}
.reply-details summary {
  cursor: pointer;
  font-size: 0.875rem;
  font-weight: 500;
  padding: 0.2rem;
}
.reply-details[open] summary {
  margin-bottom: 0.6rem;
}
.reply-details .prose-text {
  padding: 0 0.2rem 0.4rem;
  font-size: 0.875rem;
}
.table-scroll:focus-visible,
.reply-details summary:focus-visible {
  outline: 2px solid var(--color-ring);
  outline-offset: 3px;
}
@container (max-width: 420px) {
  .inline-reply {
    font-size: 0.875rem;
  }
  .reply-step {
    column-gap: 0.6rem;
  }
  .reply-callout {
    padding: 0.7rem;
  }
  .reply-table th,
  .reply-table td {
    padding: 0.6rem;
  }
}
@media (prefers-reduced-motion: reduce) {
  * {
    scroll-behavior: auto;
  }
}
</style>
