<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import SettingsSectionWorkbench from './workbench/SettingsSectionWorkbench.vue'
import { buildSettingsSubpages, SETTINGS_DEFAULT_SUBPAGE } from './settingsNavigationCatalog'
import type { SkillInstallSource, SkillScope } from '@/lib/skillsApi'

const SkillLibraryPanel = defineAsyncComponent(() => import('./skills/SkillLibraryPanel.vue'))
const SkillInstallPanel = defineAsyncComponent(() => import('./skills/SkillInstallPanel.vue'))
defineProps<{ activePage: string }>()
const route = useRoute()
const router = useRouter()
const pages = computed(() => buildSettingsSubpages('skills'))
const panel = ref<{ refresh: () => Promise<void> } | null>(null)
const installSeed = ref<{ source: SkillInstallSource; scope: SkillScope } | null>(null)

function openInstall(seed?: { source: SkillInstallSource; scope: SkillScope }) {
  installSeed.value = seed || null
  void router.push({ path: route.path, query: { view: 'install' }, hash: route.hash })
}
function installed(id: string) {
  void router.push({ path: route.path, query: { view: 'library', skill: id }, hash: route.hash })
}
defineExpose({
  refresh: async () => {
    await panel.value?.refresh()
  },
})
</script>

<template>
  <SettingsSectionWorkbench
    section="skills"
    :pages="pages"
    :active-page="activePage"
    :default-page="SETTINGS_DEFAULT_SUBPAGE.skills"
    v-slot="{ activePage }"
  >
    <SkillLibraryPanel
      v-if="activePage === 'library'"
      ref="panel"
      @install="openInstall()"
      @install-from-source="openInstall"
    />
    <SkillInstallPanel v-else ref="panel" :initial-source="installSeed" @installed="installed" />
  </SettingsSectionWorkbench>
</template>
