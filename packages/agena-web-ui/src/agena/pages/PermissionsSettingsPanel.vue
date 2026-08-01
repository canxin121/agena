<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'

import { userErrorMessage } from '@/lib/api'
import type {
  ConfigSettingsReadResponse,
  PermissionConfig,
  PermissionMode,
  SessionExecutionResource,
} from '../lib/agenaApi'
import { setSessionPermission, setSettings } from '../lib/agenaApi'
import {
  clonePermissionEditorModel,
  commandRuleRows,
  countPermissionDraftChanges,
  createPermissionEditorModel,
  normalizePermissionEditorModel,
  permissionConfigFromEditorModel,
  permissionModeLabel,
  replacePermissionEditorModel,
  suggestDuplicateKey,
  summarizePermissionEditorModel,
  type PermissionEditorModel,
} from './permissionSettingsModel'

type SectionId =
  | 'overview'
  | 'filesystem-default-zones'
  | 'filesystem-path-rules'
  | 'network-zones'
  | 'network-domain-rules'
  | 'entry-tag-rules'
  | 'entry-name-rules'
  | 'entry-command-rules'

type SimpleRuleKind = 'path' | 'domain' | 'tag' | 'name'
type DialogMode = 'add' | 'edit' | 'duplicate' | 'delete'

type SimpleRuleRow = {
  key: string
  read?: PermissionMode
  write?: PermissionMode
  access?: PermissionMode
}

type CommandRuleRow = {
  pattern: string
  access: PermissionMode
}

type DialogState = {
  open: boolean
  kind: SimpleRuleKind | 'command'
  mode: DialogMode
  originalKey: string
  originalEntry: string
  originalPattern: string
  key: string
  entry: string
  pattern: string
  read: PermissionMode
  write: PermissionMode
  access: PermissionMode
}

const props = defineProps<{
  loading: boolean
  load: () => void | Promise<void>
  permissionConfig: ConfigSettingsReadResponse | null
  clearActionStatus: () => void
  setActionError: (message: string) => void
  setActionMessage: (message: string) => void
  permissionScope?: 'global' | 'session'
  selectedSessionId?: number | null
  sessionExecution?: SessionExecutionResource | null
}>()

const activeSection = ref<SectionId>('overview')
const draft = reactive<PermissionEditorModel>(createPermissionEditorModel())
const baseline = ref<PermissionEditorModel>(createPermissionEditorModel())
const saving = ref(false)

const selectedPathRuleKey = ref('')
const selectedDomainRuleKey = ref('')
const selectedTagRuleKey = ref('')
const selectedNameRuleKey = ref('')
const selectedCommandEntry = ref('')
const selectedCommandPattern = ref('')

function createDialogState(): DialogState {
  return {
    open: false,
    kind: 'path',
    mode: 'add',
    originalKey: '',
    originalEntry: '',
    originalPattern: '',
    key: '',
    entry: '',
    pattern: '',
    read: 'auto',
    write: 'auto',
    access: 'auto',
  }
}

const dialog = reactive<DialogState>(createDialogState())

function hasPermissionConfigValue(value: unknown): boolean {
  return Boolean(value && typeof value === 'object' && Object.keys(value as Record<string, unknown>).length)
}

const dirtyCount = computed(() => countPermissionDraftChanges(draft, baseline.value))
const overviewCounts = computed(() => summarizePermissionEditorModel(draft))
const configSourceLabel = computed(() => props.permissionConfig?.source ?? 'effective')
const permissionScope = computed(() => props.permissionScope ?? 'global')
const sessionPermissionReady = computed(
  () =>
    permissionScope.value !== 'session' ||
    (props.selectedSessionId != null && props.sessionExecution?.session.id === props.selectedSessionId),
)
const isBusy = computed(() => props.loading || saving.value || !sessionPermissionReady.value)
const sourcePermissionValue = computed<unknown>(() =>
  permissionScope.value === 'session'
    ? hasPermissionConfigValue(props.sessionExecution?.execution.selected_permission)
      ? props.sessionExecution?.execution.selected_permission
      : props.sessionExecution?.execution.effective_permission || null
    : props.permissionConfig?.value,
)

const pathRuleRows = computed<SimpleRuleRow[]>(() =>
  Object.entries(draft.path.rules).map(([key, rule]) => ({
    key,
    read: rule.read,
    write: rule.write,
  })),
)

const domainRuleRows = computed<SimpleRuleRow[]>(() =>
  Object.entries(draft.network.rules).map(([key, access]) => ({
    key,
    access,
  })),
)

const tagRuleRows = computed<SimpleRuleRow[]>(() =>
  Object.entries(draft.entries.tags).map(([key, access]) => ({
    key,
    access,
  })),
)

const nameRuleRows = computed<SimpleRuleRow[]>(() =>
  Object.entries(draft.entries.names).map(([key, access]) => ({
    key,
    access,
  })),
)

const commandEntryKeys = computed(() => Object.keys(draft.entries.rules))
const commandRuleRowsForSelectedEntry = computed<CommandRuleRow[]>(() =>
  commandRuleRows(draft.entries.rules[selectedCommandEntry.value] ?? {}),
)

const selectedPathRule = computed(() => pathRuleRows.value.find((row) => row.key === selectedPathRuleKey.value) ?? null)
const selectedDomainRule = computed(
  () => domainRuleRows.value.find((row) => row.key === selectedDomainRuleKey.value) ?? null,
)
const selectedTagRule = computed(() => tagRuleRows.value.find((row) => row.key === selectedTagRuleKey.value) ?? null)
const selectedNameRule = computed(() => nameRuleRows.value.find((row) => row.key === selectedNameRuleKey.value) ?? null)
const selectedCommandRule = computed(
  () => commandRuleRowsForSelectedEntry.value.find((row) => row.pattern === selectedCommandPattern.value) ?? null,
)

