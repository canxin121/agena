<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import {
  listWorkspaceFileTree,
  listWorkspaces,
  type WorkspaceFileNode,
  type WorkspaceFileTreeResource,
  type WorkspaceResource,
} from '@/agena/lib/agenaApi'

const workspaces = ref<WorkspaceResource[]>([])
const selectedWorkspaceId = ref<number | null>(null)
const pathInput = ref('')
const tree = ref<WorkspaceFileTreeResource | null>(null)
const loading = ref(false)
const errorMessage = ref('')

type TreeRow = {
  node: WorkspaceFileNode
  depth: number
}

const selectedWorkspace = computed(() =>
  workspaces.value.find((workspace) => workspace.id === selectedWorkspaceId.value) || null,
)

const rows = computed<TreeRow[]>(() => flattenTree(tree.value?.entries || []))

function flattenTree(nodes: WorkspaceFileNode[], depth = 0): TreeRow[] {
  return nodes.flatMap((node) => [
    { node, depth },
    ...flattenTree(node.children || [], depth + 1),
  ])
}

function formatSize(node: WorkspaceFileNode): string {
  if (node.kind === 'directory') return 'dir'
  if (node.size === null || node.size === undefined) return node.kind
  if (node.size < 1024) return `${node.size} B`
  if (node.size < 1024 * 1024) return `${(node.size / 1024).toFixed(1)} KB`
  return `${(node.size / 1024 / 1024).toFixed(1)} MB`
}

async function loadWorkspaces() {
  loading.value = true
  errorMessage.value = ''
  try {
    workspaces.value = await listWorkspaces()
    if (!selectedWorkspaceId.value && workspaces.value.length) {
      selectedWorkspaceId.value = workspaces.value[0].id
    }
    if (selectedWorkspaceId.value) {
      await loadTree()
    }
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

async function loadTree() {
  const workspaceId = selectedWorkspaceId.value
  if (!workspaceId) return
  loading.value = true
  errorMessage.value = ''
  try {
    tree.value = await listWorkspaceFileTree({
      workspaceId,
      path: pathInput.value,
      depth: 4,
      limit: 1000,
    })
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

function openDirectory(node: WorkspaceFileNode) {
  if (node.kind !== 'directory') return
  pathInput.value = node.path
  void loadTree()
}

function goRoot() {
  pathInput.value = ''
  void loadTree()
}

onMounted(() => {
  void loadWorkspaces()
})
</script>

<template>
  <section class="page">
    <header class="page-header">
      <div>
        <h1 class="page-title">Workspace</h1>
        <p class="page-description">Browse workspace files and inspect model-produced patch diffs in Chat.</p>
      </div>
      <div class="button-row">
        <button class="button ghost" :disabled="loading" @click="loadWorkspaces">Refresh</button>
        <button class="button" :disabled="loading || !selectedWorkspaceId" @click="goRoot">Root</button>
      </div>
    </header>

    <div v-if="errorMessage" class="notice">{{ errorMessage }}</div>

    <section class="card">
      <div class="grid two">
        <div class="field">
          <label class="label" for="workspace-select">Workspace</label>
          <select id="workspace-select" v-model.number="selectedWorkspaceId" class="select" @change="loadTree">
            <option v-for="workspace in workspaces" :key="workspace.id" :value="workspace.id">
              {{ workspace.path }}
            </option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="workspace-path">Relative Path</label>
          <input
            id="workspace-path"
            v-model="pathInput"
            class="input mono"
            placeholder="src"
            @keyup.enter="loadTree"
          />
        </div>
      </div>
      <div class="button-row" style="margin-top: 12px">
        <button class="button primary" :disabled="loading || !selectedWorkspaceId" @click="loadTree">Open</button>
      </div>
    </section>

    <section class="card">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>File Tree</h3>
          <p v-if="selectedWorkspace" class="muted mono">{{ selectedWorkspace.path }}</p>
          <p v-if="tree?.path" class="muted mono">/{{ tree.path }}</p>
        </div>
        <span class="badge">{{ rows.length }} entries</span>
      </div>

      <div v-if="rows.length" class="list file-tree">
        <button
          v-for="row in rows"
          :key="row.node.path"
          class="list-item file-row"
          :class="{ active: row.node.kind === 'directory' }"
          :style="{ paddingLeft: `${14 + row.depth * 18}px` }"
          @click="openDirectory(row.node)"
        >
          <span class="mono">{{ row.node.kind === 'directory' ? '▸' : '·' }}</span>
          <span class="file-name mono">{{ row.node.path || row.node.name }}</span>
          <span class="muted">{{ formatSize(row.node) }}</span>
        </button>
      </div>
      <p v-else class="muted">No files to display.</p>
    </section>
  </section>
</template>
