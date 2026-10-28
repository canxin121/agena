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
.interactive-panel {
  --ui-accent: var(--color-primary);
  --ui-surface: color-mix(in oklab, var(--color-muted) 30%, var(--color-background));
  container-type: inline-size;
  min-width: 0;
  color: var(--color-foreground);
  line-height: 1.6;
  font-size: 0.9rem;
  overflow-wrap: anywhere;
}
.interactive-panel [data-tone='success'] {
  --ui-accent: var(--color-chart-2, var(--color-primary));
}
.interactive-panel [data-tone='warning'] {
  --ui-accent: var(--color-chart-4, var(--color-primary));
}
.interactive-panel [data-tone='danger'] {
  --ui-accent: var(--color-destructive);
}
.interactive-panel [data-tone='info'],
.interactive-panel [data-tone='primary'] {
  --ui-accent: var(--color-primary);
}
.interactive-panel h3,
.interactive-panel h4,
.interactive-panel p {
  margin: 0;
}
.interactive-panel h4 {
  font-size: 0.95rem;
  font-weight: 600;
  letter-spacing: -0.01em;
}
.ui-layout {
  min-width: 0;
  gap: 1rem;
}
.ui-stack {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.ui-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.85rem;
}
.ui-grid {
  display: grid;
  grid-template-columns: repeat(var(--ui-columns, 2), minmax(0, 1fr));
  align-items: stretch;
}
.ui-layout[data-gap='sm'] {
  gap: 0.5rem;
}
.ui-layout[data-gap='lg'] {
  gap: 1.5rem;
}
.ui-card {
  display: flex;
  flex-direction: column;
  padding: 1.15rem;
  border: 1px solid var(--color-border);
  border-radius: 0.9rem;
  background: color-mix(in oklab, var(--color-card, var(--color-background)) 92%, var(--color-muted));
  gap: 1rem;
}
.ui-section-header {
  grid-column: 1/-1;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}
