<script setup lang="ts">
import SettingsSourceControl from '../SettingsSourceControl.vue'

defineProps<{
  label: string
  value: string
  editable: boolean
  readOnly?: boolean
  scope?: string
  hint?: string
  source?: 'inherit' | 'local' | 'effective'
  inheritedValue?: string
  inheritedSource?: string
  disabled?: boolean
  inheritDisabled?: boolean
  localDisabled?: boolean
}>()
defineEmits<{ source: [value: 'inherit' | 'local'] }>()
</script>

<template>
  <div class="permission-policy-row" :class="{ 'is-layered': editable && source && scope !== 'global' }">
    <span class="permission-policy-label" :title="hint">{{ label }}</span>
    <div v-if="editable" class="permission-policy-controls">
      <div v-if="source" class="permission-policy-source">
        <SettingsSourceControl
          :model-value="source"
          :scope="scope"
          :label="label"
          :inherited-value="inheritedValue"
          :inherited-source="inheritedSource"
          :show-inherited-summary="false"
          :disabled="disabled"
          :inherit-disabled="inheritDisabled"
          :local-disabled="localDisabled"
          @update:model-value="$emit('source', $event)"
        />
      </div>
      <fieldset
        class="permission-policy-editor"
        :class="{ 'is-inherited': readOnly || (scope !== 'global' && source === 'inherit') }"
        :disabled="disabled || readOnly || (scope !== 'global' && source === 'inherit')"
        :data-setting-inherited="readOnly || (scope !== 'global' && source === 'inherit') ? '' : undefined"
      >
        <slot />
      </fieldset>
    </div>
    <span v-else class="permission-policy-value text-[13px] font-medium" data-permission-value>{{ value }}</span>
  </div>
</template>

<style scoped>
.permission-policy-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, min(12rem, 55%));
  align-items: center;
  gap: 0.25rem 0.625rem;
  min-height: 2.5rem;
  padding-block: 0.25rem;
}
.permission-policy-label {
  min-width: 0;
  font-size: 0.8125rem;
  font-weight: 550;
  line-height: 1.35;
  overflow-wrap: anywhere;
}
.permission-policy-row.is-layered {
  grid-template-areas: 'label editor' 'source source';
  padding-block: 0.3125rem;
}
.permission-policy-controls {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  min-width: 0;
}
.permission-policy-source {
  min-width: 0;
  max-width: 100%;
}
.is-layered .permission-policy-label {
  grid-area: label;
}
.is-layered .permission-policy-controls {
  display: contents;
}
.is-layered .permission-policy-editor {
  grid-area: editor;
}
.is-layered .permission-policy-source {
  grid-area: source;
  justify-self: end;
}
@container permission-domain (min-width: 20rem) {
  .permission-policy-row.is-layered {
    grid-template-areas: 'label editor source';
    grid-template-columns: minmax(0, 1fr) minmax(0, min(12rem, 43%)) auto;
    padding-block: 0.25rem;
  }
}
.permission-policy-editor.is-inherited {
  color: var(--color-muted-foreground);
}
.permission-policy-editor.is-inherited :deep(button:disabled) {
  opacity: 1;
  color: var(--color-muted-foreground);
  background-color: color-mix(in oklch, var(--color-muted) 25%, transparent);
}
.permission-policy-editor {
  margin: 0;
  padding: 0;
  border: 0;
  order: -1;
  flex: 1;
  min-width: 0;
}
.permission-policy-editor :deep(button[aria-haspopup]) {
  gap: 0.375rem;
  text-align: start;
}
.permission-policy-value {
  justify-self: end;
  max-width: 100%;
  padding: 0.1875rem 0.5rem;
  border-radius: 0.25rem;
  background: color-mix(in oklch, var(--color-muted) 45%, transparent);
  text-align: end;
  overflow-wrap: anywhere;
}
</style>
