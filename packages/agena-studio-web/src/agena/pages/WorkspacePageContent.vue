<script setup lang="ts">
import type { WorkspaceResource } from '@/agena/lib/agenaApi'

import type { useWorkspacePageState } from './useWorkspacePageState'

const props = defineProps<{
  workspace: ReturnType<typeof useWorkspacePageState>
}>()

function workspaceBadge(workspace: WorkspaceResource) {
  return props.workspace.selectedWorkspaceId.value === workspace.id ? 'selected' : 'workspace'
}
</script>

<template>
  <div class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Resolve Workspace</p>
          <h3 class="settings-panel-title">Workspace Manager</h3>
        </div>
        <span class="badge neutral">{{ props.workspace.workspaces.value.length }} workspaces</span>
      </div>

      <div class="settings-summary">
        <div class="summary-item">
          <div class="summary-label">Selected</div>
          <div class="summary-value">#{{ props.workspace.selectedWorkspaceId.value || 'none' }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Sessions</div>
          <div class="summary-value">{{ props.workspace.selectedWorkspace.value?.session_count ?? 0 }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Path</div>
          <div class="summary-value mono">{{ props.workspace.pathInput.value || '/' }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Entries</div>
          <div class="summary-value">{{ props.workspace.rows.value.length }}</div>
        </div>
      </div>

      <div class="inline-fields">
        <div class="field">
          <label class="label" for="workspace-root-input">Workspace Path</label>
          <input
            id="workspace-root-input"
            v-model="props.workspace.workspacePath.value"
            class="input mono"
            placeholder="/path/to/repo"
            @keyup.enter="props.workspace.resolveWorkspaceAction(true)"
          />
        </div>
        <div class="button-row">
          <button
            class="button primary"
            :disabled="props.workspace.loading.value || !props.workspace.workspacePath.value.trim()"
            @click="props.workspace.resolveWorkspaceAction(true)"
          >
            Resolve or Create
          </button>
          <button
            class="button"
            :disabled="props.workspace.loading.value || !props.workspace.workspacePath.value.trim()"
            @click="props.workspace.resolveWorkspaceAction(false)"
          >
            Create Only
          </button>
          <button
            class="button"
            :disabled="!props.workspace.selectedWorkspace.value"
            @click="props.workspace.useSelectedWorkspaceAsResolverPath"
          >
            Use Current Path
          </button>
        </div>
      </div>
    </section>

    <section id="workspace-snapshots" class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Isolated worktrees</p>
          <h3 class="settings-panel-title">Session Snapshots</h3>
          <p class="record-subtitle">
            Inspect active session ownership, managed directories, and Git/Rift backend readiness.
          </p>
        </div>
        <div class="record-meta">
          <span class="badge neutral">{{ props.workspace.snapshotStatus.value?.active.length || 0 }} active</span>
          <span class="badge neutral">{{ props.workspace.snapshotStatus.value?.managed.length || 0 }} managed</span>
        </div>
      </div>

      <template v-if="props.workspace.snapshotStatus.value">
        <div class="facts-grid">
          <div class="fact-row">
            <div class="fact-label">Preferred backend</div>
            <div class="fact-value mono">{{ props.workspace.snapshotStatus.value.preferred_backend || 'none' }}</div>
          </div>
          <div class="fact-row">
            <div class="fact-label">Git backend</div>
            <div class="fact-value">
              {{ props.workspace.snapshotStatus.value.git.available ? 'available' : 'unavailable' }} ·
              {{ props.workspace.snapshotStatus.value.git.detail }}
            </div>
          </div>
          <div class="fact-row">
            <div class="fact-label">Rift backend</div>
            <div class="fact-value">
              {{ props.workspace.snapshotStatus.value.rift.available ? 'available' : 'unavailable' }} ·
              {{ props.workspace.snapshotStatus.value.rift.detail }}
            </div>
          </div>
          <div class="fact-row">
            <div class="fact-label">Registry</div>
            <div class="fact-value">
              runtime={{ props.workspace.snapshotStatus.value.session_runtime_available ? 'ready' : 'unavailable' }} ·
              registry={{ props.workspace.snapshotStatus.value.registry_available ? 'ready' : 'unavailable' }}
            </div>
          </div>
        </div>

        <div v-if="props.workspace.snapshotStatus.value.active.length" class="record-list">
          <article
            v-for="snapshot in props.workspace.snapshotStatus.value.active"
            :key="snapshot.session_id"
            class="record-card"
          >
            <div class="record-header">
              <div>
                <h4 class="record-title">Session #{{ snapshot.session_id }} · {{ snapshot.branch }}</h4>
                <p class="record-subtitle mono">{{ snapshot.path }}</p>
              </div>
              <div class="record-meta">
                <span class="badge neutral">{{ snapshot.backend }}</span>
                <span class="badge" :class="snapshot.created_here ? 'success' : 'neutral'">
                  {{ snapshot.created_here ? 'managed here' : 'attached' }}
                </span>
              </div>
            </div>
          </article>
        </div>

        <details v-if="props.workspace.snapshotStatus.value.managed.length" class="record-card">
          <summary>All managed snapshot directories</summary>
          <div class="record-list">
            <article
              v-for="snapshot in props.workspace.snapshotStatus.value.managed"
              :key="snapshot.path"
              class="record-card"
            >
              <div class="record-header">
                <div>
                  <h4 class="record-title mono">{{ snapshot.path }}</h4>
                  <p class="record-subtitle">
                    session={{ snapshot.session_id ?? 'none' }} · branch={{ snapshot.branch || 'unknown' }} · git={{
                      snapshot.registered_with_git ? 'yes' : 'no'
                    }}
                    · rift={{ snapshot.registered_with_rift ? 'yes' : 'no' }}
                  </p>
                </div>
                <span class="badge" :class="snapshot.stale ? 'danger' : 'neutral'">{{
                  snapshot.stale ? 'stale' : snapshot.backend || 'unknown'
                }}</span>
              </div>
            </article>
          </div>
        </details>

        <div
          v-if="
            !props.workspace.snapshotStatus.value.active.length && !props.workspace.snapshotStatus.value.managed.length
          "
          class="empty-state"
        >
          No active or managed snapshots were found.
        </div>
      </template>
      <div v-else class="empty-state">Snapshot status is not available.</div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Current Workspace</p>
          <h3 class="settings-panel-title">Select, Edit, Delete</h3>
        </div>
        <div class="button-row">
          <button
            class="button"
            :disabled="!props.workspace.selectedWorkspaceId.value"
            @click="props.workspace.openChatForWorkspace"
          >
            Open in Chat
          </button>
          <button
            class="button"
            :disabled="!props.workspace.selectedWorkspaceId.value"
            @click="props.workspace.openRuntimeForWorkspace"
          >
            Open in Runtime
          </button>
        </div>
      </div>

      <div class="form-grid">
        <div class="field">
          <label class="label" for="workspace-select">Workspace</label>
          <select
            id="workspace-select"
            v-model.number="props.workspace.selectedWorkspaceId.value"
            class="select"
            @change="props.workspace.selectWorkspace(props.workspace.selectedWorkspaceId.value || '')"
          >
            <option v-for="workspace in props.workspace.workspaces.value" :key="workspace.id" :value="workspace.id">
              #{{ workspace.id }} · {{ workspace.path }}
            </option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="workspace-edit-path">Editable Path</label>
          <input
            id="workspace-edit-path"
            v-model="props.workspace.selectedWorkspacePathDraft.value"
            class="input mono"
            placeholder="/path/to/repo"
            :disabled="!props.workspace.selectedWorkspace.value"
            @keyup.enter="props.workspace.saveSelectedWorkspacePath"
          />
        </div>
      </div>

      <div class="button-row">
        <button
          class="button primary"
          :disabled="
            !props.workspace.selectedWorkspace.value ||
            props.workspace.loading.value ||
            !props.workspace.selectedWorkspacePathDraft.value.trim()
          "
          @click="props.workspace.saveSelectedWorkspacePath"
        >
          Save Path
        </button>
        <button
          class="button"
          :disabled="!props.workspace.selectedWorkspace.value"
          @click="props.workspace.resetSelectedWorkspacePathDraft"
        >
          Reset Path
        </button>
        <button
          class="button danger"
          :disabled="!props.workspace.selectedWorkspace.value || props.workspace.loading.value"
          @click="props.workspace.deleteSelectedWorkspace"
        >
          Delete Workspace
        </button>
      </div>

      <div v-if="props.workspace.workspaces.value.length" class="record-list">
        <article v-for="workspace in props.workspace.workspaces.value" :key="workspace.id" class="record-card">
          <div class="record-header">
            <div>
              <h4 class="record-title mono">{{ workspace.path }}</h4>
              <div class="record-subtitle">
                #{{ workspace.id }} · sessions={{ workspace.session_count ?? 0 }} · updated={{ workspace.updated_at }}
              </div>
            </div>
            <div class="record-meta">
              <span
                class="badge"
                :class="props.workspace.selectedWorkspaceId.value === workspace.id ? 'success' : 'neutral'"
              >
                {{ workspaceBadge(workspace) }}
              </span>
              <button
                class="button"
                :disabled="props.workspace.loading.value"
                @click="props.workspace.selectWorkspace(workspace.id)"
              >
                Select
              </button>
              <button class="button" @click="props.workspace.workspacePath.value = workspace.path">Use Path</button>
              <button
                class="button danger"
                :disabled="props.workspace.loading.value"
                @click="props.workspace.deleteWorkspaceAction(workspace)"
              >
                Delete
              </button>
            </div>
          </div>
        </article>
      </div>
      <div v-else class="empty-state">No workspaces have been registered yet.</div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Project Status</p>
          <h3 class="settings-panel-title">Git and Diff</h3>
        </div>
        <div class="button-row">
          <button
            v-if="props.workspace.gitStatus.value && !props.workspace.gitStatus.value.repo"
            class="button"
            :disabled="props.workspace.loading.value || !props.workspace.gitStatus.value.git_available"
            @click="props.workspace.initGitProjectAction"
          >
            Initialize Git
          </button>
          <button
            v-if="props.workspace.gitStatus.value?.repo"
            class="button"
            :disabled="props.workspace.rawDiffLoading.value || props.workspace.loading.value"
            @click="props.workspace.loadVcsDiffRawAction"
          >
            {{ props.workspace.rawDiffLoading.value ? 'Loading Raw Diff...' : 'Load Raw Diff' }}
          </button>
        </div>
      </div>

      <div v-if="props.workspace.gitStatus.value" class="facts-grid">
        <div class="fact-row">
          <div class="fact-label">Root</div>
          <div class="fact-value mono">{{ props.workspace.gitStatus.value.workspace_root }}</div>
        </div>
        <div class="fact-row">
          <div class="fact-label">Git</div>
          <div class="fact-value">
            {{ props.workspace.gitStatus.value.git_available ? 'available' : 'missing' }} · repo={{
              props.workspace.gitStatus.value.repo ? 'yes' : 'no'
            }}
          </div>
        </div>
        <div class="fact-row">
          <div class="fact-label">Branch</div>
          <div class="fact-value mono">{{ props.workspace.gitStatus.value.branch || 'n/a' }}</div>
        </div>
        <div class="fact-row">
          <div class="fact-label">Changes</div>
          <div class="fact-value">
            changed={{ props.workspace.gitStatus.value.changed_files }} · staged={{
              props.workspace.gitStatus.value.staged_files
            }}
            · unstaged={{ props.workspace.gitStatus.value.unstaged_files }} · untracked={{
              props.workspace.gitStatus.value.untracked_files
            }}
          </div>
        </div>
      </div>
      <div v-else class="empty-state">Git/worktree status is not available.</div>

      <details v-if="props.workspace.rawDiffLoaded.value" class="record-card">
        <summary class="muted mono">raw_diff={{ props.workspace.rawDiff.value ? 'available' : 'empty' }}</summary>
        <pre v-if="props.workspace.rawDiff.value" class="mono raw-block">{{ props.workspace.rawDiff.value }}</pre>
        <div v-else class="muted">No tracked or untracked changes to preview.</div>
      </details>

      <div v-if="props.workspace.gitStatus.value?.repo" class="git-workflow-grid">
        <article class="record-card">
          <div class="record-header">
            <div>
              <h4 class="record-title">Stage and Commit</h4>
              <p class="record-subtitle">Stage runtime-workspace changes, then create an intentional commit.</p>
            </div>
            <span class="badge neutral">{{ props.workspace.gitStatus.value.staged_files }} staged</span>
          </div>
          <div class="field">
            <label class="label" for="workspace-commit-message">Commit message</label>
            <input
              id="workspace-commit-message"
              v-model="props.workspace.commitMessage.value"
              class="input"
              placeholder="Describe the completed change"
              @keyup.enter="props.workspace.createGitCommitAction"
            />
          </div>
          <div class="button-row">
            <button
              class="button"
              :disabled="
                props.workspace.gitActionLoading.value ||
                props.workspace.gitStatus.value.unstaged_files + props.workspace.gitStatus.value.untracked_files === 0
              "
              @click="props.workspace.stageAllGitChangesAction"
            >
              Stage All
            </button>
            <button
              class="button primary"
              :disabled="
                props.workspace.gitActionLoading.value ||
                props.workspace.gitStatus.value.staged_files === 0 ||
                !props.workspace.commitMessage.value.trim()
              "
              @click="props.workspace.createGitCommitAction"
            >
              Create Commit
            </button>
          </div>
        </article>

        <article class="record-card">
          <div class="record-header">
            <div>
              <h4 class="record-title">Create Pull Request</h4>
              <p class="record-subtitle">Use GitHub CLI with explicit title, body, base, and head values.</p>
            </div>
            <span class="badge" :class="props.workspace.gitStatus.value.gh_available ? 'success' : 'danger'">
              gh {{ props.workspace.gitStatus.value.gh_available ? 'available' : 'missing' }}
            </span>
          </div>
          <div class="form-grid">
            <div class="field full">
              <label class="label" for="workspace-pr-title">Title</label>
              <input
                id="workspace-pr-title"
                v-model="props.workspace.pullRequestTitle.value"
                class="input"
                placeholder="Pull request title"
              />
            </div>
            <div class="field">
              <label class="label" for="workspace-pr-base">Base branch</label>
              <input
                id="workspace-pr-base"
                v-model="props.workspace.pullRequestBase.value"
                class="input mono"
                placeholder="repository default"
              />
            </div>
            <div class="field">
              <label class="label" for="workspace-pr-head">Head branch</label>
              <input
                id="workspace-pr-head"
                v-model="props.workspace.pullRequestHead.value"
                class="input mono"
                :placeholder="props.workspace.gitStatus.value.branch || 'current branch'"
              />
            </div>
            <div class="field full">
              <label class="label" for="workspace-pr-body">Body</label>
              <textarea
                id="workspace-pr-body"
                v-model="props.workspace.pullRequestBody.value"
                class="textarea"
                placeholder="Summary, verification, and review notes"
              />
            </div>
          </div>
          <div class="button-row">
            <button
              class="button primary"
              :disabled="
                props.workspace.gitActionLoading.value ||
                !props.workspace.gitStatus.value.gh_available ||
                !props.workspace.pullRequestTitle.value.trim()
              "
              @click="props.workspace.createGitPullRequestAction"
            >
              Create Pull Request
            </button>
            <a
              v-if="props.workspace.pullRequestUrl.value"
              class="button ghost"
              :href="props.workspace.pullRequestUrl.value"
              target="_blank"
              rel="noopener noreferrer"
            >
              Open Pull Request
            </a>
          </div>
        </article>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Project Entry Points</p>
          <h3 class="settings-panel-title">Agena Paths</h3>
        </div>
      </div>
      <div v-if="props.workspace.configSummaryFacts.value.length" class="muted mono">
        {{ props.workspace.configSummaryFacts.value.join(' · ') }}
      </div>
      <div class="record-list">
        <article v-for="shortcut in props.workspace.workspaceConfigCards.value" :key="shortcut.id" class="record-card">
          <div class="record-header">
            <button class="button ghost record-button" @click="props.workspace.openShortcut(shortcut.relativePath)">
              <span>
                <strong>{{ shortcut.title }}</strong>
                <span class="record-subtitle">{{ shortcut.description }}</span>
              </span>
            </button>
            <div class="record-meta">
              <span class="badge mono neutral">{{ shortcut.relativePath }}</span>
              <span v-if="shortcut.active" class="badge success">active</span>
              <button class="button ghost" @click="props.workspace.openSettingsForShortcut(shortcut.id)">
                Open Related
              </button>
            </div>
          </div>
        </article>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">File Tree</p>
          <h3 class="settings-panel-title">Browse Files</h3>
          <p v-if="props.workspace.selectedWorkspace.value" class="muted mono">
            {{ props.workspace.selectedWorkspace.value.path }}
          </p>
          <p v-if="props.workspace.tree.value?.path" class="muted mono">/{{ props.workspace.tree.value.path }}</p>
        </div>
        <span class="badge">{{ props.workspace.rows.value.length }} entries</span>
      </div>

      <div class="inline-fields">
        <div class="field">
          <label class="label" for="workspace-path">Relative Path</label>
          <input
            id="workspace-path"
            v-model="props.workspace.pathInput.value"
            class="input mono"
            placeholder="src"
            @keyup.enter="props.workspace.loadTree"
          />
        </div>
        <div class="button-row">
          <button
            class="button primary"
            :disabled="props.workspace.loading.value || !props.workspace.selectedWorkspaceId.value"
            @click="props.workspace.loadTree"
          >
            Open
          </button>
          <button
            class="button"
            :disabled="props.workspace.loading.value || !props.workspace.selectedWorkspaceId.value"
            @click="props.workspace.goRoot"
          >
            Root
          </button>
        </div>
      </div>

      <div v-if="props.workspace.rows.value.length" class="list file-tree">
        <button
          v-for="row in props.workspace.rows.value"
          :key="row.node.path"
          class="list-item file-row"
          :class="{ active: row.node.kind === 'directory' || row.node.kind === 'file' }"
          :style="{ paddingLeft: `${14 + row.depth * 18}px` }"
          :disabled="props.workspace.downloadingFilePath.value === row.node.path"
          @click="props.workspace.openWorkspaceNode(row.node)"
        >
          <span class="mono">{{ row.node.kind === 'directory' ? '▸' : row.node.kind === 'file' ? '↓' : '·' }}</span>
          <span class="file-name mono">{{ row.node.path || row.node.name }}</span>
          <span class="muted">
            {{
              props.workspace.downloadingFilePath.value === row.node.path
                ? 'downloading…'
                : props.workspace.formatSize(row.node)
            }}
          </span>
        </button>
      </div>
      <div v-else class="empty-state">No files to display.</div>
    </section>
  </div>
</template>
