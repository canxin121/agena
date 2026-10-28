<script setup lang="ts">
import { computed, ref, shallowRef, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import ProgramNode from './ProgramNode.vue'
import { activateUi, inputUi, reconcileUiState, renderUi, type UiProgram, type UiView } from './program'
import { programState, saveProgramState } from './programState'
import { copyTextToClipboard } from '@/lib/clipboard'
const props = defineProps<{ program: UiProgram; scope: string }>()
const { t } = useI18n()
const instance = useId()
// Two embeds of the same reference share state, not DOM identifiers.
const domScope = `panel-${instance}`
const scope = computed(() => props.scope || `local/${instance}`)
const state = computed(() => programState(scope.value, props.program))
const view = shallowRef<UiView | null>(null)
const error = ref(''),
  notice = ref('')
watch(
  () => props.program,
  () => {
    state.value.value = reconcileUiState(props.program, state.value.value)
  },
  { immediate: true },
)
watch(
  () => [props.program, state.value.value],
  () => {
    try {
      view.value = renderUi(props.program, state.value.value)
      error.value = ''
    } catch (reason) {
      error.value = reason instanceof Error ? reason.message : t('inlineUi.invalidInput')
    }
  },
  { immediate: true },
)
function resetInputs() {
  state.value.value = reconcileUiState(props.program)
  saveProgramState(scope.value, state.value.value)
}
function onInput(id: string, value: unknown) {
  try {
    const result = inputUi(props.program, state.value.value, id, value)
    state.value.value = result.state
    saveProgramState(scope.value, result.state)
    error.value = ''
    notice.value = ''
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : t('inlineUi.invalidInput')
  }
}
async function onActivate(id: string) {
  try {
    const result = activateUi(props.program, state.value.value, id)
    // Never perform effects during render/replay, only in this user event.
    if (result.copy !== null && !(await copyTextToClipboard(result.copy))) throw new Error(t('inlineUi.copyFailed'))
    state.value.value = result.state
    saveProgramState(scope.value, result.state)
    error.value = ''
    notice.value = result.copy === null ? '' : t('inlineUi.copied')
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : t('inlineUi.invalidInput')
  }
}
</script>
<template>
  <div class="interactive-panel" data-interactive-panel>
    <ProgramNode v-if="view" :node="view.root" :scope="domScope" @input="onInput" @activate="onActivate" />
    <p v-else class="ui-empty" role="status">{{ t('inlineUi.referenceUnavailable') }}</p>
    <div v-if="error" class="ui-runtime-error" role="alert">
      {{ error }}
      <button type="button" class="underline underline-offset-4" @click="resetInputs">
        {{ t('inlineUi.resetInputs') }}
      </button>
    </div>
    <p v-if="notice" class="ui-runtime-notice" role="status">{{ notice }}</p>
  </div>
</template>
<style>
/* Match Agena's native typography, density setting and control radii.
   Every descendant rule is scoped to the reply runtime, not the host UI. */
.interactive-panel {
  --ui-accent: var(--color-primary);
  --ui-surface: var(--color-card, var(--color-background));
  --ui-gap-sm: calc(0.25rem * var(--padding-scale, 1));
  --ui-gap-md: calc(0.5rem * var(--padding-scale, 1));
  --ui-gap-lg: calc(0.75rem * var(--padding-scale, 1));
  --ui-inset: calc(0.625rem * var(--padding-scale, 1));
  container: inline-ui / inline-size;
  min-width: 0;
  color: var(--color-foreground);
  font-size: var(--text-ui-label, 0.875rem);
  line-height: 1.45;
  overflow-wrap: anywhere;
}
:where(.interactive-panel) [data-tone='success'] {
  --ui-accent: var(--color-emerald-700, #047857);
}
:where(.interactive-panel) [data-tone='warning'] {
  --ui-accent: var(--color-amber-700, #b45309);
}
:where(.interactive-panel) [data-tone='danger'] {
  --ui-accent: var(--color-destructive);
}
:where(.interactive-panel) [data-tone='info'],
:where(.interactive-panel) [data-tone='primary'] {
  --ui-accent: var(--color-primary);
}
:where(.dark .interactive-panel) [data-tone='success'] {
  --ui-accent: var(--color-emerald-400, #34d399);
}
:where(.dark .interactive-panel) [data-tone='warning'] {
  --ui-accent: var(--color-amber-400, #fbbf24);
}
:where(.interactive-panel) h3,
:where(.interactive-panel) h4,
:where(.interactive-panel) p {
  margin: 0;
}
:where(.interactive-panel) h4 {
  font-size: var(--text-ui-label, 0.875rem);
  font-weight: 600;
  line-height: 1.4;
}
:where(.interactive-panel) .ui-layout {
  min-width: 0;
  gap: var(--ui-gap-md);
}
:where(.interactive-panel) .ui-stack {
  display: flex;
  flex-direction: column;
  gap: var(--ui-gap-md);
}
:where(.interactive-panel) .ui-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--ui-gap-md);
}
:where(.interactive-panel) .ui-row > .ui-field {
  flex: 0 1 12rem;
  margin-inline-start: auto;
  max-width: 100%;
}
:where(.interactive-panel) .ui-grid {
  display: grid;
  grid-template-columns: repeat(var(--ui-columns, 2), minmax(0, 1fr));
  align-items: start;
}
:where(.interactive-panel) .ui-layout[data-gap='sm'] {
  gap: var(--ui-gap-sm);
}
:where(.interactive-panel) .ui-layout[data-gap='lg'] {
  gap: var(--ui-gap-lg);
}
:where(.interactive-panel) .ui-card {
  container: inline-ui / inline-size;
  display: flex;
  flex-direction: column;
  gap: var(--ui-gap-md);
  padding: var(--ui-inset);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 0.5rem);
  background: var(--ui-surface);
}
:where(.interactive-panel) .ui-card > .ui-card {
  background: transparent;
}
:where(.interactive-panel) .ui-section-header {
  grid-column: 1 / -1;
  display: flex;
  flex-direction: column;
  gap: 0.125rem;
}
:where(.interactive-panel) .ui-row > .ui-section-header {
  flex-basis: 100%;
}
:where(.interactive-panel) .ui-section-header p {
  font-size: var(--text-micro, 0.8125rem);
  color: var(--color-muted-foreground);
}
:where(.interactive-panel) .ui-heading {
  font-size: var(--text-ui-header, 0.9375rem);
  font-weight: 600;
  line-height: 1.4;
  letter-spacing: -0.01em;
}
:where(.interactive-panel) .ui-heading[data-size='lg'] {
  font-size: calc(var(--text-ui-header, 0.9375rem) * 1.2);
}
:where(.interactive-panel) .ui-heading[data-size='sm'] {
  font-size: var(--text-ui-label, 0.875rem);
}
:where(.interactive-panel) .ui-text {
  white-space: pre-wrap;
}
:where(.interactive-panel) .ui-text[data-size='sm'],
:where(.interactive-panel) .ui-caption {
  font-size: var(--text-micro, 0.8125rem);
  color: var(--color-muted-foreground);
}
:where(.interactive-panel) .ui-metric {
  padding: var(--ui-inset);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 0.5rem);
  background: var(--ui-surface);
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  min-width: 0;
}
:where(.interactive-panel) .ui-metric-value {
  font-size: calc(var(--text-ui-header, 0.9375rem) * 1.6);
  font-weight: 600;
  letter-spacing: -0.025em;
  line-height: 1.2;
  font-variant-numeric: tabular-nums;
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.125rem;
  color: var(--ui-accent);
  min-width: 0;
}
:where(.interactive-panel) .ui-metric[data-tone='neutral'] .ui-metric-value {
  color: var(--color-foreground);
}
:where(.interactive-panel) .ui-unit {
  font-size: 0.65em;
  font-weight: 500;
}
:where(.interactive-panel) .ui-metric-detail {
  font-size: var(--text-micro, 0.8125rem);
  color: var(--color-muted-foreground);
  display: flex;
  gap: 0.375rem;
  align-items: center;
  flex-wrap: wrap;
}
:where(.interactive-panel) .ui-trend {
  display: inline-flex;
  align-items: center;
  gap: 0.125rem;
  color: var(--ui-accent);
}
:where(.interactive-panel) .ui-trend svg {
  width: 0.875rem;
  height: 0.875rem;
}
:where(.interactive-panel) .ui-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  align-self: flex-start;
  font-size: var(--text-micro, 0.8125rem);
  font-weight: 500;
  padding: 0.125rem 0.375rem;
  border-radius: var(--radius-sm, 0.375rem);
  border: 1px solid color-mix(in oklab, var(--ui-accent) 30%, var(--color-border));
  color: var(--ui-accent);
  background: color-mix(in oklab, var(--ui-accent) 6%, transparent);
  max-width: 100%;
}
:where(.interactive-panel) .ui-badge-dot {
  width: 0.3rem;
  height: 0.3rem;
  border-radius: 50%;
  background: currentColor;
  flex-shrink: 0;
}
:where(.interactive-panel) .ui-divider {
  border: 0;
  border-block-start: 1px solid var(--color-border);
  margin: 0;
}
:where(.interactive-panel) .ui-tabs {
  display: flex;
  gap: 0.125rem;
  align-self: flex-start;
  max-width: 100%;
  min-width: 0;
  padding: 0.125rem;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 0.375rem);
  background: color-mix(in oklab, var(--color-sidebar-accent, var(--color-muted)) 35%, transparent);
  overflow-x: auto;
  overscroll-behavior-inline: contain;
}
:where(.interactive-panel) .ui-tabs button {
  flex: 0 0 auto;
  white-space: nowrap;
  min-height: 2rem;
  padding: 0.25rem 0.625rem;
  font-size: var(--text-micro, 0.8125rem);
  font-weight: 500;
  line-height: 1.35;
  border-radius: calc(var(--radius-sm, 0.375rem) - 2px);
  color: var(--color-muted-foreground);
  transition:
    background 0.12s,
    color 0.12s;
}
:where(.interactive-panel) .ui-tabs button[aria-selected='true'] {
  background: var(--color-accent);
  color: var(--color-foreground);
}
:where(.interactive-panel) .ui-tabs button:hover:not(:disabled) {
  color: var(--color-foreground);
}
:where(.interactive-panel) .ui-field {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  min-width: 0;
}
:where(.interactive-panel) .ui-field-label {
  font-size: var(--text-ui-label, 0.875rem);
  font-weight: 500;
  display: block;
}
:where(.interactive-panel) .ui-field-top {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.25rem 0.5rem;
}
:where(.interactive-panel) .ui-field-top output {
  font-size: var(--text-ui-label, 0.875rem);
  font-variant-numeric: tabular-nums;
  font-weight: 600;
  color: var(--ui-accent);
}
:where(.interactive-panel) small {
  display: block;
  font-size: var(--text-micro, 0.8125rem);
  color: var(--color-muted-foreground);
  line-height: 1.4;
}
:where(.interactive-panel) .ui-field input:not([type='range']),
:where(.interactive-panel) .ui-field select {
  min-height: 2rem;
  width: 100%;
  min-width: 0;
  padding: 0.25rem 0.5rem;
  border: 1px solid var(--color-input);
  border-radius: var(--radius-md, 0.5rem);
  background: var(--color-background);
  color: var(--color-foreground);
  font-family: inherit;
  font-size: var(--text-ui-label, 0.875rem);
  line-height: 1.35;
}
:where(.interactive-panel) .ui-field select {
  padding-inline-end: 1.5rem;
}
:where(.interactive-panel) .ui-field input::placeholder {
  color: var(--color-muted-foreground);
}
:where(.interactive-panel) .ui-field input[type='range'] {
  width: 100%;
  min-height: 2rem;
  margin: 0;
  accent-color: var(--ui-accent);
  cursor: pointer;
}
:where(.interactive-panel) .ui-select {
  min-width: min(100%, 9rem);
}
:where(.interactive-panel) .ui-check {
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
  cursor: pointer;
  min-height: 2rem;
  padding-block: 0.25rem;
}
:where(.interactive-panel) .ui-check input,
:where(.interactive-panel) .ui-task input {
  margin: 0.125rem 0 0;
  width: 1rem;
  height: 1rem;
  accent-color: var(--ui-accent);
  flex-shrink: 0;
  cursor: pointer;
}
:where(.interactive-panel) .ui-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  align-self: flex-start;
  gap: 0.375rem;
  padding: 0.25rem 0.625rem;
  min-height: 2rem;
  max-width: 100%;
  border: 1px solid var(--color-input);
  border-radius: var(--radius-md, 0.5rem);
  background: var(--color-background);
  color: var(--color-foreground);
  font-family: inherit;
  font-size: var(--text-micro, 0.8125rem);
  font-weight: 500;
  line-height: 1.35;
  transition:
    background 0.12s,
    border-color 0.12s;
}
:where(.interactive-panel) .ui-button > span {
  min-width: 0;
}
:where(.interactive-panel) .ui-button:hover:not(:disabled) {
  background: var(--color-accent);
}
:where(.interactive-panel) .ui-button[data-tone='primary'] {
  background: var(--color-primary);
  color: var(--color-primary-foreground);
  border-color: var(--color-primary);
}
:where(.interactive-panel) .ui-button[data-tone='primary']:hover:not(:disabled) {
  background: color-mix(in oklab, var(--color-primary) 90%, var(--color-background));
}
:where(.interactive-panel) .ui-progress {
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}
:where(.interactive-panel) .ui-progress-track {
  height: 0.375rem;
  border-radius: 999px;
  background: var(--color-muted);
  overflow: hidden;
}
:where(.interactive-panel) .ui-progress-track > span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--ui-accent);
  transition: width 0.2s ease;
}
:where(.interactive-panel) .ui-chart {
  min-width: 0;
}
:where(.interactive-panel) .ui-table-section {
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
  min-width: 0;
}
:where(.interactive-panel) .ui-table-scroll {
  max-width: 100%;
  overflow: auto;
  max-height: 24rem;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 0.375rem);
  overscroll-behavior: contain;
}
:where(.interactive-panel) .ui-table-scroll table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--text-micro, 0.8125rem);
  text-align: start;
}
:where(.interactive-panel) .ui-table-scroll th,
:where(.interactive-panel) .ui-table-scroll td {
  text-align: start;
  padding: 0.375rem 0.5rem;
  min-width: 4rem;
  max-width: 28rem;
  vertical-align: top;
  white-space: pre-wrap;
  border-block-end: 1px solid var(--color-border);
}
:where(.interactive-panel) .ui-table-scroll th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--color-card, var(--color-background));
  font-weight: 600;
}
:where(.interactive-panel) .ui-table-scroll tbody tr:last-child td {
  border-block-end: 0;
}
:where(.interactive-panel) .ui-tasks {
  min-width: 0;
}
:where(.interactive-panel) .ui-tasks > h4 {
  margin-bottom: 0.25rem;
}
:where(.interactive-panel) .ui-task {
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
  padding: 0.375rem 0;
  min-height: 2rem;
  border-bottom: 1px solid var(--color-border);
  cursor: pointer;
}
:where(.interactive-panel) .ui-task:last-of-type {
  border-bottom: 0;
}
:where(.interactive-panel) .ui-task > span {
  flex: 1;
  min-width: 0;
}
:where(.interactive-panel) .ui-task strong {
  display: block;
  font-size: var(--text-ui-label, 0.875rem);
  font-weight: 500;
}
:where(.interactive-panel) .ui-task[data-checked='true'] strong {
  color: var(--color-muted-foreground);
  text-decoration: line-through;
}
:where(.interactive-panel) .ui-timeline > h4 {
  margin-bottom: 0.25rem;
}
:where(.interactive-panel) .ui-timeline ol {
  margin: 0;
  padding: 0;
  list-style: none;
}
:where(.interactive-panel) .ui-timeline li {
  display: grid;
  grid-template-columns: 0.5rem minmax(0, 1fr);
  gap: 0.5rem;
  position: relative;
  padding-block: 0.25rem;
}
:where(.interactive-panel) .ui-timeline li:not(:last-child)::before {
  content: '';
  position: absolute;
  inset-inline-start: 0.1875rem;
  top: 0.875rem;
  bottom: -0.625rem;
  width: 1px;
  background: var(--color-border);
}
:where(.interactive-panel) .ui-timeline-dot {
  width: 0.4375rem;
  height: 0.4375rem;
  border-radius: 50%;
  background: var(--ui-accent);
  margin-top: 0.375rem;
}
:where(.interactive-panel) .ui-timeline strong {
  font-size: var(--text-ui-label, 0.875rem);
  font-weight: 500;
}
:where(.interactive-panel) .ui-callout {
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
  padding: 0.5rem var(--ui-inset);
  border-inline-start: 2px solid var(--ui-accent);
  border-radius: 0 var(--radius-sm, 0.375rem) var(--radius-sm, 0.375rem) 0;
  background: color-mix(in oklab, var(--ui-accent) 5%, transparent);
}
:where(.interactive-panel) .ui-callout > svg {
  width: 1rem;
  height: 1rem;
  flex-shrink: 0;
  margin-top: 0.125rem;
  color: var(--ui-accent);
}
:where(.interactive-panel) .ui-callout > div {
  min-width: 0;
}
:where(.interactive-panel) .ui-callout strong {
  font-size: var(--text-ui-label, 0.875rem);
  font-weight: 600;
}
:where(.interactive-panel) .ui-callout p {
  font-size: var(--text-ui-label, 0.875rem);
  white-space: pre-wrap;
}
:where(.interactive-panel) .ui-callout strong + p {
  margin-top: 0.125rem;
}
:where(.interactive-panel) .ui-details {
  border-block: 1px solid var(--color-border);
}
:where(.interactive-panel) .ui-details summary {
  cursor: pointer;
  font-size: var(--text-ui-label, 0.875rem);
  font-weight: 500;
  padding-block: 0.375rem;
  min-height: 2rem;
}
:where(.interactive-panel) .ui-details[open] > div {
  padding-bottom: 0.5rem;
}
:where(.interactive-panel) .ui-empty {
  padding: 0.5rem;
  text-align: center;
  font-size: var(--text-micro, 0.8125rem);
  color: var(--color-muted-foreground);
}
:where(.interactive-panel) .ui-runtime-error {
  padding: 0.375rem 0.5rem;
  border-inline-start: 2px solid var(--color-destructive);
  background: color-mix(in oklab, var(--color-destructive) 6%, transparent);
  color: var(--color-destructive);
  font-size: var(--text-micro, 0.8125rem);
  margin-top: 0.5rem;
}
:where(.interactive-panel) .ui-runtime-notice {
  font-size: var(--text-micro, 0.8125rem);
  color: var(--color-muted-foreground);
  margin-top: 0.375rem;
}
:where(.interactive-panel) button:focus-visible,
:where(.interactive-panel) input:focus-visible,
:where(.interactive-panel) select:focus-visible,
:where(.interactive-panel) summary:focus-visible,
:where(.interactive-panel) .ui-table-scroll:focus-visible {
  outline: 2px solid var(--color-ring);
  outline-offset: 2px;
}
:where(.interactive-panel) .ui-tabs button:focus-visible {
  outline-offset: -2px;
}
:where(.interactive-panel) button:not(:disabled) {
  cursor: pointer;
}
:where(.interactive-panel) :disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
@container inline-ui (max-width: 36rem) {
  :where(.interactive-panel) .ui-grid[data-columns='3'],
  :where(.interactive-panel) .ui-grid[data-columns='4'] {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@container inline-ui (max-width: 28rem) {
  :where(.interactive-panel) .ui-grid {
    grid-template-columns: 1fr;
  }
  :where(.interactive-panel) .ui-grid[data-metrics='true']:not([data-columns='1']) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  :where(.interactive-panel) .ui-row > .ui-field {
    flex: 1 1 100%;
    margin-inline-start: 0;
  }
  :where(.interactive-panel) .ui-tabs button {
    padding-inline: 0.5rem;
  }
  :where(.interactive-panel) .ui-metric-value {
    font-size: calc(var(--text-ui-header, 0.9375rem) * 1.4);
  }
}
@container inline-ui (max-width: 18rem) {
  :where(.interactive-panel) .ui-grid[data-metrics='true'] {
    grid-template-columns: 1fr;
  }
}
@media (pointer: coarse) {
  :where(.interactive-panel) .ui-tabs button,
  :where(.interactive-panel) .ui-button,
  :where(.interactive-panel) .ui-field input:not([type='range']),
  :where(.interactive-panel) .ui-field input[type='range'],
  :where(.interactive-panel) .ui-field select,
  :where(.interactive-panel) .ui-task,
  :where(.interactive-panel) .ui-check,
  :where(.interactive-panel) .ui-details summary {
    min-height: 2.5rem;
  }
  :where(.interactive-panel) .ui-check input,
  :where(.interactive-panel) .ui-task input {
    min-height: 0;
  }
}
@media (prefers-reduced-motion: reduce) {
  :where(.interactive-panel) * {
    transition: none !important;
  }
}
</style>
