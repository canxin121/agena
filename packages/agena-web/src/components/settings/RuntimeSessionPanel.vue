<script setup lang="ts">
import { useSettingsDraftGuard } from './useSettingsDraftGuard'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { RiRefreshLine, RiRefreshFill, RiCpuLine, RiStackLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import AcpAgentsPanel from './AcpAgentsPanel.vue'
import AgentProfilesPanel from './AgentProfilesPanel.vue'
import AgentProfileDefaultsPanel from './AgentProfileDefaultsPanel.vue'
import ModelSettingsPanel from './ModelSettingsPanel.vue'
import PromptLibraryPanel from './PromptLibraryPanel.vue'
import SystemPromptPanel from './SystemPromptPanel.vue'
import { resolveSettingsSessionId } from './settingsSessionScope'
import ServerSettingField from '@/components/settings/ServerSettingField.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'
import Button from '@/components/ui/Button.vue'
import { apiJson } from '@/lib/api'
import { lastSessionId } from '@/lib/lastSession'
import { useChatStoreUnscoped } from '@/stores/chat'
import { settingsText as st } from '@/i18n/settingsText'

const { t } = useI18n()
const props = defineProps<{ activePage: string; scope: string; sessionId?: string }>()
const route = useRoute()
const chat = useChatStoreUnscoped()
const configLayer = computed(() => (props.scope === 'global' || props.scope === 'workspace' ? props.scope : undefined))
const defaultsScope = computed(() => (props.scope === 'effective' ? props.scope : configLayer.value))
const workspaceConfigPath = ref('')
const refreshBusy = ref(false)
const refreshError = ref('')
const versionFields = ref<InstanceType<typeof ServerSettingField>[]>([])
const autoUpdateField = ref<InstanceType<typeof ServerSettingField> | null>(null)
const autoCompactionField = ref<InstanceType<typeof ServerSettingField> | null>(null)
const tokensField = ref<InstanceType<typeof ServerSettingField> | null>(null)
const titleFields = ref<InstanceType<typeof ServerSettingField>[]>([])
const modelSettingsPanel = ref<InstanceType<typeof ModelSettingsPanel> | null>(null)
const acpAgentsPanel = ref<InstanceType<typeof AcpAgentsPanel> | null>(null)
const agentProfilesPanel = ref<InstanceType<typeof AgentProfilesPanel> | null>(null)
const promptLibraryPanel = ref<InstanceType<typeof PromptLibraryPanel> | null>(null)
const systemPromptPanel = ref<InstanceType<typeof SystemPromptPanel> | null>(null)
const profileDefaultsPanel = ref<InstanceType<typeof AgentProfileDefaultsPanel> | null>(null)
const saving = computed(
  () =>
    modelSettingsPanel.value?.saving ||
    profileDefaultsPanel.value?.saving ||
    systemPromptPanel.value?.saving ||
    acpAgentsPanel.value?.saving,
)
const dirty = computed(
  () =>
    modelSettingsPanel.value?.dirty ||
    profileDefaultsPanel.value?.dirty ||
    systemPromptPanel.value?.dirty ||
    acpAgentsPanel.value?.dirty,
)

useSettingsDraftGuard({
  dirty: () => Boolean(dirty.value),
  busy: () => Boolean(saving.value),
  dialog: () => ({
    title: modelSettingsPanel.value?.dirty ? st('Discard unsaved model changes?') : st('Discard unsaved changes?'),
  }),
  sameTarget: (to) =>
    to.path === route.path &&
    to.query.scope === route.query.scope &&
    to.query.view === route.query.view &&
    (props.scope !== 'session' ||
      resolveSettingsSessionId(to.query.targetSessionId, lastSessionId.value || chat.selectedSessionId) ===
        props.sessionId),
})

const titleOptions = computed(() => [
  {
    path: 'session.auto_title.enabled',
    label: st('Automatically name sessions'),
    description: st(
      'Name new sessions in the background and update misleading titles. Manually assigned names are preserved.',
    ),
    kind: 'boolean' as const,
    defaultValue: true,
  },
  {
    path: 'session.auto_title.message_interval',
    label: st('User messages between title checks'),
    description: st(
      'After initial naming and third-message calibration, wait this many new user messages before another check.',
    ),
    kind: 'number' as const,
    defaultValue: 5,
  },
  {
    path: 'session.auto_title.cooldown_seconds',
    label: st('Title check cooldown (seconds)'),
    description: st('Minimum time between later title checks. Set to 0 to use only the message interval.'),
    kind: 'number' as const,
    defaultValue: 300,
  },
])
const versionOptions = computed(() => [
  {
    path: 'runtime.providers.client_versions.codex',
    label: t('settings.tui.fields.codexVersion'),
    description: t('settings.tui.fields.codexVersionDescription'),
  },
  {
    path: 'runtime.providers.client_versions.claude',
    label: t('settings.tui.fields.claudeVersion'),
    description: t('settings.tui.fields.claudeVersionDescription'),
  },
  {
    path: 'runtime.providers.client_versions.gemini',
    label: t('settings.tui.fields.geminiVersion'),
    description: t('settings.tui.fields.geminiVersionDescription'),
  },
])
async function refresh() {
  await Promise.all(
    [
      ...versionFields.value,
      ...titleFields.value,
      autoUpdateField.value,
      autoCompactionField.value,
      tokensField.value,
      modelSettingsPanel.value,
      acpAgentsPanel.value,
      agentProfilesPanel.value,
      promptLibraryPanel.value,
      systemPromptPanel.value,
      profileDefaultsPanel.value,
    ].map((field) => field?.refresh()),
  )
}
defineExpose({ refresh })

const pages = computed(() => buildSettingsSubpages('runtime-session'))

async function refreshClientVersions() {
  if (refreshBusy.value) return
  refreshBusy.value = true
  refreshError.value = ''
  try {
    await apiJson('/api/v1/providers/client-versions/refresh', { method: 'POST' })
    await refresh()
  } catch (reason) {
    refreshError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    refreshBusy.value = false
  }
}
</script>

<template>
  <SettingsSectionWorkbench
    section="runtime-session"
    :pages="pages"
    :active-page="activePage"
    :active-scope="scope"
    :workspace-config-path="configLayer && activePage !== 'client-versions' ? workspaceConfigPath : undefined"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['runtime-session']"
    v-slot="{ activePage }"
  >
    <div v-if="activePage === 'client-versions'" class="settings-columns">
      <SettingsCard :title="$st('Update policy')" :icon="RiRefreshFill">
        <ServerSettingField
          ref="autoUpdateField"
          path="runtime.providers.client_versions.auto_update"
          :label="st('Automatically update provider client versions')"
          :description="
            st(
              'When enabled, refresh Codex, Claude, and Gemini client identities from npm on startup and daily. Turn this off to pin the configured versions.',
            )
          "
          kind="boolean"
          :default-value="true"
          compact
        />
      </SettingsCard>
      <SettingsCard :title="$st('Client versions')" :icon="RiCpuLine" :count="versionOptions.length">
        <template #actions>
          <Button variant="outline" size="sm" :disabled="refreshBusy" @click="refreshClientVersions">
            <RiRefreshLine class="mr-2 h-4 w-4 shrink-0" :class="refreshBusy ? 'animate-spin' : ''" />
            {{ refreshBusy ? t('settings.tui.refreshing') : t('settings.tui.refreshFromNpm') }}
          </Button>
        </template>
        <p
          v-if="refreshError"
          role="alert"
          class="mb-3 break-words rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-xs text-destructive"
        >
          {{ refreshError }}
        </p>
        <ServerSettingField
          v-for="version in versionOptions"
          :key="version.path"
          ref="versionFields"
          v-bind="version"
          placeholder="npm version or semver"
          monospace
          compact
        />
      </SettingsCard>
    </div>
    <AcpAgentsPanel v-else-if="activePage === 'acp-agents'" ref="acpAgentsPanel" />
    <AgentProfilesPanel
      v-else-if="activePage === 'agent-profiles'"
      ref="agentProfilesPanel"
      :scope="scope"
      :session-id="sessionId"
      @workspace-config-path="workspaceConfigPath = $event"
    />
    <PromptLibraryPanel
      v-else-if="activePage === 'prompt-library'"
      ref="promptLibraryPanel"
      :scope="scope === 'workspace' ? 'workspace' : 'global'"
      @workspace-config-path="workspaceConfigPath = $event"
    />
    <SystemPromptPanel
      v-else-if="activePage === 'system-prompt'"
      ref="systemPromptPanel"
      :scope="scope === 'session' ? 'session' : configLayer || 'global'"
      :session-id="sessionId"
      @workspace-config-path="workspaceConfigPath = $event"
    />
    <div v-else class="settings-stack">
      <ModelSettingsPanel
        ref="modelSettingsPanel"
        :scope="scope"
        :session-id="scope === 'session' ? sessionId : undefined"
      />
      <AgentProfileDefaultsPanel v-if="defaultsScope" ref="profileDefaultsPanel" :scope="defaultsScope" />
      <div v-if="configLayer" class="settings-columns">
        <SettingsCard :title="$st('Automatic session titles')" :icon="RiCpuLine">
          <ServerSettingField
            v-for="field in titleOptions"
            :key="field.path"
            ref="titleFields"
            v-bind="field"
            :target-layer="configLayer"
            compact
          />
        </SettingsCard>
        <SettingsCard id="settings-context-management" :title="$st('Context management')" :icon="RiStackLine">
          <ServerSettingField
            ref="autoCompactionField"
            path="session.compaction.auto"
            :label="t('settings.tui.fields.autoCompaction')"
            :description="t('settings.tui.fields.autoCompactionDescription')"
            kind="boolean"
            :default-value="true"
            :target-layer="configLayer"
            @loaded="workspaceConfigPath = $event.workspace.config_path || ''"
            compact
          />
          <ServerSettingField
            ref="tokensField"
            path="session.compaction.reserved_tokens"
            :label="t('settings.tui.fields.reservedTokens')"
            :description="
              st(
                'Leave empty to derive headroom from the model’s usable input budget. A value of 0 uses the entire input budget before compaction.',
              )
            "
            kind="number"
            include-empty
            :placeholder="st('Automatic')"
            :target-layer="configLayer"
            compact
          />
        </SettingsCard>
      </div>
    </div>
  </SettingsSectionWorkbench>
</template>
