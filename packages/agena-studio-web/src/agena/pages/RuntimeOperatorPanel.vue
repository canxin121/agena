<script setup lang="ts">
import type { RuntimeStatus } from '@/agena/lib/agenaApi'

const props = defineProps<{
  runtime: RuntimeStatus | null
}>()
</script>

<template>
  <div class="grid three">
    <section class="card">
      <h3>Runtime</h3>
      <div v-if="props.runtime" class="stack">
        <div><strong>Config Found:</strong> {{ props.runtime.config_found ? 'yes' : 'no' }}</div>
        <div>
          <strong>Session Runtime:</strong> {{ props.runtime.session_runtime_available ? 'enabled' : 'disabled' }}
        </div>
        <div><strong>Watch Paths:</strong> {{ props.runtime.watch_paths.length }}</div>
        <div><strong>Tool Registry:</strong> {{ props.runtime.operator.ui?.tool_registry_generation ?? 0 }}</div>
      </div>
    </section>
    <section class="card">
      <h3>MCP</h3>
      <div v-if="props.runtime" class="stack">
        <div><strong>Servers:</strong> {{ props.runtime.operator.mcp.server_count }}</div>
        <div><strong>Tools:</strong> {{ props.runtime.operator.mcp.tool_count }}</div>
      </div>
    </section>
    <section class="card">
      <h3>Agents + Skills</h3>
      <div v-if="props.runtime" class="stack">
        <div><strong>Default Agent:</strong> {{ props.runtime.operator.agents.default_agent }}</div>
        <div><strong>Agents:</strong> {{ props.runtime.operator.agents.total_count }}</div>
        <div><strong>LSP Servers:</strong> {{ props.runtime.operator.lsp.server_count }}</div>
        <div><strong>Diagnostics:</strong> {{ props.runtime.operator.lsp.diagnostics_count }}</div>
        <div><strong>Skills:</strong> {{ props.runtime.operator.skills.skill_count }}</div>
        <div><strong>Commands:</strong> {{ props.runtime.operator.skills.command_count }}</div>
        <div>
          <strong>Last Tool Event:</strong>
          {{ props.runtime.operator.ui?.tool_registry_last_event?.exposed_name || 'n/a' }}
        </div>
      </div>
    </section>
  </div>
</template>