const overviewSections = computed(() => [
  {
    title: 'Filesystem',
    rows: [
      {
        label: 'workspace',
        value: `read: ${permissionModeLabel(draft.path.workspace.read)}   write: ${permissionModeLabel(draft.path.workspace.write)}`,
      },
      {
        label: 'external',
        value: `read: ${permissionModeLabel(draft.path.external.read)}   write: ${permissionModeLabel(draft.path.external.write)}`,
      },
      {
        label: 'path rules',
        value: String(overviewCounts.value.pathRules),
      },
    ],
  },
  {
    title: 'Network',
    rows: [
      {
        label: 'internet',
        value: permissionModeLabel(draft.network.internet),
      },
      {
        label: 'private',
        value: permissionModeLabel(draft.network.private),
      },
      {
        label: 'loopback',
        value: permissionModeLabel(draft.network.loopback),
      },
      {
        label: 'domain rules',
        value: String(overviewCounts.value.networkRules),
      },
    ],
  },
  {
    title: 'Entry Access',
    rows: [
      {
        label: 'tag rules',
        value: String(overviewCounts.value.tagRules),
      },
      {
        label: 'name rules',
        value: String(overviewCounts.value.nameRules),
      },
      {
        label: 'command rules',
        value: String(overviewCounts.value.commandRules),
      },
    ],
  },
])

watch(
  sourcePermissionValue,
  (value) => {
    const next = normalizePermissionEditorModel(value)
    baseline.value = clonePermissionEditorModel(next)
    replacePermissionEditorModel(draft, next)
    closeDialog()
  },
  { immediate: true },
)

watch(
  pathRuleRows,
  (rows) =>
    syncSelection(
      selectedPathRuleKey,
      rows.map((row) => row.key),
    ),
  { immediate: true },
)
watch(
  domainRuleRows,
  (rows) =>
    syncSelection(
      selectedDomainRuleKey,
      rows.map((row) => row.key),
    ),
  { immediate: true },
)
watch(
  tagRuleRows,
  (rows) =>
    syncSelection(
      selectedTagRuleKey,
      rows.map((row) => row.key),
    ),
  { immediate: true },
)
watch(
  nameRuleRows,
  (rows) =>
    syncSelection(
      selectedNameRuleKey,
      rows.map((row) => row.key),
    ),
  { immediate: true },
)
watch(commandEntryKeys, (keys) => syncSelection(selectedCommandEntry, keys), { immediate: true })
watch(
  commandRuleRowsForSelectedEntry,
  (rows) =>
    syncSelection(
      selectedCommandPattern,
      rows.map((row) => row.pattern),
    ),
  { immediate: true },
)

function syncSelection(target: { value: string }, keys: string[]) {
  if (!keys.length) {
    target.value = ''
    return
  }
  if (!keys.includes(target.value)) {
    target.value = keys[0]
  }
}

function closeDialog() {
  Object.assign(dialog, createDialogState())
}

function sectionTitle(section: SectionId) {
  switch (section) {
    case 'overview':
      return 'Overview'
    case 'filesystem-default-zones':
      return 'Filesystem / Default Zones'
    case 'filesystem-path-rules':
      return 'Filesystem / Path Rules'
    case 'network-zones':
      return 'Network / Network Zones'
    case 'network-domain-rules':
      return 'Network / Domain Rules'
    case 'entry-tag-rules':
      return 'Entry Access / Tag Rules'
    case 'entry-name-rules':
      return 'Entry Access / Name Rules'
    case 'entry-command-rules':
      return 'Entry Access / Command Rules'
  }
}

function simpleRuleTitle(kind: SimpleRuleKind) {
  switch (kind) {
    case 'path':
      return 'Path Rule'
    case 'domain':
      return 'Domain Rule'
    case 'tag':
      return 'Tag Rule'
    case 'name':
      return 'Name Rule'
  }
}

function simpleRuleKeyLabel(kind: SimpleRuleKind) {
  switch (kind) {
    case 'path':
      return 'Path'
    case 'domain':
      return 'Domain'
    case 'tag':
      return 'Tag'
    case 'name':
      return 'Entry'
  }
}

function simpleRulePlaceholder(kind: SimpleRuleKind) {
  switch (kind) {
    case 'path':
      return 'docs/**'
    case 'domain':
      return 'api.openai.com'
    case 'tag':
      return 'read_only'
    case 'name':
      return 'web.search'
  }
}

function simpleRuleDefaultAccess(kind: SimpleRuleKind): PermissionMode {
  void kind
  return 'auto'
}

function simpleRuleMap(model: PermissionEditorModel, kind: SimpleRuleKind) {
  switch (kind) {
    case 'path':
      return model.path.rules
    case 'domain':
      return model.network.rules
    case 'tag':
      return model.entries.tags
    case 'name':
      return model.entries.names
  }
}

function simpleRuleRows(kind: SimpleRuleKind) {
  switch (kind) {
    case 'path':
      return pathRuleRows.value
    case 'domain':
      return domainRuleRows.value
    case 'tag':
      return tagRuleRows.value
    case 'name':
      return nameRuleRows.value
  }
}

function simpleRuleSelection(kind: SimpleRuleKind) {
  switch (kind) {
    case 'path':
      return selectedPathRuleKey.value
    case 'domain':
      return selectedDomainRuleKey.value
    case 'tag':
      return selectedTagRuleKey.value
    case 'name':
      return selectedNameRuleKey.value
  }
}

function setSimpleRuleSelection(kind: SimpleRuleKind, key: string) {
  switch (kind) {
    case 'path':
      selectedPathRuleKey.value = key
      return
    case 'domain':
      selectedDomainRuleKey.value = key
      return
    case 'tag':
      selectedTagRuleKey.value = key
      return
    case 'name':
      selectedNameRuleKey.value = key
  }
}

function selectedSimpleRule(kind: SimpleRuleKind) {
  switch (kind) {
    case 'path':
      return selectedPathRule.value
    case 'domain':
      return selectedDomainRule.value
    case 'tag':
      return selectedTagRule.value
    case 'name':
      return selectedNameRule.value
  }
}

