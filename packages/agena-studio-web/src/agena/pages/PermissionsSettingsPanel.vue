<script setup lang="ts">
import type { PermissionMode, PermissionRuleResource } from '@/agena/lib/agenaApi'

const props = defineProps<{
  loading: boolean
  permissionSearch: string
  permissionStatusFilter: 'all' | 'active' | 'revoked'
  permissionScopeFilter: 'all' | 'session' | 'workspace' | 'global'
  permissionModeFilter: 'all' | PermissionMode
  permissionSubjectFilter: 'all' | 'builtin_tool' | 'path_access'
  permissionDraft: {
    subjectKind: 'builtin_tool' | 'path_access'
    toolName: string
    qualifier: string
    pathAccessKind: string
    workspaceRoot: string
    targetPath: string
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
  'update:permissionSubjectFilter': [value: 'all' | 'builtin_tool' | 'path_access']
}>()
</script>

<template>
  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3>Permission Rules</h3>
        <p class="muted">Persist allow / ask / deny decisions as structured tool/path rules with scope and source metadata.</p>
      </div>
      <button class="button ghost" :disabled="props.loading" @click="props.load">Refresh</button>
    </div>

    <div class="field">
      <label class="label" for="permission-search">Search</label>
      <input
        id="permission-search"
        :value="props.permissionSearch"
        class="input mono"
        placeholder="Bash:ls"
        @input="emit('update:permissionSearch', ($event.target as HTMLInputElement).value)"
        @keyup.enter="props.load"
      />
    </div>

    <div class="grid two" style="margin-top: 12px">
      <div class="field">
        <label class="label" for="permission-status-filter">Status</label>
        <select
          id="permission-status-filter"
          :value="props.permissionStatusFilter"
          class="select"
          @change="emit('update:permissionStatusFilter', ($event.target as HTMLSelectElement).value as 'all' | 'active' | 'revoked')"
        >
          <option value="active">active</option>
          <option value="revoked">revoked</option>
          <option value="all">all</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-scope-filter">Scope</label>
        <select
          id="permission-scope-filter"
          :value="props.permissionScopeFilter"
          class="select"
          @change="emit('update:permissionScopeFilter', ($event.target as HTMLSelectElement).value as 'all' | 'session' | 'workspace' | 'global')"
        >
          <option value="all">all</option>
          <option value="workspace">workspace</option>
          <option value="session">session</option>
          <option value="global">global</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-mode-filter">Mode</label>
        <select
          id="permission-mode-filter"
          :value="props.permissionModeFilter"
          class="select"
          @change="emit('update:permissionModeFilter', ($event.target as HTMLSelectElement).value as 'all' | PermissionMode)"
        >
          <option value="all">all</option>
          <option value="allow">allow</option>
          <option value="ask">ask</option>
          <option value="deny">deny</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-subject-filter">Subject</label>
        <select
          id="permission-subject-filter"
          :value="props.permissionSubjectFilter"
          class="select"
          @change="emit('update:permissionSubjectFilter', ($event.target as HTMLSelectElement).value as 'all' | 'builtin_tool' | 'path_access')"
        >
          <option value="all">all</option>
          <option value="builtin_tool">builtin_tool</option>
          <option value="path_access">path_access</option>
        </select>
      </div>
    </div>

    <div class="grid two" style="margin-top: 12px">
      <div class="field">
        <label class="label" for="permission-subject-kind">Subject</label>
        <select id="permission-subject-kind" v-model="props.permissionDraft.subjectKind" class="select">
          <option value="builtin_tool">builtin_tool</option>
          <option value="path_access">path_access</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-mode">Mode</label>
        <select id="permission-mode" v-model="props.permissionDraft.mode" class="select">
          <option value="allow">allow</option>
          <option value="ask">ask</option>
          <option value="deny">deny</option>
        </select>
      </div>
    </div>

    <div v-if="props.permissionDraft.subjectKind === 'builtin_tool'" class="grid two" style="margin-top: 12px">
      <div class="field">
        <label class="label" for="permission-tool-name">Tool Name</label>
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
    </div>

    <div v-else class="grid two" style="margin-top: 12px">
      <div class="field">
        <label class="label" for="permission-path-access-kind">Path Access</label>
        <select id="permission-path-access-kind" v-model="props.permissionDraft.pathAccessKind" class="select">
          <option value="read">read</option>
          <option value="write">write</option>
          <option value="external_directory">external_directory</option>
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
      <div class="field" style="grid-column: 1 / -1">
        <label class="label" for="permission-workspace-root">Workspace Root Override</label>
        <input
          id="permission-workspace-root"
          v-model="props.permissionDraft.workspaceRoot"
          class="input mono"
          placeholder="optional workspace root override"
        />
      </div>
    </div>

    <div class="grid two" style="margin-top: 12px">
      <div class="field">
        <label class="label" for="permission-scope">Scope</label>
        <select id="permission-scope" v-model="props.permissionDraft.scope" class="select">
          <option value="workspace">workspace</option>
          <option value="session">session</option>
          <option value="global">global</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="permission-session-id">Session ID</label>
        <input
          id="permission-session-id"
          v-model="props.permissionDraft.sessionId"
          class="input mono"
          placeholder="required for session scope"
          :disabled="props.permissionDraft.scope !== 'session'"
        />
      </div>
    </div>

    <div class="button-row" style="margin-top: 12px">
      <button class="button primary" @click="props.savePermissionRule">
        {{ props.editingPermissionRuleId ? 'Update Rule' : 'Create Rule' }}
      </button>
      <button class="button" @click="props.resetPermissionDraft">Reset</button>
    </div>

    <div v-if="props.filteredPermissionRules.length" class="list" style="margin-top: 12px">
      <div v-for="rule in props.filteredPermissionRules" :key="rule.id" class="list-item">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <strong class="mono">{{ props.permissionRuleLabel(rule) }}</strong>
            <div class="muted">{{ props.permissionRulePreview(rule) }}</div>
            <div class="muted">updated {{ rule.updated_at }}</div>
            <div class="muted mono">{{ props.permissionRuleFacts(rule).join(' · ') }}</div>
          </div>
          <div class="button-row">
            <span class="badge">{{ rule.mode }}</span>
            <span class="badge">{{ rule.revoked_at ? 'revoked' : 'active' }}</span>
          </div>
        </div>
        <div class="button-row" style="margin-top: 10px">
          <button class="button" :disabled="Boolean(rule.revoked_at)" @click="props.editPermissionRule(rule)">Edit</button>
          <button class="button danger" :disabled="Boolean(rule.revoked_at)" @click="props.revokePermissionRuleAction(rule)">
            Revoke
          </button>
          <button class="button danger" @click="props.deletePermissionRuleAction(rule)">Delete</button>
        </div>
      </div>
    </div>
    <p v-else class="muted" style="margin-top: 12px">No permission rules matched the current filters.</p>
  </section>
</template>
