<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine } from '@remixicon/vue'

import ServerSettingField from '@/components/settings/ServerSettingField.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'
import Button from '@/components/ui/Button.vue'
import { apiJson } from '@/lib/api'
import { settingsText as st } from '@/i18n/settingsText'

const { t } = useI18n()
const refreshBusy = ref(false)
const refreshError = ref('')
const clientVersionNonce = ref(0)

const pages = computed(() => buildSettingsSubpages('runtime-session'))

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
    <section v-if="activePage === 'client-versions'" class="grid gap-4">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <p class="max-w-3xl text-sm text-muted-foreground">{{ t('settings.tui.clientVersionsDescription') }}</p>
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
      <ServerSettingField
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
      <p class="max-w-3xl text-sm text-muted-foreground">{{ t('settings.tui.compactionDescription') }}</p>
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
    </section>
  </SettingsSectionWorkbench>
</template>
