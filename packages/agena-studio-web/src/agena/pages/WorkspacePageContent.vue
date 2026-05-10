<script setup lang="ts">
import type { useWorkspacePageState } from './useWorkspacePageState'

const props = defineProps<{
  workspace: ReturnType<typeof useWorkspacePageState>
}>()
</script>

<template>
  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3>Resolve Workspace</h3>
        <p class="muted">Attach a repo or project path so agena can expose sessions, files, commands, and plans.</p>
      </div>
    </div>
    <div class="field">
      <label class="label" for="workspace-root-input">Workspace Path</label>
      <input id="workspace-root-input" v-model="props.workspace.workspacePath.value" class="input mono" placeholder="/path/to/repo" />
    </div>
    <div class="button-row" style="margin-top: 12px">
      <button class="button primary" :disabled="props.workspace.loading.value || !props.workspace.workspacePath.value.trim()" @click="props.workspace.resolveWorkspaceAction(true)">
        Resolve or Create
      </button>
      <button class="button" :disabled="props.workspace.loading.value || !props.workspace.workspacePath.value.trim()" @click="props.workspace.resolveWorkspaceAction(false)">
        Create Only
      </button>
    </div>
  </section>

  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3>Project Status</h3>
        <p class="muted">See git health, tracked worktree activity, and quick repo readiness before editing files.</p>
      </div>
    </div>
    <div v-if="props.workspace.gitStatus.value" class="stack">
      <div class="muted mono">root={{ props.workspace.gitStatus.value.workspace_root }}</div>
      <div class="muted mono">
        git={{ props.workspace.gitStatus.value.git_available ? 'available' : 'missing' }} · repo={{ props.workspace.gitStatus.value.repo ? 'yes' : 'no' }} · gh={{ props.workspace.gitStatus.value.gh_available ? 'available' : 'missing' }}
      </div>
      <div v-if="props.workspace.gitStatus.value.branch" class="muted mono">
        branch={{ props.workspace.gitStatus.value.branch }} · upstream={{ props.workspace.gitStatus.value.upstream || 'none' }} · ahead={{ props.workspace.gitStatus.value.ahead ?? 0 }} · behind={{ props.workspace.gitStatus.value.behind ?? 0 }}
      </div>
      <div class="muted mono">
        changed={{ props.workspace.gitStatus.value.changed_files }} · staged={{ props.workspace.gitStatus.value.staged_files }} · unstaged={{ props.workspace.gitStatus.value.unstaged_files }} · untracked={{ props.workspace.gitStatus.value.untracked_files }}
      </div>
      <div class="muted mono">
        clean={{ props.workspace.gitStatus.value.clean ? 'true' : 'false' }} · worktree_active_sessions={{ props.workspace.gitStatus.value.worktree_active_sessions }} · worktree_managed_dirs={{ props.workspace.gitStatus.value.worktree_managed_dirs }}
      </div>
    </div>
    <p v-else class="muted">Git/worktree status is not available.</p>
  </section>

  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3>Current Workspace</h3>
        <p class="muted">Use Workspace as the project entry hub, then jump straight into Chat or Runtime with the selected workspace context.</p>
      </div>
      <div class="button-row" style="flex-wrap: wrap">
        <button class="button" :disabled="!props.workspace.selectedWorkspaceId.value" @click="props.workspace.openChatForWorkspace">Open in Chat</button>
        <button class="button" :disabled="!props.workspace.selectedWorkspaceId.value" @click="props.workspace.openRuntimeForWorkspace">Open in Runtime</button>
        <button class="button" :disabled="!props.workspace.selectedWorkspaceId.value" @click="props.workspace.openRuntimeConfigRoot">Open Config Root</button>
        <button class="button" :disabled="!props.workspace.selectedWorkspaceId.value" @click="props.workspace.openWorktreeDirectory">Open Worktrees</button>
        <button class="button" :disabled="!props.workspace.selectedWorkspaceId.value" @click="props.workspace.openLogsDirectory">Open Logs</button>
        <button class="button" :disabled="!props.workspace.selectedWorkspaceId.value || props.workspace.loading.value" @click="props.workspace.renameSelectedWorkspace">
          Rename Workspace
        </button>
        <button class="button danger" :disabled="!props.workspace.selectedWorkspaceId.value || props.workspace.loading.value" @click="props.workspace.deleteSelectedWorkspace">
          Delete Workspace
        </button>
      </div>
    </div>
    <div v-if="props.workspace.selectedWorkspace.value" class="stack">
      <div class="muted mono">{{ props.workspace.selectedWorkspace.value.path }}</div>
      <div v-if="props.workspace.workspaceSummaryFacts.value.length" class="muted mono">{{ props.workspace.workspaceSummaryFacts.value.join(' · ') }}</div>
      <div v-if="props.workspace.selectedShortcutId.value" class="muted mono">active_shortcut={{ props.workspace.selectedShortcutId.value }}</div>
    </div>
    <p v-else class="muted">Select or resolve a workspace to inspect project entry points.</p>
  </section>

  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3>Project Entry Points</h3>
        <p class="muted">Jump to common agena project directories without typing paths manually.</p>
      </div>
    </div>
    <div v-if="props.workspace.configSummaryFacts.value.length" class="muted mono" style="margin-bottom: 12px">
      {{ props.workspace.configSummaryFacts.value.join(' · ') }}
    </div>
    <div class="list">
      <div v-for="shortcut in props.workspace.workspaceConfigCards.value" :key="shortcut.id" class="list-item">
        <div class="page-header" style="align-items: flex-start">
          <button class="button ghost" style="text-align: left; flex: 1" @click="props.workspace.openShortcut(shortcut.relativePath)">
            <div>
              <strong>{{ shortcut.title }}</strong>
              <div class="muted">{{ shortcut.description }}</div>
            </div>
          </button>
          <div class="button-row" style="flex-wrap: wrap; justify-content: flex-end">
            <span class="badge mono">{{ shortcut.relativePath }}</span>
            <span v-if="shortcut.active" class="badge">active</span>
            <button class="button ghost" @click="props.workspace.openSettingsForShortcut(shortcut.id)">Open Related</button>
          </div>
        </div>
      </div>
    </div>
  </section>

  <section class="card">
    <div class="grid two">
      <div class="field">
        <label class="label" for="workspace-select">Workspace</label>
        <select id="workspace-select" v-model.number="props.workspace.selectedWorkspaceId.value" class="select" @change="props.workspace.loadTree">
          <option v-for="workspace in props.workspace.workspaces.value" :key="workspace.id" :value="workspace.id">
            {{ workspace.path }}
          </option>
        </select>
      </div>
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
    </div>
    <div class="button-row" style="margin-top: 12px">
      <button class="button primary" :disabled="props.workspace.loading.value || !props.workspace.selectedWorkspaceId.value" @click="props.workspace.loadTree">Open</button>
    </div>
  </section>

  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3>File Tree</h3>
        <p v-if="props.workspace.selectedWorkspace.value" class="muted mono">{{ props.workspace.selectedWorkspace.value.path }}</p>
        <p v-if="props.workspace.tree.value?.path" class="muted mono">/{{ props.workspace.tree.value.path }}</p>
      </div>
      <span class="badge">{{ props.workspace.rows.value.length }} entries</span>
    </div>

    <div v-if="props.workspace.rows.value.length" class="list file-tree">
      <button
        v-for="row in props.workspace.rows.value"
        :key="row.node.path"
        class="list-item file-row"
        :class="{ active: row.node.kind === 'directory' }"
        :style="{ paddingLeft: `${14 + row.depth * 18}px` }"
        @click="props.workspace.openDirectory(row.node)"
      >
        <span class="mono">{{ row.node.kind === 'directory' ? '▸' : '·' }}</span>
        <span class="file-name mono">{{ row.node.path || row.node.name }}</span>
        <span class="muted">{{ props.workspace.formatSize(row.node) }}</span>
      </button>
    </div>
    <p v-else class="muted">No files to display.</p>
  </section>
</template>