function commandDeleteLabel() {
  const entry = dialog.originalEntry.trim()
  const pattern = dialog.originalPattern.trim()
  return entry && pattern ? `${entry} / ${pattern}` : pattern || entry || 'command rule'
}

function openSimpleRuleDialog(kind: SimpleRuleKind, mode: Exclude<DialogMode, 'delete'>) {
  const row = selectedSimpleRule(kind)
  if (mode !== 'add' && !row) return

  closeDialog()
  dialog.open = true
  dialog.kind = kind
  dialog.mode = mode

  if (mode === 'add') {
    dialog.key = ''
    dialog.read = 'auto'
    dialog.write = 'auto'
    dialog.access = simpleRuleDefaultAccess(kind)
    return
  }

  if (!row) return

  dialog.originalKey = row.key
  dialog.key =
    mode === 'duplicate'
      ? suggestDuplicateKey(
          row.key,
          simpleRuleRows(kind).map((entry) => entry.key),
        )
      : row.key
  dialog.read = row.read ?? 'auto'
  dialog.write = row.write ?? 'auto'
  dialog.access = row.access ?? simpleRuleDefaultAccess(kind)
}

function openSimpleDeleteDialog(kind: SimpleRuleKind) {
  const row = selectedSimpleRule(kind)
  if (!row) return

  closeDialog()
  dialog.open = true
  dialog.kind = kind
  dialog.mode = 'delete'
  dialog.originalKey = row.key
  dialog.key = row.key
  dialog.read = row.read ?? 'auto'
  dialog.write = row.write ?? 'auto'
  dialog.access = row.access ?? simpleRuleDefaultAccess(kind)
}

function openCommandDialog(mode: Exclude<DialogMode, 'delete'>) {
  const row = selectedCommandRule.value
  if (mode !== 'add' && !row) return

  closeDialog()
  dialog.open = true
  dialog.kind = 'command'
  dialog.mode = mode

  if (mode === 'add') {
    dialog.entry = selectedCommandEntry.value || 'bash'
    dialog.pattern = ''
    dialog.access = 'auto'
    return
  }

  if (!row) return

  dialog.originalEntry = selectedCommandEntry.value
  dialog.originalPattern = row.pattern
  dialog.entry = selectedCommandEntry.value || 'bash'
  dialog.pattern =
    mode === 'duplicate'
      ? suggestDuplicateKey(
          row.pattern,
          commandRuleRowsForSelectedEntry.value.map((entry) => entry.pattern),
        )
      : row.pattern
  dialog.access = row.access
}

function openCommandDeleteDialog() {
  const row = selectedCommandRule.value
  if (!row) return

  closeDialog()
  dialog.open = true
  dialog.kind = 'command'
  dialog.mode = 'delete'
  dialog.originalEntry = selectedCommandEntry.value
  dialog.originalPattern = row.pattern
  dialog.entry = selectedCommandEntry.value
  dialog.pattern = row.pattern
  dialog.access = row.access
}

function hasOwnKey(target: Record<string, unknown>, key: string) {
  return Object.prototype.hasOwnProperty.call(target, key)
}

async function persistSnapshot(
  snapshot: PermissionEditorModel,
  successMessage = 'Permission config saved.',
): Promise<boolean> {
  if (saving.value) return false

  props.clearActionStatus()
  saving.value = true
  const editorSnapshot = clonePermissionEditorModel(snapshot)
  const payload = permissionConfigFromEditorModel(editorSnapshot)

  try {
    if (permissionScope.value === 'session') {
      const sessionId = props.selectedSessionId
      if (sessionId == null || !sessionPermissionReady.value || !props.sessionExecution) {
        props.setActionError('Select and load a session before editing session permissions.')
        return false
      }
      await setSessionPermission(sessionId, payload as PermissionConfig, props.sessionExecution?.session.version)
    } else {
      await setSettings({
        path: 'permission',
        value: payload,
        validate: true,
        reload: true,
      })
    }
    baseline.value = editorSnapshot
    replacePermissionEditorModel(draft, editorSnapshot)
    await props.load()
    props.setActionMessage(successMessage)
    return true
  } catch (error) {
    props.setActionError(userErrorMessage(error))
    return false
  } finally {
    saving.value = false
  }
}

async function saveCurrentDraft() {
  await persistSnapshot(draft)
}

async function saveDialog() {
  if (dialog.kind === 'command') {
    return await saveCommandDialog()
  }
  return await saveSimpleDialog()
}

async function saveSimpleDialog() {
  const kind = dialog.kind as SimpleRuleKind
  const key = dialog.key.trim()

  if (!key) {
    props.setActionError(`${simpleRuleKeyLabel(kind)} is required.`)
    return false
  }

  const next = clonePermissionEditorModel(draft)
  const map = simpleRuleMap(next, kind)
  const originalKey = dialog.originalKey.trim()

  if (dialog.mode === 'delete') {
    if (!originalKey) return false
    if (!hasOwnKey(map, originalKey)) {
      props.setActionError(`${simpleRuleTitle(kind)} does not exist.`)
      return false
    }
    delete map[originalKey]
  } else {
    if (dialog.mode === 'duplicate' && key === originalKey) {
      props.setActionError(`Choose a different ${simpleRuleKeyLabel(kind).toLowerCase()} for the duplicate.`)
      return false
    }
    if (key !== originalKey && hasOwnKey(map, key)) {
      props.setActionError(`${simpleRuleKeyLabel(kind)} ${key} already exists.`)
      return false
    }

    if (dialog.mode === 'edit' && originalKey && key !== originalKey) {
      delete map[originalKey]
    }

    if (kind === 'path') {
      map[key] = { read: dialog.read, write: dialog.write }
    } else {
      map[key] = dialog.access
    }
  }

  const saved = await persistSnapshot(next)
  if (!saved) return false

  setSimpleRuleSelection(kind, key)
  closeDialog()
  return true
}

