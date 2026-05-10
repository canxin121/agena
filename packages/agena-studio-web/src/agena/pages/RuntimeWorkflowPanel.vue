<script setup lang="ts">
import {
  permissionActionView,
  permissionExplainability,
  permissionReplyPreview,
  permissionRiskLabel,
} from '@/agena/lib/permissionFormatting'
import type { SessionExecutionResource, SessionResource, WorkspaceResource } from '@/agena/lib/agenaApi'

const props = defineProps<{
  selectedWorkspaceId: number | null
  selectedSessionId: number | null
  workspaces: WorkspaceResource[]
  sessions: SessionResource[]
  executionFacts: Array<{ label: string; value: string; mono?: boolean }>
  workflowLoading: boolean
  sessionExecution: SessionExecutionResource | null
  timelineSummaries: Array<{ key: string; kind: string; summary: string; sessionId: string; timestamp: string }>
  openSelectedSessionInChat: () => void
  selectWorkspace: (workspaceId: number) => void | Promise<void>
  selectSession: (sessionId: number) => void | Promise<void>
  approvePermission: (
    requestId: string,
    kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always',
    scope?: 'session' | 'workspace' | 'global',
  ) => void | Promise<void>
}>()
</script>

<template>
  <div class="grid two">
    <section class="card">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>Workflow Inspector</h3>
          <p class="muted">Observe real session execution state without leaving the runtime page.</p>
        </div>
        <button class="button ghost" :disabled="!props.selectedSessionId" @click="props.openSelectedSessionInChat">
          Open in Chat
        </button>
      </div>

      <div class="grid two" style="margin-top: 12px">
        <div class="field">
          <label class="label" for="workflow-workspace">Workspace</label>
          <select
            id="workflow-workspace"
            :value="props.selectedWorkspaceId ?? ''"
            class="select"
            @change="props.selectWorkspace(Number(($event.target as HTMLSelectElement).value))"
          >
            <option v-for="workspace in props.workspaces" :key="workspace.id" :value="workspace.id">
              #{{ workspace.id }} · {{ workspace.path }}
            </option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="workflow-session">Session</label>
          <select
            id="workflow-session"
            :value="props.selectedSessionId ?? ''"
            class="select"
            @change="props.selectSession(Number(($event.target as HTMLSelectElement).value))"
          >
            <option v-for="session in props.sessions" :key="session.id" :value="session.id">
              #{{ session.id }} · {{ session.title }}
            </option>
          </select>
        </div>
      </div>

      <div v-if="props.executionFacts.length" class="stack" style="margin-top: 12px">
        <div v-for="fact in props.executionFacts" :key="fact.label">
          <strong>{{ fact.label }}:</strong>
          <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
        </div>
      </div>
      <p v-else-if="props.workflowLoading" class="muted" style="margin-top: 12px">Loading execution state…</p>
      <p v-else class="muted" style="margin-top: 12px">Select a session to inspect workflow execution state.</p>
    </section>

    <section class="card">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>Pending Permissions</h3>
          <p class="muted">Approve or deny pending requests directly from the runtime workflow inspector.</p>
        </div>
        <span class="badge">{{ props.sessionExecution?.pending_permission_requests.length || 0 }}</span>
      </div>
      <div v-if="props.sessionExecution?.pending_permission_requests?.length" class="list">
        <div v-for="request in props.sessionExecution.pending_permission_requests" :key="request.request_id" class="list-item">
          <div>
            <strong>{{ permissionActionView(request.action).title }}</strong>
          </div>
          <div class="muted mono">request_id={{ request.request_id }}</div>
          <div class="muted">{{ request.reason }}</div>
          <div class="muted">risk={{ permissionRiskLabel(request.action) }}</div>
          <div v-if="request.explanation" class="muted">{{ request.explanation }}</div>
          <div v-if="permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator }).summary" class="muted">
            {{ permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator }).summary }}
          </div>
          <div v-if="permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator }).details.length" class="muted mono">
            {{ permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator }).details.join(' · ') }}
          </div>
          <div class="muted mono">{{ permissionActionView(request.action).details.join(' · ') }}</div>
          <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
            <button class="button primary" @click="props.approvePermission(request.request_id, 'allow_once')">Allow Once</button>
            <button class="button" @click="props.approvePermission(request.request_id, 'allow_always', 'session')">Allow Always (Session)</button>
            <button class="button" @click="props.approvePermission(request.request_id, 'allow_always', 'workspace')">Allow Always (Workspace)</button>
            <button class="button" @click="props.approvePermission(request.request_id, 'allow_always', 'global')">Allow Always (Global)</button>
            <button class="button danger" @click="props.approvePermission(request.request_id, 'deny_once')">Deny Once</button>
            <button class="button danger" @click="props.approvePermission(request.request_id, 'deny_always', 'session')">Deny Always (Session)</button>
            <button class="button danger" @click="props.approvePermission(request.request_id, 'deny_always', 'workspace')">Deny Always (Workspace)</button>
            <button class="button danger" @click="props.approvePermission(request.request_id, 'deny_always', 'global')">Deny Always (Global)</button>
          </div>
          <div class="muted">
            once={{ permissionReplyPreview() }} · session={{ permissionReplyPreview('session') }} · workspace={{ permissionReplyPreview('workspace') }} · global={{ permissionReplyPreview('global') }}
          </div>
        </div>
      </div>
      <p v-else-if="props.workflowLoading" class="muted">Loading pending permissions…</p>
      <p v-else class="muted">No pending permission requests for the selected session.</p>
    </section>

    <section class="card">
      <h3>Recent Timeline</h3>
      <div v-if="props.timelineSummaries.length" class="list">
        <div v-for="event in props.timelineSummaries" :key="event.key" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div><strong>{{ event.kind }}</strong></div>
              <div class="muted">{{ event.summary }}</div>
              <div class="muted">{{ event.sessionId }}</div>
            </div>
            <span class="badge">{{ event.timestamp }}</span>
          </div>
        </div>
      </div>
      <p v-else-if="props.workflowLoading" class="muted">Loading session timeline…</p>
      <p v-else class="muted">No timeline events loaded yet.</p>
    </section>
  </div>
</template>
