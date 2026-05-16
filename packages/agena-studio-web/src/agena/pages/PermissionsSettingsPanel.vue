<script setup lang="ts">
import { computed } from 'vue'

import type { PermissionMode, PermissionRuleResource, PermissionSubjectKind } from '@/agena/lib/agenaApi'

type PermissionSubjectFilter = 'all' | PermissionSubjectKind

const props = defineProps<{
  loading: boolean
  permissionSearch: string
  permissionStatusFilter: 'all' | 'active' | 'revoked'
  permissionScopeFilter: 'all' | 'session' | 'workspace' | 'global'
  permissionModeFilter: 'all' | PermissionMode
  permissionSubjectFilter: PermissionSubjectFilter
  permissionDraft: {
    subjectKind: PermissionSubjectKind
    toolName: string
    qualifier: string
    pathAccessKind: string
    workspaceRoot: string
    targetPath: string
    networkTarget: string
    networkPort: string
    scope: 'session' | 'workspace' | 'global'
    sessionId: string
    mode: PermissionMode
  }
  editingPermissionRuleId: number | null
  filteredPermissionRules: PermissionRuleResource[]
  load: () => void | Promise<void>
  savePermissionRule: () => void | Promise<void>
  resetPermissionDraft: () => void
  editPermissionRule: (rule: PermissionRuleResource) => void
  revokePermissionRuleAction: (rule: PermissionRuleResource) => void | Promise<void>
  deletePermissionRuleAction: (rule: PermissionRuleResource) => void | Promise<void>
  permissionRuleLabel: (rule: PermissionRuleResource) => string
  permissionRulePreview: (rule: PermissionRuleResource) => string
  permissionRuleFacts: (rule: PermissionRuleResource) => string[]
}>()

const emit = defineEmits<{
  'update:permissionSearch': [value: string]
  'update:permissionStatusFilter': [value: 'all' | 'active' | 'revoked']
  'update:permissionScopeFilter': [value: 'all' | 'session' | 'workspace' | 'global']
  'update:permissionModeFilter': [value: 'all' | PermissionMode]
  'update:permissionSubjectFilter': [value: PermissionSubjectFilter]
}>()

const activeRuleCount = computed(() => props.filteredPermissionRules.filter((rule) => !rule.revoked_at).length)
const revokedRuleCount = computed(() => props.filteredPermissionRules.filter((rule) => rule.revoked_at).length)

function subjectLabel(value: string) {
  if (value === 'path_access') return 'Path'
  if (value === 'network_access') return 'Network'
  if (value === 'tool') return 'Tool'
  return 'All'
}

function modeBadgeClass(mode: PermissionMode) {
  if (mode === 'allow') return 'success'
  if (mode === 'deny') return 'danger'
  return 'warn'
}

function statusBadgeClass(rule: PermissionRuleResource) {
  return rule.revoked_at ? 'neutral' : 'success'
}
</script>

