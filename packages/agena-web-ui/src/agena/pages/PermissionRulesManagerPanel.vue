<script setup lang="ts">
import { onMounted } from 'vue'
import { useRoute } from 'vue-router'
import type { useSettingsPageState } from './useSettingsPageState'

const props = defineProps<{
  permissions: ReturnType<typeof useSettingsPageState>['panels']['permissions']
}>()
const route = useRoute()

onMounted(() => {
  if (route.query.mode !== 'new' || typeof document === 'undefined') return
  document.getElementById('permission-rule-editor')?.scrollIntoView({ behavior: 'smooth', block: 'start' })
})

function draftIsValid(): boolean {
  const draft = props.permissions.permissionDraft
  if (draft.subjectKind === 'tool') return Boolean(draft.toolName.trim())
  if (draft.subjectKind === 'path_access') return Boolean(draft.targetPath.trim())
  return Boolean(draft.networkTarget.trim())
}

async function revokeRule(rule: Parameters<typeof props.permissions.revokePermissionRuleAction>[0]) {
  if (typeof window !== 'undefined' && !window.confirm(`Revoke rule #${rule.id}?`)) return
  await props.permissions.revokePermissionRuleAction(rule)
}

async function deleteRule(rule: Parameters<typeof props.permissions.deletePermissionRuleAction>[0]) {
  if (typeof window !== 'undefined' && !window.confirm(`Permanently delete rule #${rule.id}?`)) return
  await props.permissions.deletePermissionRuleAction(rule)
}
</script>

