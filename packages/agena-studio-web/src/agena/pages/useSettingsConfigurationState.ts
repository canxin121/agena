import { computed, reactive, ref, type Ref } from 'vue'

import { deleteSettings, getSettings, setSettings, type ConfigSettingsReadResponse } from '../lib/agenaApi'
import {
  createSettingsConfigurationDraft,
  formatSettingsConfigurationValue,
  parseSettingsConfigurationDraft,
  settingsConfigurationDraftChanged,
  settingsConfigurationFields,
  settingsConfigurationSectionLabels,
  valueAtSettingsPath,
  type SettingsConfigurationDraft,
  type SettingsConfigurationField,
  type SettingsConfigurationSection,
} from './settingsConfigurationModel'

export type SettingsConfigurationStateDeps = {
  deleteSettings: typeof deleteSettings
  getSettings: typeof getSettings
  setSettings: typeof setSettings
}

export const advancedConfigurationSections = [
  {
    path: 'harnesses.browser',
    label: 'Browser harnesses',
    description: 'Named browser automation drivers, domain allowlists, and viewport policies.',
  },
  {
    path: 'harnesses.shell',
    label: 'Shell harnesses',
    description: 'Named shell environments and command allow/deny policies.',
  },
  {
    path: 'harnesses.editor',
    label: 'Editor harnesses',
    description: 'Named editor environments, extension allowlists, and file-size limits.',
  },
] as const

const defaultDeps: SettingsConfigurationStateDeps = {
  deleteSettings,
  getSettings,
  setSettings,
}

