<script setup lang="ts">
import HarnessSettingsPanel from '@/components/settings/HarnessSettingsPanel.vue'
import McpServerControlPanel from '@/components/settings/McpServerControlPanel.vue'
import PluginsPanel from '@/components/settings/PluginsPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import type { SettingsSubpageDefinition } from '@/components/settings/workbench/settingsSectionNavigation'

const pages: SettingsSubpageDefinition[] = [
  {
    id: 'plugin-workbench',
    label: 'Plugin Workbench',
    description: 'Configure plugins, run tools and commands, and inspect capabilities, logs, and diagnostics.',
    keywords: ['plugins', 'schema', 'config', 'tools', 'commands', 'logs'],
  },
  {
    id: 'mcp-server',
    label: 'MCP Server',
    description: 'Manage the connected server’s MCP listener, OAuth policy, public identity, and tool exposure.',
    keywords: ['mcp', 'oauth', 'chatgpt', 'public url', 'tools'],
  },
  {
    id: 'harnesses',
    label: 'Tool harnesses',
    description: 'Create named browser, shell, and editor harness configurations.',
    keywords: ['browser', 'shell', 'editor', 'environment', 'commands'],
  },
]
</script>

<template>
  <SettingsSectionWorkbench
    section="plugins-tools"
    title="Plugins & Tools"
    description="Operate the plugin runtime, expose Agena through MCP, and configure provider-native tool harnesses."
    :pages="pages"
    default-page="plugin-workbench"
    v-slot="{ activePage }"
  >
    <PluginsPanel v-if="activePage === 'plugin-workbench'" />
    <McpServerControlPanel v-else-if="activePage === 'mcp-server'" />
    <HarnessSettingsPanel v-else />
  </SettingsSectionWorkbench>
</template>