.ui-section-header p {
  font-size: 0.78rem;
  color: var(--color-muted-foreground);
}
.ui-heading {
  font-size: 1.18rem;
  font-weight: 650;
  letter-spacing: -0.025em;
  line-height: 1.4;
}
.ui-heading[data-size='lg'] {
  font-size: 1.65rem;
}
.ui-heading[data-size='sm'] {
  font-size: 1rem;
}
.ui-text {
  white-space: pre-wrap;
}
.ui-text[data-size='sm'] {
  font-size: 0.8rem;
  color: var(--color-muted-foreground);
}
.ui-caption {
  font-size: 0.78rem;
  color: var(--color-muted-foreground);
}
.ui-metric {
  padding: 1.15rem 1.2rem;
  border-radius: 0.85rem;
  background: var(--ui-surface);
  display: flex;
  flex-direction: column;
  gap: 0.45rem;
  min-width: 0;
  border: 1px solid color-mix(in oklab, var(--color-border) 40%, transparent);
}
.ui-metric-value {
  font-size: clamp(1.5rem, 4cqw, 2rem);
  font-weight: 650;
  letter-spacing: -0.04em;
  line-height: 1.2;
  font-variant-numeric: tabular-nums;
  display: flex;
  align-items: baseline;
  gap: 0.2rem;
  color: var(--ui-accent);
}
.ui-metric[data-tone='neutral'] .ui-metric-value {
  color: var(--color-foreground);
}
.ui-unit {
  font-size: 0.65em;
  font-weight: 500;
  opacity: 0.8;
}
.ui-metric-detail {
  font-size: 0.73rem;
  color: var(--color-muted-foreground);
  display: flex;
  gap: 0.5rem;
  align-items: center;
  flex-wrap: wrap;
}
.ui-trend {
  display: inline-flex;
  align-items: center;
  gap: 0.15rem;
  color: var(--ui-accent);
}
.ui-trend svg {
  width: 0.8rem;
  height: 0.8rem;
}
.ui-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  align-self: flex-start;
  font-size: 0.7rem;
  font-weight: 500;
  padding: 0.28rem 0.65rem;
  border-radius: 999px;
  background: color-mix(in oklab, var(--ui-accent) 8%, transparent);
  border: 1px solid color-mix(in oklab, var(--ui-accent) 18%, transparent);
  color: var(--ui-accent);
  max-width: 100%;
}
.ui-badge-dot {
  width: 0.35rem;
  height: 0.35rem;
  border-radius: 50%;
  background: currentColor;
  flex-shrink: 0;
}
.ui-divider {
  border: 0;
  border-block-start: 1px solid var(--color-border);
  margin: 0;
}
.ui-tabs {
  display: flex;
  gap: 0.2rem;
  background: var(--ui-surface);
  padding: 0.3rem;
  border: 1px solid var(--color-border);
  border-radius: 0.85rem;
  overflow: auto;
  max-width: 100%;
}
.ui-tabs button {
  flex: 1;
  white-space: nowrap;
  font-size: 0.82rem;
  font-weight: 500;
  border-radius: 0.6rem;
  padding: 0.6rem 0.9rem;
  color: var(--color-muted-foreground);
  transition:
    background 0.15s,
    color 0.15s;
}
.ui-tabs button[aria-selected='true'] {
  background: var(--color-background);
  color: var(--color-foreground);
  box-shadow: 0 1px 4px color-mix(in oklab, var(--color-foreground) 8%, transparent);
}
.ui-field {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  min-width: 0;
}
.ui-field-label {
  font-size: 0.83rem;
  font-weight: 500;
  display: block;
}
.ui-field-top {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.8rem;
}
.ui-field-top output {
  font-size: 0.82rem;
  font-variant-numeric: tabular-nums;
  font-weight: 600;
  color: var(--ui-accent);
}
.interactive-panel small {
  display: block;
  font-size: 0.75rem;
  color: var(--color-muted-foreground);
  line-height: 1.5;
}
.ui-field input:not([type='range']),
.ui-field select {
  min-height: 2.5rem;
  border: 1px solid var(--color-input, var(--color-border));
  border-radius: 0.6rem;
  background: var(--color-background);
  padding: 0.55rem 0.75rem;
  color: var(--color-foreground);
  font-size: 0.85rem;
  width: 100%;
  min-width: 0;
  outline: none;
}
.ui-field input[type='range'] {
  width: 100%;
  min-height: 1.75rem;
  accent-color: var(--ui-accent);
  cursor: pointer;
}
.ui-select {
  min-width: min(100%, 10rem);
}
.ui-check {
  display: flex;
  align-items: flex-start;
  gap: 0.6rem;
  cursor: pointer;
}
.ui-check input,
.ui-task input {
  margin-top: 0.25rem;
  width: 1rem;
  height: 1rem;
  accent-color: var(--ui-accent);
  flex-shrink: 0;
  cursor: pointer;
}
.ui-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  align-self: flex-start;
  gap: 0.65rem;
  padding: 0.55rem 0.85rem;
  border: 1px solid var(--color-border);
  border-radius: 0.6rem;
  background: var(--color-background);
  font-size: 0.8rem;
  font-weight: 500;
  transition:
    background 0.15s,
    border-color 0.15s;
  min-height: 2.25rem;
}
.ui-button svg {
  width: 1rem;
  height: 1rem;
  opacity: 0.65;
}
.ui-button:hover {
  background: var(--ui-surface);
  border-color: var(--ui-accent);
}
.ui-button[data-tone='primary'] {
  background: var(--color-primary);
  color: var(--color-primary-foreground);
  border-color: var(--color-primary);
}
.ui-progress {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
}
.ui-progress-track {
  height: 0.5rem;
  border-radius: 999px;
  background: var(--color-muted);
  overflow: hidden;
}
.ui-progress-track > span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--ui-accent);
  transition: width 0.25s ease;
}
.ui-chart {
  min-width: 0;
}
.ui-chart > section {
  border: 0;
  background: none;
  padding: 0;
}
.ui-table-section {
  display: flex;
  flex-direction: column;
  gap: 0.65rem;
  min-width: 0;
}
.ui-table-scroll {
  overflow: auto;
  max-height: 24rem;
  border: 1px solid var(--color-border);
  border-radius: 0.7rem;
  overscroll-behavior: contain;
}
.ui-table-scroll table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.8rem;
  text-align: start;
}
.ui-table-scroll th,
.ui-table-scroll td {
  padding: 0.75rem 0.85rem;
  text-align: start;
  border-block-end: 1px solid color-mix(in oklab, var(--color-border) 60%, transparent);
  min-width: 5.5rem;
  white-space: pre-wrap;
  vertical-align: top;
}
.ui-table-scroll th {
  font-size: 0.72rem;
  font-weight: 600;
  background: var(--ui-surface);
  position: sticky;
  top: 0;
  z-index: 1;
  color: var(--color-muted-foreground);
}
.ui-table-scroll tr:last-child td {
  border-bottom: 0;
}
.ui-table-scroll tbody tr:hover {
  background: color-mix(in oklab, var(--color-muted) 18%, transparent);
}
.ui-tasks {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
.ui-task {
  display: flex;
  align-items: flex-start;
  gap: 0.8rem;
  border: 1px solid var(--color-border);
  border-radius: 0.7rem;
  padding: 0.9rem 1rem;
  cursor: pointer;
  transition: background 0.15s;
}
.ui-task > span {
  flex: 1;
  min-width: 0;
}
.ui-task strong {
  font-size: 0.88rem;
  font-weight: 550;
  display: block;
  margin-bottom: 0.2rem;
}
.ui-task[data-checked='true'] {
  background: color-mix(in oklab, var(--color-primary) 5%, transparent);
  border-color: color-mix(in oklab, var(--color-primary) 25%, var(--color-border));
}
.ui-task-done {
  width: 1rem;
  height: 1rem;
  color: var(--color-primary);
  margin-top: 0.2rem;
  flex-shrink: 0;
}
.ui-timeline ol {
  list-style: none;
  margin: 0;
  padding: 0;
}
.ui-timeline li {
  position: relative;
  display: flex;
  gap: 0.75rem;
  padding: 0.25rem 0 0.9rem;
}
.ui-timeline li:not(:last-child)::after {
  content: '';
  position: absolute;
  left: 0.22rem;
  top: 0.8rem;
  bottom: 0.05rem;
  width: 1px;
  background: var(--color-border);
}
.ui-timeline-dot {
  margin-top: 0.45rem;
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: var(--color-muted-foreground);
  flex-shrink: 0;
}
.ui-timeline strong {
  font-size: 0.8rem;
  font-weight: 500;
}
.ui-timeline li:last-child {
  padding-bottom: 0;
}
.ui-timeline li:last-child .ui-timeline-dot {
  background: var(--color-primary);
}
.ui-callout {
  display: flex;
  align-items: flex-start;
  gap: 0.7rem;
  border: 1px solid color-mix(in oklab, var(--ui-accent) 22%, var(--color-border));
  background: color-mix(in oklab, var(--ui-accent) 5%, transparent);
  padding: 0.85rem 1rem;
  border-radius: 0.7rem;
}
.ui-callout > svg {
  width: 1.1rem;
  height: 1.1rem;
  flex-shrink: 0;
  margin-top: 0.2rem;
  color: var(--ui-accent);
}
.ui-callout strong {
  font-size: 0.82rem;
  font-weight: 600;
}
.ui-callout p {
  font-size: 0.8rem;
  margin-top: 0.2rem;
  white-space: pre-wrap;
  color: var(--color-muted-foreground);
}
.ui-details {
  border-block: 1px solid var(--color-border);
  padding: 0.6rem 0;
}
.ui-details summary {
  cursor: pointer;
  font-size: 0.85rem;
  font-weight: 500;
  padding: 0.2rem;
}
.ui-details[open] > div {
  margin-top: 0.8rem;
}
.ui-empty {
  padding: 1.2rem;
  text-align: center;
  font-size: 0.8rem;
  color: var(--color-muted-foreground);
}
.ui-runtime-error {
  padding: 0.5rem 0.75rem;
  border-radius: 0.5rem;
  background: color-mix(in oklab, var(--color-destructive) 8%, transparent);
  color: var(--color-destructive);
  font-size: 0.78rem;
  margin-top: 0.75rem !important;
}
.ui-runtime-notice {
  font-size: 0.75rem;
  color: var(--color-muted-foreground);
  margin-top: 0.5rem !important;
}
.interactive-panel button:focus-visible,
.interactive-panel input:focus-visible,
.interactive-panel select:focus-visible,
.interactive-panel summary:focus-visible,
.ui-table-scroll:focus-visible {
  outline: 2px solid var(--color-ring);
  outline-offset: 3px;
}
.interactive-panel :disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.interactive-panel button {
  cursor: pointer;
}
@container (max-width:600px) {
  .ui-grid {
    grid-template-columns: repeat(min(var(--ui-columns), 2), minmax(0, 1fr));
  }
  .ui-metric {
    padding: 0.85rem;
  }
  .ui-card {
    padding: 0.9rem;
  }
  .ui-heading[data-size='lg'] {
    font-size: 1.4rem;
  }
}
@container (max-width:400px) {
  .ui-grid {
    grid-template-columns: 1fr;
  }
  .ui-tabs button {
    padding: 0.55rem 0.6rem;
  }
  .ui-row {
    align-items: stretch;
  }
  .ui-select {
    width: 100%;
  }
  .ui-metric-value {
    font-size: 1.7rem;
  }
}
@media (prefers-reduced-motion: reduce) {
  .interactive-panel * {
    transition: none !important;
  }
}
</style>
