<script setup lang="ts">
import type { RuntimeStatus } from '@/agena/lib/agenaApi'

const props = defineProps<{
  kind: 'mcp' | 'lsp'
  runtime: RuntimeStatus | null
  filteredMcpServers: Array<{ name: string; tool_count: number }>
  filteredLspServers: Array<{ name: string; command: string; file_extensions: string[]; root_markers: string[] }>
  mcpQuery: string
  lspQuery: string
  openRuntimeConfigRoot: () => void
  openWorkspaceShortcut: (shortcutId: string) => void
  openWorkspacePath: (relativePath?: string | null) => void
}>()

const emit = defineEmits<{
  'update:mcpQuery': [value: string]
  'update:lspQuery': [value: string]
}>()
</script>

<template>
  <section v-if="props.kind === 'mcp'" class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3>MCP Servers</h3>
        <p class="muted">Inspect connected MCP servers and their resolved runtime state.</p>
      </div>
      <div class="button-row" style="flex-wrap: wrap">
        <span class="badge">{{ props.filteredMcpServers.length }}/{{ props.runtime?.operator.mcp.servers.length || 0 }}</span>
        <button class="button" @click="props.openRuntimeConfigRoot">Open Config</button>
      </div>
    </div>
    <div v-if="props.runtime" class="stack">
      <div><strong>Server Count:</strong> {{ props.runtime.operator.mcp.server_count }}</div>
      <div><strong>Tool Count:</strong> {{ props.runtime.operator.mcp.tool_count }}</div>
      <div class="field" style="margin-top: 12px">
        <label class="label" for="runtime-mcp-query">Search MCP Servers</label>
        <input
          id="runtime-mcp-query"
          :value="props.mcpQuery"
          class="input mono"
          placeholder="server name / tool count"
          @input="emit('update:mcpQuery', ($event.target as HTMLInputElement).value)"
        />
      </div>
      <div v-if="props.filteredMcpServers.length" class="list">
        <div v-for="server in props.filteredMcpServers" :key="server.name" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div><strong>{{ server.name }}</strong></div>
              <div class="muted">tools {{ server.tool_count }}</div>
              <div class="muted mono">config=~/agena/agena.json</div>
            </div>
            <div class="button-row" style="flex-wrap: wrap">
              <button class="button" @click="props.openRuntimeConfigRoot">Open Config</button>
            </div>
          </div>
        </div>
      </div>
      <div v-else class="muted">No MCP servers matched the current filter.</div>
    </div>
  </section>

  <section v-else class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3>LSP Fleet</h3>
        <p class="muted">Inspect configured LSP servers and source roots when diagnostics look wrong.</p>
      </div>
      <div class="button-row" style="flex-wrap: wrap">
        <span class="badge">{{ props.filteredLspServers.length }}/{{ props.runtime?.operator.lsp.servers.length || 0 }}</span>
        <button class="button" @click="props.openRuntimeConfigRoot">Open Config</button>
        <button class="button" @click="props.openWorkspacePath('src')">Open Source Root</button>
      </div>
    </div>
    <div v-if="props.runtime" class="stack">
      <div><strong>Server Count:</strong> {{ props.runtime.operator.lsp.server_count }}</div>
      <div><strong>Diagnostics:</strong> {{ props.runtime.operator.lsp.diagnostics_count }}</div>
      <div><strong>Files With Diagnostics:</strong> {{ props.runtime.operator.lsp.files_with_diagnostics }}</div>
      <div class="field" style="margin-top: 12px">
        <label class="label" for="runtime-lsp-query">Search LSP Servers</label>
        <input
          id="runtime-lsp-query"
          :value="props.lspQuery"
          class="input mono"
          placeholder="name / command / extension / root marker"
          @input="emit('update:lspQuery', ($event.target as HTMLInputElement).value)"
        />
      </div>
      <div v-if="props.filteredLspServers.length" class="list">
        <div v-for="server in props.filteredLspServers" :key="server.name" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div><strong>{{ server.name }}</strong></div>
              <div class="muted mono">{{ server.command }}</div>
              <div class="muted">extensions: {{ server.file_extensions.join(', ') || 'all' }}</div>
              <div class="muted">root markers: {{ server.root_markers.join(', ') || 'workspace root' }}</div>
            </div>
            <div class="button-row" style="flex-wrap: wrap">
              <button class="button" @click="props.openRuntimeConfigRoot">Open Config</button>
              <button class="button" @click="props.openWorkspacePath('src')">Open Source Root</button>
            </div>
          </div>
        </div>
      </div>
      <div v-else class="muted">No LSP servers matched the current filter.</div>
    </div>
  </section>
</template>