async function saveCommandDialog() {
  const entry = dialog.entry.trim()
  const pattern = dialog.pattern.trim()
  const originalEntry = dialog.originalEntry.trim()
  const originalPattern = dialog.originalPattern.trim()

  if (!entry) {
    props.setActionError('Entry is required.')
    return false
  }
  if (!pattern) {
    props.setActionError('Command pattern is required.')
    return false
  }

  const next = clonePermissionEditorModel(draft)
  const target = next.entries.rules[entry] || (next.entries.rules[entry] = {})
  const source = originalEntry ? next.entries.rules[originalEntry] : undefined

  if (dialog.mode === 'delete') {
    if (!source || !hasOwnKey(source, originalPattern)) {
      props.setActionError('Command rule does not exist.')
      return false
    }
    delete source[originalPattern]
    if (!Object.keys(source).length) {
      delete next.entries.rules[originalEntry]
    }
  } else {
    const sameLocation = entry === originalEntry && pattern === originalPattern
    if (dialog.mode === 'duplicate' && sameLocation) {
      props.setActionError('Choose a different entry or command pattern for the duplicate.')
      return false
    }
    if (!sameLocation && hasOwnKey(target, pattern)) {
      props.setActionError(`Command pattern ${pattern} already exists for ${entry}.`)
      return false
    }

    if (dialog.mode === 'edit' && originalEntry && originalPattern) {
      if (entry === originalEntry) {
        if (pattern !== originalPattern) {
          delete target[originalPattern]
        }
      } else if (source) {
        delete source[originalPattern]
        if (!Object.keys(source).length) {
          delete next.entries.rules[originalEntry]
        }
      }
    }

    target[pattern] = dialog.access
  }

  const saved = await persistSnapshot(next)
  if (!saved) return false

  selectedCommandEntry.value = entry
  selectedCommandPattern.value = pattern
  closeDialog()
  return true
}

function simpleActionButtonsDisabled(kind: SimpleRuleKind) {
  return isBusy.value || !selectedSimpleRule(kind)
}

function commandActionButtonsDisabled() {
  return isBusy.value || !selectedCommandRule.value || !selectedCommandEntry.value
}

function changeSection(section: SectionId) {
  if (isBusy.value) return
  activeSection.value = section
}
</script>

