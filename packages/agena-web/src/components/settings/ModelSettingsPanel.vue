<script setup lang="ts">
import { computed, ref } from 'vue'
import { RiCpuLine, RiRefreshLine } from '@remixicon/vue'
import ModelSelectionSettings from './ModelSelectionSettings.vue'
import SettingsCard from './SettingsCard.vue'
import IconButton from '@/components/ui/IconButton.vue'

defineProps<{ scope: string; sessionId?: string }>()

type ModelEditor = {
  refresh: () => Promise<void>
  dirty: boolean
  loading: boolean
  saving: boolean
  catalogError: string
}

const defaultEditor = ref<ModelEditor | null>(null)
const approvalEditor = ref<ModelEditor | null>(null)
const auxiliaryEditor = ref<ModelEditor | null>(null)
const editors = computed(() => [defaultEditor.value, approvalEditor.value, auxiliaryEditor.value])
const dirty = computed(() => editors.value.some((editor) => editor?.dirty))
const loading = computed(() => editors.value.some((editor) => editor?.loading))
const saving = computed(() => editors.value.some((editor) => editor?.saving))
const catalogError = computed(() => editors.value.find((editor) => editor?.catalogError)?.catalogError)

async function refresh() {
  await Promise.all(editors.value.map((editor) => editor?.refresh()))
}
defineExpose({ refresh, dirty, saving })
</script>

<template>
  <SettingsCard :title="$st('Models')" :icon="RiCpuLine" data-model-settings>
    <template #actions>
      <IconButton
        size="sm"
        :loading="loading"
        :disabled="saving"
        :tooltip="$st('Refresh model settings')"
        :aria-label="$st('Refresh model settings')"
        @click="refresh"
        ><RiRefreshLine class="h-4 w-4" aria-hidden="true"
      /></IconButton>
    </template>
    <p v-if="catalogError" class="mb-2 break-words text-xs text-destructive" role="alert">{{ catalogError }}</p>
    <table class="model-settings-table" role="table" :aria-label="$st('Models')">
      <colgroup>
        <col class="model-setting-name-column" />
        <col />
        <col class="model-setting-preference-column" span="3" />
      </colgroup>
      <thead role="rowgroup">
        <tr role="row">
          <th scope="col" role="columnheader">{{ $st('Model role') }}</th>
          <th scope="col" role="columnheader">{{ $st('Model route') }}</th>
          <th scope="col" role="columnheader">{{ $st('Thinking') }}</th>
          <th scope="col" role="columnheader">{{ $st('Speed') }}</th>
          <th scope="col" role="columnheader">{{ $st('Verbosity') }}</th>
        </tr>
      </thead>
      <ModelSelectionSettings ref="defaultEditor" purpose="default" :scope="scope" :session-id="sessionId" />
      <template v-if="scope !== 'session'">
        <ModelSelectionSettings id="settings-approval-model" ref="approvalEditor" purpose="approval" :scope="scope" />
        <ModelSelectionSettings ref="auxiliaryEditor" purpose="auxiliary" :scope="scope" />
      </template>
    </table>
  </SettingsCard>
</template>

<style scoped>
@reference '../../style.css';

.model-settings-table {
  width: 100%;
  table-layout: fixed;
  border-collapse: collapse;
  font-size: 0.8125rem;
}
.model-setting-name-column {
  width: 17%;
}
.model-setting-preference-column {
  width: 14%;
}
.model-settings-table thead {
  @apply bg-muted/30 text-muted-foreground;
  font-size: 0.75rem;
}
.model-settings-table :deep(:is(th, td)) {
  padding: 0.4375rem 0.5rem;
  text-align: start;
  vertical-align: top;
  overflow-wrap: anywhere;
}
.model-settings-table thead th {
  font-weight: 550;
}
.model-settings-table :deep(.model-setting-role + .model-setting-role) {
  @apply border-t border-border/60;
}
.model-settings-table :deep(.model-setting-name) {
  padding-block-start: 0.875rem;
  font-weight: 600;
}
.model-settings-table :deep(.model-setting-route-controls) {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.25rem;
  min-width: 0;
}
.model-settings-table :deep(.model-setting-route-picker) {
  flex: 1 1 8rem;
}
.model-settings-table :deep(.model-setting-model-id) {
  @apply text-muted-foreground;
  display: block;
  margin-top: 0.1875rem;
  font-size: 0.6875rem;
  line-height: 1.25;
  text-align: start;
  overflow-wrap: anywhere;
}
.model-settings-table :deep(.model-setting-preference-controls) {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 0.25rem;
}
.model-settings-table :deep(.model-setting-preference [data-setting-source-choice]) {
  min-width: 0;
  padding-inline: 0.25rem;
  white-space: normal;
  font-size: 0.6875rem;
  overflow-wrap: anywhere;
}
.model-settings-table :deep(.model-setting-picker button[aria-haspopup='dialog']) {
  height: auto;
  min-height: 2rem;
  padding: 0.1875rem 0.25rem;
  gap: 0.125rem;
  text-align: start;
  box-shadow: none;
}
.model-settings-table :deep(.model-setting-picker button[aria-haspopup='dialog'] > svg) {
  width: 0.75rem;
  height: 0.75rem;
}
.model-settings-table :deep(.model-setting-picker .truncate) {
  overflow: visible;
  overflow-wrap: anywhere;
  text-overflow: clip;
  white-space: normal;
  line-height: 1.25;
}
.model-settings-table :deep(.model-setting-value) {
  display: block;
  padding-block: 0.3125rem;
}
.model-settings-table :deep(.model-setting-feedback td) {
  padding-top: 0;
}
.model-settings-table :deep(.model-setting-status button) {
  height: auto;
  min-height: 2rem;
  padding-block: 0.25rem;
  white-space: normal;
}
.model-settings-table :deep(.model-setting-compact-label) {
  display: none;
}

@container settings-card (width < 44rem) {
  .model-settings-table,
  .model-settings-table :deep(.model-setting-role) {
    display: block;
  }
  .model-settings-table colgroup {
    display: none;
  }
  .model-settings-table thead {
    @apply sr-only;
  }
  .model-settings-table :deep(.model-setting-row) {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    align-items: start;
    gap: 0.375rem 0.5rem;
    padding-block: 0.5rem;
  }
  .model-settings-table :deep(.model-setting-row > *) {
    min-width: 0;
    padding: 0;
  }
  .model-settings-table :deep(.model-setting-name) {
    grid-column: span 2;
    padding-block-start: 0.375rem;
  }
  .model-settings-table :deep(.model-setting-route) {
    grid-column: span 4;
  }
  .model-settings-table :deep(.model-setting-preference) {
    grid-column: span 2;
  }
  .model-settings-table :deep(.model-setting-compact-label) {
    @apply text-muted-foreground;
    display: block;
    margin-bottom: 0.1875rem;
    font-size: 0.75rem;
    font-weight: 550;
  }
  .model-settings-table :deep(.model-setting-feedback),
  .model-settings-table :deep(.model-setting-feedback td) {
    display: block;
  }
  .model-settings-table :deep(.model-setting-feedback td) {
    padding-inline: 0;
  }
}

@container settings-card (width < 28rem) {
  .model-settings-table :deep(.model-setting-name),
  .model-settings-table :deep(.model-setting-route),
  .model-settings-table :deep(.model-setting-role-with-sources .model-setting-preference) {
    grid-column: 1 / -1;
  }
  .model-settings-table :deep(.model-setting-name) {
    padding-block-start: 0;
  }
  .model-settings-table :deep(.model-setting-role-with-sources .model-setting-preference-controls) {
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
  }
}
</style>