<template>
  <div class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Agena Runtime</p>
          <h3 class="settings-panel-title">Guardrails</h3>
        </div>
        <button class="button ghost" :disabled="props.loading" @click="props.load">Refresh</button>
      </div>

      <div class="settings-summary">
        <div class="summary-item">
          <div class="summary-label">Matched</div>
          <div class="summary-value">{{ props.filteredPermissionRules.length }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Active</div>
          <div class="summary-value">{{ activeRuleCount }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Revoked</div>
          <div class="summary-value">{{ revokedRuleCount }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Draft</div>
          <div class="summary-value">
            {{ subjectLabel(props.permissionDraft.subjectKind) }} · {{ props.permissionDraft.mode }}
          </div>
        </div>
      </div>

      <div class="settings-toolbar">
        <div class="field">
          <label class="label" for="permission-search">Search</label>
          <input
            id="permission-search"
            :value="props.permissionSearch"
            class="input mono"
            placeholder="bash, src/**, api.example.com"
            @input="emit('update:permissionSearch', ($event.target as HTMLInputElement).value)"
            @keyup.enter="props.load"
          />
        </div>
        <div class="field">
          <label class="label" for="permission-status-filter">Status</label>
          <select
            id="permission-status-filter"
            :value="props.permissionStatusFilter"
            class="select"
            @change="
              emit(
                'update:permissionStatusFilter',
                ($event.target as HTMLSelectElement).value as 'all' | 'active' | 'revoked',
              )
            "
          >
            <option value="active">Active</option>
            <option value="revoked">Revoked</option>
            <option value="all">All</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="permission-scope-filter">Scope</label>
          <select
            id="permission-scope-filter"
            :value="props.permissionScopeFilter"
            class="select"
            @change="
              emit(
                'update:permissionScopeFilter',
                ($event.target as HTMLSelectElement).value as 'all' | 'session' | 'workspace' | 'global',
              )
            "
          >
            <option value="all">All</option>
            <option value="workspace">Workspace</option>
            <option value="session">Session</option>
            <option value="global">Global</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="permission-mode-filter">Mode</label>
          <select
            id="permission-mode-filter"
            :value="props.permissionModeFilter"
            class="select"
            @change="
              emit('update:permissionModeFilter', ($event.target as HTMLSelectElement).value as 'all' | PermissionMode)
            "
          >
            <option value="all">All</option>
            <option value="allow">Allow</option>
            <option value="ask">Ask</option>
            <option value="deny">Deny</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="permission-subject-filter">Subject</label>
          <select
            id="permission-subject-filter"
            :value="props.permissionSubjectFilter"
            class="select"
            @change="
              emit(
                'update:permissionSubjectFilter',
                ($event.target as HTMLSelectElement).value as PermissionSubjectFilter,
              )
            "
          >
            <option value="all">All</option>
            <option value="tool">Tool</option>
            <option value="path_access">Path</option>
            <option value="network_access">Network</option>
          </select>
        </div>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Policy Editor</p>
          <h3 class="settings-panel-title">{{ props.editingPermissionRuleId ? 'Edit Rule' : 'Create Rule' }}</h3>
        </div>
        <span class="badge" :class="props.editingPermissionRuleId ? 'warn' : 'neutral'">
          {{ props.editingPermissionRuleId ? `#${props.editingPermissionRuleId}` : 'New' }}
        </span>
      </div>

      <div class="form-grid">
        <div class="field">
          <label class="label" for="permission-subject-kind">Subject</label>
          <select id="permission-subject-kind" v-model="props.permissionDraft.subjectKind" class="select">
            <option value="tool">Tool</option>
            <option value="path_access">Path</option>
            <option value="network_access">Network</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="permission-mode">Decision</label>
          <select id="permission-mode" v-model="props.permissionDraft.mode" class="select">
            <option value="allow">Allow</option>
            <option value="ask">Ask</option>
            <option value="deny">Deny</option>
          </select>
        </div>

        <template v-if="props.permissionDraft.subjectKind === 'tool'">
          <div class="field">
            <label class="label" for="permission-tool-name">Tool</label>
            <input
              id="permission-tool-name"
              v-model="props.permissionDraft.toolName"
              class="input mono"
              placeholder="bash"
            />
          </div>
          <div class="field">
            <label class="label" for="permission-qualifier">Qualifier</label>
            <input
              id="permission-qualifier"
              v-model="props.permissionDraft.qualifier"
              class="input mono"
              placeholder="git status *"
            />
          </div>
        </template>

        <template v-else-if="props.permissionDraft.subjectKind === 'path_access'">
          <div class="field">
            <label class="label" for="permission-path-access-kind">Access</label>
            <select id="permission-path-access-kind" v-model="props.permissionDraft.pathAccessKind" class="select">
              <option value="read">Read</option>
              <option value="write">Write</option>
              <option value="external_directory">External Directory</option>
            </select>
          </div>
          <div class="field">
            <label class="label" for="permission-target-path">Target Path</label>
            <input
              id="permission-target-path"
              v-model="props.permissionDraft.targetPath"
              class="input mono"
              placeholder="src/**"
            />
          </div>
          <div class="field full">
            <label class="label" for="permission-workspace-root">Workspace Root Override</label>
            <input
              id="permission-workspace-root"
              v-model="props.permissionDraft.workspaceRoot"
              class="input mono"
              placeholder="/repo"
            />
          </div>
        </template>

        <template v-else>
          <div class="field">
            <label class="label" for="permission-network-target">Network Target</label>
            <input
              id="permission-network-target"
              v-model="props.permissionDraft.networkTarget"
              class="input mono"
              placeholder="api.example.com"
            />
          </div>
          <div class="field">
            <label class="label" for="permission-network-port">Port</label>
            <input
              id="permission-network-port"
              v-model="props.permissionDraft.networkPort"
              class="input mono"
              inputmode="numeric"
              placeholder="443"
            />
          </div>
        </template>

        <div class="field">
          <label class="label" for="permission-scope">Scope</label>
          <select id="permission-scope" v-model="props.permissionDraft.scope" class="select">
            <option value="workspace">Workspace</option>
            <option value="session">Session</option>
            <option value="global">Global</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="permission-session-id">Session ID</label>
          <input
            id="permission-session-id"
            v-model="props.permissionDraft.sessionId"
            class="input mono"
            placeholder="required for session"
            :disabled="props.permissionDraft.scope !== 'session'"
          />
        </div>
      </div>

      <div class="button-row">
        <button class="button primary" @click="props.savePermissionRule">
          {{ props.editingPermissionRuleId ? 'Update Rule' : 'Create Rule' }}
        </button>
        <button class="button" @click="props.resetPermissionDraft">Reset</button>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Policy Table</p>
          <h3 class="settings-panel-title">Rules</h3>
        </div>
        <span class="badge neutral">{{ props.filteredPermissionRules.length }}</span>
      </div>

      <div v-if="props.filteredPermissionRules.length" class="record-list">
        <article
          v-for="rule in props.filteredPermissionRules"
          :key="rule.id"
          class="record-card"
          :class="{ revoked: Boolean(rule.revoked_at) }"
        >
          <div class="record-header">
            <div>
              <h4 class="record-title mono">{{ props.permissionRuleLabel(rule) }}</h4>
              <div class="record-subtitle">{{ props.permissionRulePreview(rule) }}</div>
              <div class="record-subtitle mono">{{ props.permissionRuleFacts(rule).join(' · ') }}</div>
            </div>
            <div class="record-meta">
              <span class="badge" :class="modeBadgeClass(rule.mode)">{{ rule.mode }}</span>
              <span class="badge neutral">{{ subjectLabel(rule.subject_kind) }}</span>
              <span class="badge neutral">{{ rule.scope }}</span>
              <span class="badge" :class="statusBadgeClass(rule)">{{ rule.revoked_at ? 'revoked' : 'active' }}</span>
            </div>
          </div>

          <div class="record-header">
            <span class="muted">Updated {{ rule.updated_at }}</span>
            <div class="button-row">
              <button class="button" :disabled="Boolean(rule.revoked_at)" @click="props.editPermissionRule(rule)">
                Edit
              </button>
              <button
                class="button danger"
                :disabled="Boolean(rule.revoked_at)"
                @click="props.revokePermissionRuleAction(rule)"
              >
                Revoke
              </button>
              <button class="button danger" @click="props.deletePermissionRuleAction(rule)">Delete</button>
            </div>
          </div>
        </article>
      </div>

      <div v-else class="empty-state">No permission rules matched the current filters.</div>
    </section>
  </div>
</template>