<template>
  <div class="settings-page permission-editor">
    <section class="settings-panel permission-editor-panel">
      <div class="permission-editor-header">
        <div>
          <p class="permission-editor-kicker">Permissions</p>
          <h2 class="permission-editor-title">
            {{
              permissionScope === 'session'
                ? `Session #${props.selectedSessionId ?? '—'} Policy Editor`
                : 'Policy Editor'
            }}
          </h2>
        </div>

        <div class="permission-editor-meta">
          <span class="badge neutral">Section: permission</span>
          <span class="badge neutral">Source: {{ configSourceLabel }}</span>
          <span class="badge" :class="dirtyCount ? 'warn' : 'success'">Unsaved changes: {{ dirtyCount }}</span>
          <span v-if="saving" class="badge warn">Saving…</span>
        </div>
      </div>

      <div v-if="permissionScope === 'session' && !sessionPermissionReady" class="permission-scope-warning">
        Select and load a session before editing session permissions. Open this page from an active chat session or
        include a session id in the settings URL.
      </div>

      <div class="permission-editor-body">
        <aside class="permission-sidebar">
          <button
            class="permission-nav-item"
            :class="{ active: activeSection === 'overview' }"
            :disabled="isBusy"
            type="button"
            @click="changeSection('overview')"
          >
            Overview
          </button>

          <div class="permission-nav-group">
            <div class="permission-nav-group-title">Filesystem</div>
            <button
              class="permission-nav-item nested"
              :class="{ active: activeSection === 'filesystem-default-zones' }"
              :disabled="isBusy"
              type="button"
              @click="changeSection('filesystem-default-zones')"
            >
              Default Zones
            </button>
            <button
              class="permission-nav-item nested"
              :class="{ active: activeSection === 'filesystem-path-rules' }"
              :disabled="isBusy"
              type="button"
              @click="changeSection('filesystem-path-rules')"
            >
              Path Rules
            </button>
          </div>

          <div class="permission-nav-group">
            <div class="permission-nav-group-title">Network</div>
            <button
              class="permission-nav-item nested"
              :class="{ active: activeSection === 'network-zones' }"
              :disabled="isBusy"
              type="button"
              @click="changeSection('network-zones')"
            >
              Network Zones
            </button>
            <button
              class="permission-nav-item nested"
              :class="{ active: activeSection === 'network-domain-rules' }"
              :disabled="isBusy"
              type="button"
              @click="changeSection('network-domain-rules')"
            >
              Domain Rules
            </button>
          </div>

          <div class="permission-nav-group">
            <div class="permission-nav-group-title">Entry Access</div>
            <button
              class="permission-nav-item nested"
              :class="{ active: activeSection === 'entry-tag-rules' }"
              :disabled="isBusy"
              type="button"
              @click="changeSection('entry-tag-rules')"
            >
              Tag Rules
            </button>
            <button
              class="permission-nav-item nested"
              :class="{ active: activeSection === 'entry-name-rules' }"
              :disabled="isBusy"
              type="button"
              @click="changeSection('entry-name-rules')"
            >
              Name Rules
            </button>
            <button
              class="permission-nav-item nested"
              :class="{ active: activeSection === 'entry-command-rules' }"
              :disabled="isBusy"
              type="button"
              @click="changeSection('entry-command-rules')"
            >
              Command Rules
            </button>
          </div>
        </aside>

        <div class="permission-content">
          <section v-if="activeSection === 'overview'" class="permission-section-card">
            <div class="permission-section-title-row">
              <h3 class="permission-section-title">{{ sectionTitle('overview') }}</h3>
            </div>

            <div class="overview-grid">
              <article v-for="block in overviewSections" :key="block.title" class="overview-block">
                <h4 class="overview-block-title">{{ block.title }}</h4>
                <div v-for="row in block.rows" :key="row.label" class="overview-row">
                  <span class="overview-row-label">{{ row.label }}</span>
                  <span class="overview-row-value">{{ row.value }}</span>
                </div>
              </article>
            </div>

            <div class="permission-approval-editor">
              <div class="permission-section-title-row">
                <h4 class="permission-section-title">Automatic approval model</h4>
                <span class="muted">Missing or unavailable models safely fall back to Ask.</span>
              </div>
              <div class="form-grid">
                <div class="field">
                  <label class="label" for="permission-approval-provider">Provider</label>
                  <input
                    id="permission-approval-provider"
                    v-model="draft.approvalModel.providerId"
                    class="input mono"
                    placeholder="openai"
                    :disabled="isBusy"
                    @change="saveCurrentDraft"
                  />
                </div>
                <div class="field">
                  <label class="label" for="permission-approval-adapter">Adapter (optional)</label>
                  <input
                    id="permission-approval-adapter"
                    v-model="draft.approvalModel.adapterId"
                    class="input mono"
                    placeholder="responses"
                    :disabled="isBusy"
                    @change="saveCurrentDraft"
                  />
                </div>
                <div class="field full">
                  <label class="label" for="permission-approval-model">Model</label>
                  <input
                    id="permission-approval-model"
                    v-model="draft.approvalModel.modelId"
                    class="input mono"
                    placeholder="gpt-5"
                    :disabled="isBusy"
                    @change="saveCurrentDraft"
                  />
                </div>
              </div>
            </div>
          </section>

          <section v-else-if="activeSection === 'filesystem-default-zones'" class="permission-section-card">
            <div class="permission-section-title-row">
              <h3 class="permission-section-title">{{ sectionTitle('filesystem-default-zones') }}</h3>
            </div>

            <div class="table-shell">
              <table class="permission-table">
                <thead>
                  <tr>
                    <th class="permission-table-key">Zone</th>
                    <th>Read</th>
                    <th>Write</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="permission-table-key mono">workspace</td>
                    <td>
                      <select
                        v-model="draft.path.workspace.read"
                        :disabled="isBusy"
                        class="select permission-select"
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                    <td>
                      <select
                        v-model="draft.path.workspace.write"
                        :disabled="isBusy"
                        class="select permission-select"
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                  <tr>
                    <td class="permission-table-key mono">external</td>
                    <td>
                      <select
                        v-model="draft.path.external.read"
                        :disabled="isBusy"
                        class="select permission-select"
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                    <td>
                      <select
                        v-model="draft.path.external.write"
                        :disabled="isBusy"
                        class="select permission-select"
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          <section v-else-if="activeSection === 'filesystem-path-rules'" class="permission-section-card">
            <div class="permission-section-title-row">
              <h3 class="permission-section-title">{{ sectionTitle('filesystem-path-rules') }}</h3>
            </div>

            <div class="table-shell">
              <table class="permission-table">
                <thead>
                  <tr>
                    <th class="permission-table-select">&nbsp;</th>
                    <th class="permission-table-key">Path</th>
                    <th>Read</th>
                    <th>Write</th>
                  </tr>
                </thead>
                <tbody>
                  <tr
                    v-for="row in pathRuleRows"
                    :key="row.key"
                    :class="{ selected: selectedPathRuleKey === row.key }"
                    @click="selectedPathRuleKey = row.key"
                  >
                    <td class="permission-table-select">{{ selectedPathRuleKey === row.key ? '>' : '' }}</td>
                    <td class="permission-table-key mono">{{ row.key }}</td>
                    <td>
                      <select
                        v-model="draft.path.rules[row.key].read"
                        :disabled="isBusy"
                        class="select permission-select"
                        @click.stop
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                    <td>
                      <select
                        v-model="draft.path.rules[row.key].write"
                        :disabled="isBusy"
                        class="select permission-select"
                        @click.stop
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div class="permission-actions">
              <button class="button" :disabled="isBusy" type="button" @click="openSimpleRuleDialog('path', 'add')">
                Add
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('path')"
                type="button"
                @click="openSimpleRuleDialog('path', 'edit')"
              >
                Edit
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('path')"
                type="button"
                @click="openSimpleRuleDialog('path', 'duplicate')"
              >
                Duplicate
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('path')"
                type="button"
                @click="openSimpleDeleteDialog('path')"
              >
                Delete
              </button>
            </div>
          </section>

          <section v-else-if="activeSection === 'network-zones'" class="permission-section-card">
            <div class="permission-section-title-row">
              <h3 class="permission-section-title">{{ sectionTitle('network-zones') }}</h3>
            </div>

            <div class="table-shell">
              <table class="permission-table">
                <thead>
                  <tr>
                    <th class="permission-table-key">Zone</th>
                    <th>Connect</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="permission-table-key mono">internet</td>
                    <td>
                      <select
                        v-model="draft.network.internet"
                        :disabled="isBusy"
                        class="select permission-select"
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                  <tr>
                    <td class="permission-table-key mono">private</td>
                    <td>
                      <select
                        v-model="draft.network.private"
                        :disabled="isBusy"
                        class="select permission-select"
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                  <tr>
                    <td class="permission-table-key mono">loopback</td>
                    <td>
                      <select
                        v-model="draft.network.loopback"
                        :disabled="isBusy"
                        class="select permission-select"
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          <section v-else-if="activeSection === 'network-domain-rules'" class="permission-section-card">
            <div class="permission-section-title-row">
              <h3 class="permission-section-title">{{ sectionTitle('network-domain-rules') }}</h3>
            </div>

            <div class="table-shell">
              <table class="permission-table">
                <thead>
                  <tr>
                    <th class="permission-table-select">&nbsp;</th>
                    <th class="permission-table-key">Domain</th>
                    <th>Connect</th>
                  </tr>
                </thead>
                <tbody>
                  <tr
                    v-for="row in domainRuleRows"
                    :key="row.key"
                    :class="{ selected: selectedDomainRuleKey === row.key }"
                    @click="selectedDomainRuleKey = row.key"
                  >
                    <td class="permission-table-select">{{ selectedDomainRuleKey === row.key ? '>' : '' }}</td>
                    <td class="permission-table-key mono">{{ row.key }}</td>
                    <td>
                      <select
                        v-model="draft.network.rules[row.key]"
                        :disabled="isBusy"
                        class="select permission-select"
                        @click.stop
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div class="permission-actions">
              <button class="button" :disabled="isBusy" type="button" @click="openSimpleRuleDialog('domain', 'add')">
                Add
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('domain')"
                type="button"
                @click="openSimpleRuleDialog('domain', 'edit')"
              >
                Edit
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('domain')"
                type="button"
                @click="openSimpleRuleDialog('domain', 'duplicate')"
              >
                Duplicate
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('domain')"
                type="button"
                @click="openSimpleDeleteDialog('domain')"
              >
                Delete
              </button>
            </div>
          </section>

          <section v-else-if="activeSection === 'entry-tag-rules'" class="permission-section-card">
            <div class="permission-section-title-row">
              <h3 class="permission-section-title">{{ sectionTitle('entry-tag-rules') }}</h3>
            </div>

            <div class="table-shell">
              <table class="permission-table">
                <thead>
                  <tr>
                    <th class="permission-table-select">&nbsp;</th>
                    <th class="permission-table-key">Tag</th>
                    <th>Access</th>
                  </tr>
                </thead>
                <tbody>
                  <tr
                    v-for="row in tagRuleRows"
                    :key="row.key"
                    :class="{ selected: selectedTagRuleKey === row.key }"
                    @click="selectedTagRuleKey = row.key"
                  >
                    <td class="permission-table-select">{{ selectedTagRuleKey === row.key ? '>' : '' }}</td>
                    <td class="permission-table-key mono">{{ row.key }}</td>
                    <td>
                      <select
                        v-model="draft.entries.tags[row.key]"
                        :disabled="isBusy"
                        class="select permission-select"
                        @click.stop
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div class="permission-actions">
              <button class="button" :disabled="isBusy" type="button" @click="openSimpleRuleDialog('tag', 'add')">
                Add
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('tag')"
                type="button"
                @click="openSimpleRuleDialog('tag', 'edit')"
              >
                Edit
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('tag')"
                type="button"
                @click="openSimpleRuleDialog('tag', 'duplicate')"
              >
                Duplicate
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('tag')"
                type="button"
                @click="openSimpleDeleteDialog('tag')"
              >
                Delete
              </button>
            </div>
          </section>

          <section v-else-if="activeSection === 'entry-name-rules'" class="permission-section-card">
            <div class="permission-section-title-row">
              <h3 class="permission-section-title">{{ sectionTitle('entry-name-rules') }}</h3>
            </div>

            <div class="table-shell">
              <table class="permission-table">
                <thead>
                  <tr>
                    <th class="permission-table-select">&nbsp;</th>
                    <th class="permission-table-key">Entry Name</th>
                    <th>Access</th>
                  </tr>
                </thead>
                <tbody>
                  <tr
                    v-for="row in nameRuleRows"
                    :key="row.key"
                    :class="{ selected: selectedNameRuleKey === row.key }"
                    @click="selectedNameRuleKey = row.key"
                  >
                    <td class="permission-table-select">{{ selectedNameRuleKey === row.key ? '>' : '' }}</td>
                    <td class="permission-table-key mono">{{ row.key }}</td>
                    <td>
                      <select
                        v-model="draft.entries.names[row.key]"
                        :disabled="isBusy"
                        class="select permission-select"
                        @click.stop
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div class="permission-actions">
              <button class="button" :disabled="isBusy" type="button" @click="openSimpleRuleDialog('name', 'add')">
                Add
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('name')"
                type="button"
                @click="openSimpleRuleDialog('name', 'edit')"
              >
                Edit
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('name')"
                type="button"
                @click="openSimpleRuleDialog('name', 'duplicate')"
              >
                Duplicate
              </button>
              <button
                class="button"
                :disabled="simpleActionButtonsDisabled('name')"
                type="button"
                @click="openSimpleDeleteDialog('name')"
              >
                Delete
              </button>
            </div>
          </section>

          <section v-else-if="activeSection === 'entry-command-rules'" class="permission-section-card">
            <div class="permission-section-title-row">
              <h3 class="permission-section-title">{{ sectionTitle('entry-command-rules') }}</h3>
            </div>

            <div class="permission-command-header">
              <div class="field compact">
                <label class="label" for="permission-command-entry">Entry</label>
                <select
                  id="permission-command-entry"
                  v-model="selectedCommandEntry"
                  :disabled="isBusy || !commandEntryKeys.length"
                  class="select permission-select"
                >
                  <option v-if="!commandEntryKeys.length" value="">No command entries yet</option>
                  <option v-for="entry in commandEntryKeys" :key="entry" :value="entry">
                    {{ entry }}
                  </option>
                </select>
              </div>
            </div>

            <div class="table-shell">
              <table class="permission-table">
                <thead>
                  <tr>
                    <th class="permission-table-select">&nbsp;</th>
                    <th class="permission-table-key">Command Pattern</th>
                    <th>Access</th>
                  </tr>
                </thead>
                <tbody>
                  <tr
                    v-for="row in commandRuleRowsForSelectedEntry"
                    :key="row.pattern"
                    :class="{ selected: selectedCommandPattern === row.pattern }"
                    @click="selectedCommandPattern = row.pattern"
                  >
                    <td class="permission-table-select">{{ selectedCommandPattern === row.pattern ? '>' : '' }}</td>
                    <td class="permission-table-key mono">{{ row.pattern }}</td>
                    <td>
                      <select
                        v-model="draft.entries.rules[selectedCommandEntry][row.pattern]"
                        :disabled="isBusy"
                        class="select permission-select"
                        @click.stop
                        @change="saveCurrentDraft"
                      >
                        <option value="allow">Allow</option>
                        <option value="auto">Auto</option>
                        <option value="ask">Ask</option>
                        <option value="deny">Deny</option>
                      </select>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div class="permission-actions">
              <button class="button" :disabled="isBusy" type="button" @click="openCommandDialog('add')">Add</button>
              <button
                class="button"
                :disabled="commandActionButtonsDisabled()"
                type="button"
                @click="openCommandDialog('edit')"
              >
                Edit
              </button>
              <button
                class="button"
                :disabled="commandActionButtonsDisabled()"
                type="button"
                @click="openCommandDialog('duplicate')"
              >
                Duplicate
              </button>
              <button
                class="button"
                :disabled="commandActionButtonsDisabled()"
                type="button"
                @click="openCommandDeleteDialog()"
              >
                Delete
              </button>
            </div>
          </section>
        </div>
      </div>
    </section>

    <teleport to="body">
      <div v-if="dialog.open" class="permission-modal-backdrop" @click.self="closeDialog">
        <section class="permission-modal">
          <form v-if="dialog.mode !== 'delete'" class="permission-modal-body" @submit.prevent="saveDialog">
            <div class="permission-modal-header">
              <h3 class="permission-modal-title">
                {{
                  dialog.kind === 'command'
                    ? `${dialog.mode === 'add' ? 'Add' : dialog.mode === 'edit' ? 'Edit' : 'Duplicate'} Command Rule`
                    : `${dialog.mode === 'add' ? 'Add' : dialog.mode === 'edit' ? 'Edit' : 'Duplicate'} ${simpleRuleTitle(dialog.kind)}`
                }}
              </h3>
            </div>

            <div v-if="dialog.kind === 'command'" class="permission-modal-grid">
              <div class="field">
                <label class="label" for="permission-command-entry-input">Entry</label>
                <input
                  id="permission-command-entry-input"
                  v-model="dialog.entry"
                  :disabled="isBusy"
                  class="input mono"
                  list="permission-command-entry-options"
                  placeholder="bash"
                />
              </div>
              <div class="field full">
                <label class="label" for="permission-command-pattern">Command</label>
                <input
                  id="permission-command-pattern"
                  v-model="dialog.pattern"
                  :disabled="isBusy"
                  class="input mono"
                  placeholder="rm -rf *"
                />
              </div>
              <div class="field">
                <label class="label" for="permission-command-access">Access</label>
                <select id="permission-command-access" v-model="dialog.access" :disabled="isBusy" class="select">
                  <option value="allow">Allow</option>
                  <option value="auto">Auto</option>
                  <option value="ask">Ask</option>
                  <option value="deny">Deny</option>
                </select>
              </div>
            </div>

            <div v-else class="permission-modal-grid">
              <div class="field full">
                <label class="label" :for="`permission-${dialog.kind}-key`">{{
                  simpleRuleKeyLabel(dialog.kind)
                }}</label>
                <input
                  :id="`permission-${dialog.kind}-key`"
                  v-model="dialog.key"
                  :disabled="isBusy"
                  class="input mono"
                  :placeholder="simpleRulePlaceholder(dialog.kind)"
                />
              </div>

              <template v-if="dialog.kind === 'path'">
                <div class="field">
                  <label class="label" for="permission-path-read">Read</label>
                  <select id="permission-path-read" v-model="dialog.read" :disabled="isBusy" class="select">
                    <option value="allow">Allow</option>
                    <option value="auto">Auto</option>
                    <option value="ask">Ask</option>
                    <option value="deny">Deny</option>
                  </select>
                </div>
                <div class="field">
                  <label class="label" for="permission-path-write">Write</label>
                  <select id="permission-path-write" v-model="dialog.write" :disabled="isBusy" class="select">
                    <option value="allow">Allow</option>
                    <option value="auto">Auto</option>
                    <option value="ask">Ask</option>
                    <option value="deny">Deny</option>
                  </select>
                </div>
              </template>

              <div v-else class="field">
                <label class="label" for="permission-rule-access">Access</label>
                <select id="permission-rule-access" v-model="dialog.access" :disabled="isBusy" class="select">
                  <option value="allow">Allow</option>
                  <option value="auto">Auto</option>
                  <option value="ask">Ask</option>
                  <option value="deny">Deny</option>
                </select>
              </div>
            </div>

            <div class="permission-modal-actions">
              <button class="button primary" :disabled="isBusy" type="submit">
                {{ dialog.mode === 'edit' ? 'Save' : 'Create' }}
              </button>
              <button class="button" :disabled="isBusy" type="button" @click="closeDialog">Cancel</button>
            </div>
          </form>

          <form v-else class="permission-modal-body" @submit.prevent="saveDialog">
            <div class="permission-modal-header">
              <h3 class="permission-modal-title">
                {{ dialog.kind === 'command' ? 'Delete Command Rule' : `Delete ${simpleRuleTitle(dialog.kind)}` }}
              </h3>
            </div>

            <div class="permission-delete-copy">
              {{
                dialog.kind === 'command'
                  ? `Delete command rule: ${commandDeleteLabel()}`
                  : `Delete ${simpleRuleKeyLabel(dialog.kind).toLowerCase()}: ${dialog.originalKey || dialog.key}`
              }}
            </div>

            <div class="permission-modal-actions">
              <button class="button danger" :disabled="isBusy" type="submit">Delete</button>
              <button class="button" :disabled="isBusy" type="button" @click="closeDialog">Cancel</button>
            </div>
          </form>
        </section>
      </div>

      <datalist id="permission-command-entry-options">
        <option v-for="entry in commandEntryKeys" :key="entry" :value="entry" />
      </datalist>
    </teleport>
  </div>