export function useSettingsConfigurationState(
  status: {
    actionError: Ref<string>
    actionMessage: Ref<string>
  },
  deps: SettingsConfigurationStateDeps = defaultDeps,
) {
  const effectiveSettings = ref<ConfigSettingsReadResponse | null>(null)
  const fileSettings = ref<ConfigSettingsReadResponse | null>(null)
  const drafts = reactive<Record<string, SettingsConfigurationDraft>>({})
  const advancedDrafts = reactive<Record<string, string>>({})
  const loading = ref(false)
  const savingPaths = reactive(new Set<string>())
  const search = ref('')

  const sections = computed(() => {
    const sectionOrder: SettingsConfigurationSection[] = ['defaults', 'interface', 'tracing', 'runtime', 'session']
    const query = search.value.trim().toLowerCase()
    return sectionOrder
      .map((id) => ({
        id,
        label: settingsConfigurationSectionLabels[id],
        fields: settingsConfigurationFields.filter(
          (field) =>
            field.section === id &&
            (!query ||
              [field.label, field.description, field.path, settingsConfigurationSectionLabels[id]]
                .join(' ')
                .toLowerCase()
                .includes(query)),
        ),
      }))
      .filter((section) => !query || section.fields.length > 0)
  })

  const dirtyCount = computed(
    () =>
      settingsConfigurationFields.filter((field) => isFieldChanged(field)).length +
      advancedConfigurationSections.filter((section) => isAdvancedChanged(section.path)).length,
  )

  function replaceDrafts() {
    for (const key of Object.keys(drafts)) delete drafts[key]
    for (const field of settingsConfigurationFields) {
      drafts[field.path] = createSettingsConfigurationDraft(fileSettings.value?.value, field)
    }
    for (const section of advancedConfigurationSections) {
      const value = valueAtSettingsPath(fileSettings.value?.value, section.path)
      advancedDrafts[section.path] = value === undefined ? '' : JSON.stringify(value, null, 2)
    }
  }

  async function load(announce = false) {
    loading.value = true
    status.actionError.value = ''
    if (announce) status.actionMessage.value = ''
    try {
      const [effective, file] = await Promise.all([
        deps.getSettings({ source: 'effective' }),
        deps.getSettings({ source: 'file' }),
      ])
      effectiveSettings.value = effective
      fileSettings.value = file
      replaceDrafts()
      if (announce) status.actionMessage.value = 'Configuration values refreshed.'
    } catch (error) {
      status.actionError.value = error instanceof Error ? error.message : String(error)
    } finally {
      loading.value = false
    }
  }

  function effectiveValue(field: SettingsConfigurationField): string {
    return formatSettingsConfigurationValue(valueAtSettingsPath(effectiveSettings.value?.value, field.path))
  }

  function draftFor(field: SettingsConfigurationField): SettingsConfigurationDraft {
    return drafts[field.path] ?? { override: false, value: '' }
  }

  function isFieldChanged(field: SettingsConfigurationField): boolean {
    return settingsConfigurationDraftChanged(fileSettings.value?.value, field, draftFor(field))
  }

  function setOverride(field: SettingsConfigurationField, override: boolean) {
    const draft = draftFor(field)
    drafts[field.path] = {
      override,
      value:
        override && !draft.value ? effectiveValue(field) || (field.kind === 'boolean' ? 'false' : '') : draft.value,
    }
  }

  function setDraftValue(field: SettingsConfigurationField, value: string) {
    drafts[field.path] = { ...draftFor(field), value }
  }

  function resetField(field: SettingsConfigurationField) {
    drafts[field.path] = createSettingsConfigurationDraft(fileSettings.value?.value, field)
  }

  function advancedBaseline(path: string): string {
    const value = valueAtSettingsPath(fileSettings.value?.value, path)
    return value === undefined ? '' : JSON.stringify(value, null, 2)
  }

  function isAdvancedChanged(path: string): boolean {
    return (advancedDrafts[path] || '').trim() !== advancedBaseline(path).trim()
  }

  function effectiveAdvancedValue(path: string): string {
    const value = valueAtSettingsPath(effectiveSettings.value?.value, path)
    return value === undefined ? 'unset' : JSON.stringify(value, null, 2)
  }

  function resetAdvanced(path: string) {
    advancedDrafts[path] = advancedBaseline(path)
  }

  async function saveAdvanced(path: string) {
    const raw = (advancedDrafts[path] || '').trim()
    status.actionError.value = ''
    status.actionMessage.value = ''
    savingPaths.add(path)
    try {
      const response = raw
        ? await deps.setSettings({ path, value: JSON.parse(raw), validate: true, reload: true })
        : await deps.deleteSettings({ path, validate: true, reload: true })
      const reloadLabel = response.reload
        ? ` Runtime generation ${response.reload.previous_generation} → ${response.reload.generation}.`
        : response.reload_required
          ? ' A runtime reload is still required.'
          : ''
      status.actionMessage.value = raw ? `Saved ${path}.${reloadLabel}` : `Removed the ${path} override.${reloadLabel}`
      await load(false)
    } catch (error) {
      status.actionError.value = error instanceof Error ? error.message : String(error)
    } finally {
      savingPaths.delete(path)
    }
  }

  async function saveField(field: SettingsConfigurationField) {
    const draft = draftFor(field)
    status.actionError.value = ''
    status.actionMessage.value = ''
    savingPaths.add(field.path)
    try {
      const response = draft.override
        ? await deps.setSettings({
            path: field.path,
            value: parseSettingsConfigurationDraft(field, draft),
            validate: true,
            reload: true,
          })
        : await deps.deleteSettings({
            path: field.path,
            validate: true,
            reload: true,
          })
      const reloadLabel = response.reload
        ? ` Runtime generation ${response.reload.previous_generation} → ${response.reload.generation}.`
        : response.reload_required
          ? ' A runtime reload is still required.'
          : ''
      status.actionMessage.value = draft.override
        ? `Saved ${field.label}.${reloadLabel}`
        : `Restored ${field.label} to its inherited value.${reloadLabel}`
      await load(false)
    } catch (error) {
      status.actionError.value = error instanceof Error ? error.message : String(error)
    } finally {
      savingPaths.delete(field.path)
    }
  }

  return {
    configFound: computed(() => fileSettings.value?.config_found ?? false),
    configPath: computed(() => fileSettings.value?.config_path || effectiveSettings.value?.config_path || ''),
    dirtyCount,
    drafts,
    advancedDrafts,
    advancedSections: advancedConfigurationSections,
    effectiveAdvancedValue,
    effectiveValue,
    fileSettings,
    isFieldChanged,
    isAdvancedChanged,
    load,
    loading,
    resetField,
    resetAdvanced,
    search,
    saveField,
    saveAdvanced,
    savingPaths,
    sections,
    setDraftValue,
    setOverride,
  }
}
