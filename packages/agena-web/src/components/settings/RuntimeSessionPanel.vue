<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine } from '@remixicon/vue'

import ServerSettingField from '@/components/settings/ServerSettingField.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'
import Button from '@/components/ui/Button.vue'
import { apiJson } from '@/lib/api'

const { t } = useI18n()
const refreshBusy = ref(false)
const refreshError = ref('')
const clientVersionNonce = ref(0)

const pages = buildSettingsSubpages('runtime-session')

async function refreshClientVersions() {
  if (refreshBusy.value) return
  refreshBusy.value = true
  refreshError.value = ''
  try {
    await apiJson('/api/v1/providers/client-versions/refresh', { method: 'POST' })
    clientVersionNonce.value += 1
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
    :title="String(t('settings.tabs.runtimeSession'))"
    :description="String(t('settings.tui.runtimeDescription'))"
    :pages="pages"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['runtime-session']"
    v-slot="{ activePage }"
  >
    <section v-if="activePage === 'client-versions'" class="grid gap-4">
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 class="text-base font-semibold">{{ t('settings.tui.clientVersionsTitle') }}</h2>
          <p class="mt-1 max-w-3xl text-sm text-muted-foreground">{{ t('settings.tui.clientVersionsDescription') }}</p>
        </div>
        <Button variant="outline" size="sm" :disabled="refreshBusy" @click="refreshClientVersions">
          <RiRefreshLine class="mr-2 h-4 w-4" :class="refreshBusy ? 'animate-spin' : ''" />
          {{ refreshBusy ? t('settings.tui.refreshing') : t('settings.tui.refreshFromNpm') }}
        </Button>
      </div>
      <div
        v-if="refreshError"
        class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
      >
        {{ refreshError }}
      </div>
      <div :key="clientVersionNonce" class="grid gap-2">
        <ServerSettingField
          path="runtime.providers.client_versions.codex"
          :label="t('settings.tui.fields.codexVersion')"
          :description="t('settings.tui.fields.codexVersionDescription')"
          placeholder="npm version or semver"
          monospace
          compact
        />
        <ServerSettingField
          path="runtime.providers.client_versions.claude"
          :label="t('settings.tui.fields.claudeVersion')"
          :description="t('settings.tui.fields.claudeVersionDescription')"
          placeholder="npm version or semver"
          monospace
          compact
        />
        <ServerSettingField
          path="runtime.providers.client_versions.gemini"
          :label="t('settings.tui.fields.geminiVersion')"
          :description="t('settings.tui.fields.geminiVersionDescription')"
          placeholder="npm version or semver"
          monospace
          compact
        />
      </div>
    </section>

    <section v-else class="grid gap-3">
      <div>
        <h2 class="text-base font-semibold">{{ t('settings.tui.compactionTitle') }}</h2>
        <p class="mt-1 max-w-3xl text-sm text-muted-foreground">{{ t('settings.tui.compactionDescription') }}</p>
      </div>
      <ServerSettingField
        path="session.compaction.auto"
        :label="t('settings.tui.fields.autoCompaction')"
        :description="t('settings.tui.fields.autoCompactionDescription')"
        kind="boolean"
        :default-value="true"
        compact
      />
      <ServerSettingField
        path="session.compaction.reserved_tokens"
        :label="t('settings.tui.fields.reservedTokens')"
        :description="t('settings.tui.fields.reservedTokensDescription')"
        kind="number"
        :default-value="12000"
        placeholder="12000"
        compact
      />
    </section>
  </SettingsSectionWorkbench>
</template>