</template>

<style scoped>
.permission-editor {
  display: grid;
  gap: 16px;
}

.permission-editor-panel {
  display: grid;
  gap: 16px;
}

.permission-editor-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.permission-editor-kicker {
  margin: 0 0 4px;
  text-transform: uppercase;
  letter-spacing: 0.12em;
  font-size: 11px;
  color: #64748b;
}

.permission-editor-title {
  margin: 0;
  font-size: 24px;
  line-height: 1.2;
  color: #0f172a;
}

.permission-editor-meta {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
}

.permission-scope-warning {
  padding: 12px 14px;
  border: 1px solid rgba(180, 83, 9, 0.22);
  border-radius: 12px;
  background: rgba(255, 247, 237, 0.9);
  color: #9a3412;
  line-height: 1.5;
}

.permission-editor-body {
  display: grid;
  grid-template-columns: 230px minmax(0, 1fr);
  gap: 16px;
  align-items: start;
}

.permission-sidebar {
  display: grid;
  gap: 6px;
  padding: 8px;
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 16px;
  background: rgba(248, 250, 252, 0.88);
}

.permission-nav-group {
  display: grid;
  gap: 4px;
  padding-top: 6px;
}

.permission-nav-group-title {
  padding: 6px 10px 4px;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.12em;
  color: #64748b;
}

