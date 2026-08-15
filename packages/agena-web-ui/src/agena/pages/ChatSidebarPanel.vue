<script setup lang="ts">
import type { SessionResource, WorkspaceResource } from '@/agena/lib/agenaApi'

const props = defineProps<{
  loading: boolean
  workspacePath: string
  workspaces: WorkspaceResource[]
  selectedWorkspaceId: number | null
  sessionSearch: string
  sessionViewMode: 'all' | 'roots' | 'subtree'
  newSessionTitle: string
  sessions: SessionResource[]
  selectedSessionId: number | null
  resolveWorkspaceAction: (createIfMissing: boolean) => void | Promise<void>
  selectWorkspace: (workspaceId: number) => void | Promise<void>
  loadSessionsForWorkspace: (workspaceId: number, preserveSelection?: boolean) => void | Promise<void>
  setSessionViewMode: (mode: 'all' | 'roots' | 'subtree', query?: string) => void | Promise<void>
  createSessionAction: () => void | Promise<void>
  selectSession: (sessionId: number) => void | Promise<void>
  formatMessageTime: (value: string) => string
}>()

const emit = defineEmits<{
  'update:workspacePath': [value: string]
  'update:sessionSearch': [value: string]
  'update:newSessionTitle': [value: string]
}>()
</script>

<template>
  <aside class="stack">
    <section class="card">
      <h3>Workspace</h3>
      <div class="field">
        <label class="label" for="workspace-path">Path</label>
        <input
          id="workspace-path"
          :value="props.workspacePath"
          class="input mono"
          placeholder="D:/git/ai/project"
          @input="emit('update:workspacePath', ($event.target as HTMLInputElement).value)"
        />
      </div>
      <div class="button-row" style="margin-top: 12px">
        <button
          class="button primary"
          :disabled="props.loading || !props.workspacePath.trim()"
          @click="props.resolveWorkspaceAction(true)"
        >
          Resolve or Create
        </button>
        <button
          class="button"
          :disabled="props.loading || !props.workspacePath.trim()"
          @click="props.resolveWorkspaceAction(false)"
        >
          Create Only
        </button>
      </div>
    </section>

    <section class="card">
      <h3>Workspaces</h3>
      <div v-if="props.workspaces.length" class="list">
        <button
          v-for="workspace in props.workspaces"
          :key="workspace.id"
          class="list-item"
          :class="{ active: workspace.id === props.selectedWorkspaceId }"
          @click="props.selectWorkspace(workspace.id)"
        >
          <div>
            <strong>{{ workspace.path }}</strong>
          </div>
          <div class="muted">{{ workspace.session_count ?? 0 }} session(s)</div>
        </button>
      </div>
      <p v-else class="muted">No workspaces yet.</p>
    </section>

    <section class="card">
      <h3>Sessions</h3>
      <div class="field">
        <label class="label" for="session-view-mode">View</label>
        <select
          id="session-view-mode"
          :value="props.sessionViewMode"
          class="select"
          @change="
            props.setSessionViewMode(
              ($event.target as HTMLSelectElement).value as 'all' | 'roots' | 'subtree',
              props.sessionSearch,
            )
          "
        >
          <option value="all">All sessions</option>
          <option value="roots">Root sessions</option>
          <option value="subtree" :disabled="!props.selectedSessionId">Selected subtree</option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="session-search">Search</label>
        <input
          id="session-search"
          :value="props.sessionSearch"
          class="input"
          placeholder="search sessions"
          @input="emit('update:sessionSearch', ($event.target as HTMLInputElement).value)"
          @keyup.enter="props.selectedWorkspaceId && props.loadSessionsForWorkspace(props.selectedWorkspaceId, false)"
        />
      </div>
      <div class="field">
        <label class="label" for="session-title">Title</label>
        <input
          id="session-title"
          :value="props.newSessionTitle"
          class="input"
          placeholder="New session"
          @input="emit('update:newSessionTitle', ($event.target as HTMLInputElement).value)"
        />
      </div>
      <div class="button-row" style="margin-top: 12px">
        <button
          class="button primary"
          :disabled="!props.selectedWorkspaceId || props.loading"
          @click="props.createSessionAction"
        >
          Create Session
        </button>
      </div>
      <div v-if="props.sessions.length" class="list" style="margin-top: 14px">
        <button
          v-for="session in props.sessions"
          :key="session.id"
          class="list-item"
          :class="{ active: session.id === props.selectedSessionId }"
          @click="props.selectSession(session.id)"
        >
          <div>
            <strong>{{ session.title }}</strong>
            <span class="badge" style="margin-left: 8px">{{ session.state.replace('_', ' ') }}</span>
          </div>
          <div class="muted">
            {{ session.message_count }} message(s) · updated {{ props.formatMessageTime(session.updated_at) }}
          </div>
        </button>
      </div>
      <p v-else class="muted" style="margin-top: 14px">No sessions in the selected workspace.</p>
    </section>
  </aside>
</template>
