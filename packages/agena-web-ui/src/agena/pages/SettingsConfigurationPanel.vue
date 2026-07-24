<script setup lang="ts">
import type { useSettingsConfigurationState } from './useSettingsConfigurationState'

const props = defineProps<{
  configuration: ReturnType<typeof useSettingsConfigurationState>
}>()
</script>

<template>
  <div class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Agena Configuration</p>
          <h3 class="settings-panel-title">Runtime settings file</h3>
        </div>
        <div class="button-row">
          <span v-if="props.configuration.dirtyCount.value" class="badge warn">
            {{ props.configuration.dirtyCount.value }} unsaved
          </span>
          <button
            class="button ghost"
            :disabled="props.configuration.loading.value"
            @click="props.configuration.load(true)"
          >
            Refresh
          </button>
        </div>
      </div>

      <div class="settings-summary">
        <div class="summary-item">
          <div class="summary-label">Config file</div>
          <div class="summary-value mono">{{ props.configuration.configPath.value || 'not reported' }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">File status</div>
          <div class="summary-value">{{ props.configuration.configFound.value ? 'present' : 'defaults only' }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Exposed settings</div>
          <div class="summary-value">25 typed · 3 JSON groups</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Unsaved</div>
          <div class="summary-value">{{ props.configuration.dirtyCount.value }}</div>
        </div>
      </div>

      <p class="muted">
        Each value can inherit Agena's resolved default or write an explicit override. Saves are validated and request a
        runtime reload through the same settings API used by the TUI.
      </p>
      <div class="field">
        <label class="label" for="configuration-search">Search configuration</label>
        <input
          id="configuration-search"
          v-model="props.configuration.search.value"
          class="input"
          placeholder="path, label, description, or group"
        />
      </div>
    </section>

    <div v-if="!props.configuration.sections.value.length" class="empty-state">
      No configuration fields match “{{ props.configuration.search.value }}”.
    </div>

    <section
      v-if="
        !props.configuration.search.value.trim() ||
        ['harness', 'browser', 'shell', 'editor'].some((term) =>
          term.includes(props.configuration.search.value.trim().toLowerCase()),
        )
      "
      class="settings-panel"
    >
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Advanced configuration</p>
          <h3 class="settings-panel-title">Harness JSON</h3>
          <p class="record-subtitle">Edit the structured harness groups exposed by the TUI settings studio.</p>
        </div>
        <span class="badge neutral">3 groups</span>
      </div>
      <div class="record-list">
        <article v-for="section in props.configuration.advancedSections" :key="section.path" class="record-card">
          <div class="record-header">
            <div>
              <h4 class="record-title">{{ section.label }}</h4>
              <p class="record-subtitle">{{ section.description }}</p>
              <p class="muted mono">{{ section.path }}</p>
            </div>
            <span v-if="props.configuration.isAdvancedChanged(section.path)" class="badge warn">changed</span>
          </div>
          <div class="form-grid">
            <div class="field">
              <label class="label" :for="`advanced-${section.path}`"
                >File override (JSON; empty removes override)</label
              >
              <textarea
                :id="`advanced-${section.path}`"
                v-model="props.configuration.advancedDrafts[section.path]"
                class="textarea mono"
                rows="10"
                placeholder="{}"
              />
            </div>
            <div class="field">
              <label class="label">Effective now</label>
              <pre class="mono raw-block">{{ props.configuration.effectiveAdvancedValue(section.path) }}</pre>
            </div>
          </div>
          <div class="button-row">
            <button
              class="button primary"
              :disabled="
                !props.configuration.isAdvancedChanged(section.path) ||
                props.configuration.savingPaths.has(section.path)
              "
              @click="props.configuration.saveAdvanced(section.path)"
            >
              {{ props.configuration.savingPaths.has(section.path) ? 'Saving…' : 'Save JSON' }}
            </button>
            <button
              class="button ghost"
              :disabled="!props.configuration.isAdvancedChanged(section.path)"
              @click="props.configuration.resetAdvanced(section.path)"
            >
              Discard
            </button>
          </div>
        </article>
      </div>
    </section>

    <section v-for="section in props.configuration.sections.value" :key="section.id" class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Configuration group</p>
          <h3 class="settings-panel-title">{{ section.label }}</h3>
        </div>
        <span class="badge neutral">{{ section.fields.length }}</span>
      </div>

      <div class="configuration-field-list">
        <article v-for="field in section.fields" :key="field.path" class="record-card configuration-field-card">
          <div class="record-header">
            <div>
              <h4 class="record-title">{{ field.label }}</h4>
              <p class="record-subtitle">{{ field.description }}</p>
              <p class="muted mono">{{ field.path }}</p>
            </div>
            <div class="record-meta">
              <span class="badge neutral">{{ field.kind }}</span>
              <span v-if="props.configuration.isFieldChanged(field)" class="badge warn">changed</span>
            </div>
          </div>

          <div class="configuration-field-editor">
            <label class="configuration-override-toggle">
              <input
                type="checkbox"
                :checked="props.configuration.drafts[field.path]?.override"
                @change="props.configuration.setOverride(field, ($event.target as HTMLInputElement).checked)"
              />
              Write an explicit file override
            </label>

            <div class="field">
              <label class="label" :for="`configuration-${field.path}`">Override value</label>
              <select
                v-if="field.kind === 'boolean'"
                :id="`configuration-${field.path}`"
                class="select"
                :disabled="!props.configuration.drafts[field.path]?.override"
                :value="props.configuration.drafts[field.path]?.value || 'false'"
                @input="props.configuration.setDraftValue(field, ($event.target as HTMLSelectElement).value)"
              >
                <option value="true">Enabled</option>
                <option value="false">Disabled</option>
              </select>
              <input
                v-else
                :id="`configuration-${field.path}`"
                class="input mono"
                :disabled="!props.configuration.drafts[field.path]?.override"
                :inputmode="field.kind === 'integer' ? 'numeric' : 'text'"
                :placeholder="field.placeholder || props.configuration.effectiveValue(field) || 'No resolved value'"
                :value="props.configuration.drafts[field.path]?.value || ''"
                @input="props.configuration.setDraftValue(field, ($event.target as HTMLInputElement).value)"
              />
            </div>

            <div class="configuration-effective-value">
              <span class="summary-label">Effective now</span>
              <span class="mono">{{ props.configuration.effectiveValue(field) || 'unset' }}</span>
            </div>

            <div class="button-row configuration-field-actions">
              <button
                class="button primary"
                :disabled="
                  !props.configuration.isFieldChanged(field) || props.configuration.savingPaths.has(field.path)
                "
                @click="props.configuration.saveField(field)"
              >
                {{ props.configuration.savingPaths.has(field.path) ? 'Saving…' : 'Save' }}
              </button>
              <button
                class="button ghost"
                :disabled="!props.configuration.isFieldChanged(field)"
                @click="props.configuration.resetField(field)"
              >
                Discard
              </button>
            </div>
          </div>
        </article>
      </div>
    </section>
  </div>
</template>