.permission-nav-item {
  width: 100%;
  border: 0;
  border-radius: 12px;
  background: transparent;
  color: #0f172a;
  text-align: left;
  cursor: pointer;
  font: inherit;
  padding: 10px 12px;
  transition:
    background-color 140ms ease,
    color 140ms ease,
    box-shadow 140ms ease,
    transform 140ms ease;
}

.permission-nav-item.nested {
  padding-left: 18px;
}

.permission-nav-item:hover:not(:disabled) {
  background: rgba(15, 23, 42, 0.04);
}

.permission-nav-item.active {
  background: rgba(15, 23, 42, 0.08);
  box-shadow: inset 0 0 0 1px rgba(15, 23, 42, 0.05);
  font-weight: 600;
}

.permission-nav-item:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.permission-content {
  min-width: 0;
  display: grid;
  gap: 16px;
}

.permission-section-card {
  display: grid;
  gap: 16px;
  padding: 20px;
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 18px;
  background: rgba(255, 255, 255, 0.95);
  box-shadow: 0 10px 30px rgba(15, 23, 42, 0.04);
}

.permission-section-title-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.permission-section-title {
  margin: 0;
  font-size: 18px;
  line-height: 1.2;
  color: #0f172a;
}

.overview-grid {
  display: grid;
  gap: 16px;
}