<template>
  <section id="permission-rules-manager" class="settings-panel">
    <div class="settings-panel-header">
      <div>
        <p class="settings-panel-kicker">Effective runtime rules</p>
        <h3 class="settings-panel-title">Permission Rule Manager</h3>
        <p class="record-subtitle">
          Create scoped tool, filesystem, and network decisions; edit, revoke, or delete existing rules.
        </p>
      </div>
      <span class="badge neutral">{{ props.permissions.filteredPermissionRules.value.length }} visible</span>
    </div>

    <div class="form-grid">
      <div class="field full">
        <label class="label" for="permission-rule-search">Search rules</label>
        <input
          id="permission-rule-search"
          v-model="props.permissions.permissionSearch.value"
          class="input"
          placeholder="tool, qualifier, path, host, scope, or source"
        />
      </div>
      <div class="field">
        <label class="label" for="permission-mode-filter">Mode</label>
        <select id="permission-mode-filter" v-model="props.permissions.permissionModeFilter.value" class="select">
          <option value="all">All modes</option>
          <option value="allow">Allow</option>
          <option value="auto">Auto</option>
          <option value="ask">Ask</option>
          <option value="deny">Deny</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-scope-filter">Scope</label>
        <select id="permission-scope-filter" v-model="props.permissions.permissionScopeFilter.value" class="select">
          <option value="all">All scopes</option>
          <option value="session">Session</option>
          <option value="workspace">Workspace</option>
          <option value="global">Global</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-subject-filter">Subject</label>
        <select id="permission-subject-filter" v-model="props.permissions.permissionSubjectFilter.value" class="select">
          <option value="all">All subjects</option>
          <option value="tool">Tool</option>
          <option value="path_access">Path access</option>
          <option value="network_access">Network access</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-status-filter">Status</label>
        <select id="permission-status-filter" v-model="props.permissions.permissionStatusFilter.value" class="select">
          <option value="active">Active</option>
          <option value="revoked">Revoked</option>
          <option value="all">All statuses</option>
        </select>
      </div>
    </div>
  </section>

  <section id="permission-rule-editor" class="settings-panel">
    <div class="settings-panel-header">
      <div>
        <p class="settings-panel-kicker">
          {{ props.permissions.editingPermissionRuleId.value ? 'Edit rule' : 'New rule' }}
        </p>
        <h3 class="settings-panel-title">
          {{
            props.permissions.editingPermissionRuleId.value
              ? `Rule #${props.permissions.editingPermissionRuleId.value}`
              : 'Rule Draft'
          }}
        </h3>
      </div>
      <button class="button ghost" @click="props.permissions.resetPermissionDraft">Reset draft</button>
    </div>

    <div class="form-grid">
      <div class="field">
        <label class="label" for="permission-rule-subject">Subject kind</label>
        <select id="permission-rule-subject" v-model="props.permissions.permissionDraft.subjectKind" class="select">
          <option value="tool">Tool</option>
          <option value="path_access">Path access</option>
          <option value="network_access">Network access</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-rule-mode">Decision</label>
        <select id="permission-rule-mode" v-model="props.permissions.permissionDraft.mode" class="select">
          <option value="allow">Allow</option>
          <option value="auto">Auto</option>
          <option value="ask">Ask</option>
          <option value="deny">Deny</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-rule-scope">Scope</label>
        <select id="permission-rule-scope" v-model="props.permissions.permissionDraft.scope" class="select">
          <option value="session">Session</option>
          <option value="workspace">Workspace</option>
          <option value="global">Global</option>
        </select>
      </div>
      <div v-if="props.permissions.permissionDraft.scope === 'session'" class="field">
        <label class="label" for="permission-rule-session">Session id</label>
        <input
          id="permission-rule-session"
          v-model="props.permissions.permissionDraft.sessionId"
          class="input mono"
          inputmode="numeric"
        />
      </div>

      <template v-if="props.permissions.permissionDraft.subjectKind === 'tool'">
        <div class="field">
          <label class="label" for="permission-rule-tool">Tool name</label>
          <input
            id="permission-rule-tool"
            v-model="props.permissions.permissionDraft.toolName"
            class="input mono"
            placeholder="exec_command"
          />
        </div>
        <div class="field">
          <label class="label" for="permission-rule-qualifier">Qualifier</label>
          <input
            id="permission-rule-qualifier"
            v-model="props.permissions.permissionDraft.qualifier"
            class="input mono"
            placeholder="optional command or operation qualifier"
          />
        </div>
      </template>

      <template v-else-if="props.permissions.permissionDraft.subjectKind === 'path_access'">
        <div class="field">
          <label class="label" for="permission-rule-path-kind">Access kind</label>
          <select
            id="permission-rule-path-kind"
            v-model="props.permissions.permissionDraft.pathAccessKind"
            class="select"
          >
            <option value="read">Read</option>
            <option value="write">Write</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="permission-rule-root">Workspace root</label>
          <input
            id="permission-rule-root"
            v-model="props.permissions.permissionDraft.workspaceRoot"
            class="input mono"
            placeholder="optional workspace root"
          />
        </div>
        <div class="field full">
          <label class="label" for="permission-rule-path">Target path</label>
          <input
            id="permission-rule-path"
            v-model="props.permissions.permissionDraft.targetPath"
            class="input mono"
            placeholder="relative or absolute target path"
          />
        </div>
      </template>

      <template v-else>
        <div class="field">
          <label class="label" for="permission-rule-network">Network target</label>
          <input
            id="permission-rule-network"
            v-model="props.permissions.permissionDraft.networkTarget"
            class="input mono"
            placeholder="host, domain, or network target"
          />
        </div>
        <div class="field">
          <label class="label" for="permission-rule-port">Port</label>
          <input
            id="permission-rule-port"
            v-model="props.permissions.permissionDraft.networkPort"
            class="input mono"
            inputmode="numeric"
            placeholder="optional"
          />
        </div>
      </template>
    </div>

    <div class="button-row">
      <button class="button primary" :disabled="!draftIsValid()" @click="props.permissions.savePermissionRule">
        {{ props.permissions.editingPermissionRuleId.value ? 'Update Rule' : 'Create Rule' }}
      </button>
      <button class="button" @click="props.permissions.resetPermissionDraft">Cancel Edit</button>
    </div>
  </section>

  <section class="settings-panel">
    <div class="settings-panel-header">
      <div>
        <p class="settings-panel-kicker">Persisted decisions</p>
        <h3 class="settings-panel-title">Rules</h3>
      </div>
    </div>
    <div v-if="props.permissions.filteredPermissionRules.value.length" class="record-list">
      <article v-for="rule in props.permissions.filteredPermissionRules.value" :key="rule.id" class="record-card">
        <div class="record-header">
          <div>
            <h4 class="record-title">#{{ rule.id }} · {{ props.permissions.permissionRuleLabel(rule) }}</h4>
            <p class="record-subtitle mono">{{ props.permissions.permissionRulePreview(rule) }}</p>
            <p class="muted mono">{{ props.permissions.permissionRuleFacts(rule).join(' · ') }}</p>
          </div>
          <div class="record-meta">
            <span class="badge" :class="rule.mode === 'allow' ? 'success' : rule.mode === 'deny' ? 'danger' : 'warn'">{{
              rule.mode
            }}</span>
            <span v-if="rule.revoked_at" class="badge neutral">revoked</span>
            <button class="button" @click="props.permissions.editPermissionRule(rule)">Edit</button>
            <button v-if="!rule.revoked_at" class="button warn" @click="revokeRule(rule)">Revoke</button>
            <button class="button danger" @click="deleteRule(rule)">Delete</button>
          </div>
        </div>
      </article>
    </div>
    <div v-else class="empty-state">No permission rules match the current filters.</div>
  </section>
</template>
