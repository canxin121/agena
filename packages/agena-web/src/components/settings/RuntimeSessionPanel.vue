<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine, RiRefreshFill, RiCpuLine, RiStackLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import ServerSettingField from '@/components/settings/ServerSettingField.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'
import Button from '@/components/ui/Button.vue'
import { apiJson } from '@/lib/api'
import { settingsText as st } from '@/i18n/settingsText'

const { t } = useI18n()
const refreshBusy = ref(false)
const refreshError = ref('')
const versionFields = ref<InstanceType<typeof ServerSettingField>[]>([])
const autoUpdateField = ref<InstanceType<typeof ServerSettingField> | null>(null)
const autoCompactionField = ref<InstanceType<typeof ServerSettingField> | null>(null)
const tokensField = ref<InstanceType<typeof ServerSettingField> | null>(null)
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
    [...versionFields.value, autoUpdateField.value, autoCompactionField.value, tokensField.value].map((field) =>
      field?.refresh(),
    ),
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
defineProps<{ activePage: string }>()
</script>

<template>
  <SettingsSectionWorkbench
    section="runtime-session"
    :pages="pages"
    :active-page="activePage"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['runtime-session']"
    v-slot="{ activePage }"
  >
    <div v-if="activePage === 'client-versions'" class="space-y-4">
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
    <SettingsCard v-else :title="$st('Context management')" :icon="RiStackLine">
      <ServerSettingField
        ref="autoCompactionField"
        path="session.compaction.auto"
        :label="t('settings.tui.fields.autoCompaction')"
        :description="t('settings.tui.fields.autoCompactionDescription')"
        kind="boolean"
        :default-value="true"
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
        compact
      />
    </SettingsCard>
  </SettingsSectionWorkbench>
</template>