.overview-block {
  padding: 16px;
  border-radius: 14px;
  border: 1px solid rgba(15, 23, 42, 0.08);
  background: rgba(248, 250, 252, 0.82);
}

.overview-block-title {
  margin: 0 0 12px;
  font-size: 14px;
  color: #0f172a;
}

.overview-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
  padding: 8px 0;
  border-top: 1px solid rgba(15, 23, 42, 0.06);
}

.overview-row:first-of-type {
  border-top: 0;
  padding-top: 0;
}

.overview-row-label {
  text-transform: lowercase;
  color: #475569;
}

.overview-row-value {
  text-align: right;
  color: #0f172a;
  white-space: pre-wrap;
}

.table-shell {
  overflow: auto;
  border-radius: 16px;
  border: 1px solid rgba(15, 23, 42, 0.08);
  background: rgba(255, 255, 255, 0.9);
}

.permission-table {
  width: 100%;
  border-collapse: collapse;
}

.permission-table th,
.permission-table td {
  padding: 12px 14px;
  border-bottom: 1px solid rgba(15, 23, 42, 0.06);
  text-align: left;
  vertical-align: middle;
}

.permission-table thead th {
  background: rgba(248, 250, 252, 0.96);
  color: #475569;
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: 0.08em;
}

.permission-table tbody tr {
  cursor: pointer;
  transition: background-color 120ms ease;
}

.permission-table tbody tr:hover {
  background: rgba(15, 23, 42, 0.03);
}

.permission-table tbody tr.selected {
  background: rgba(15, 23, 42, 0.05);
}

.permission-table tbody tr:last-child td {
  border-bottom: 0;
}

.permission-table-key {
  width: min(56%, 520px);
}

.permission-table-select {
  width: 28px;
  color: #94a3b8;
  text-align: center;
}

.permission-select {
  width: 100%;
  min-width: 108px;
}

.permission-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}

.permission-command-header {
  display: flex;
  align-items: flex-end;
  justify-content: flex-start;
}

.field.compact {
  min-width: 180px;
}

.permission-modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1200;
  display: grid;
  place-items: center;
  padding: 24px;
  background: rgba(15, 23, 42, 0.52);
  backdrop-filter: blur(10px);
}

.permission-modal {
  width: min(560px, calc(100vw - 32px));
  max-height: min(90vh, 780px);
  overflow: auto;
  border-radius: 20px;
  border: 1px solid rgba(15, 23, 42, 0.1);
  background: rgba(255, 255, 255, 0.98);
  box-shadow: 0 24px 70px rgba(15, 23, 42, 0.3);
}

.permission-modal-body {
  display: grid;
  gap: 18px;
  padding: 22px;
}

.permission-modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.permission-modal-title {
  margin: 0;
  font-size: 18px;
  line-height: 1.2;
  color: #0f172a;
}

.permission-modal-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px;
}

.field.full {
  grid-column: 1 / -1;
}

.permission-modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}

.permission-delete-copy {
  color: #334155;
  line-height: 1.5;
}

@media (max-width: 980px) {
  .permission-editor-header {
    flex-direction: column;
  }

  .permission-editor-meta {
    justify-content: flex-start;
  }

  .permission-editor-body {
    grid-template-columns: 1fr;
  }

  .permission-sidebar {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .permission-nav-group {
    padding-top: 0;
  }
}

@media (max-width: 720px) {
  .permission-sidebar {
    grid-template-columns: 1fr;
  }

  .permission-modal-grid {
    grid-template-columns: 1fr;
  }

  .permission-table {
    min-width: 580px;
  }
}
</style>
