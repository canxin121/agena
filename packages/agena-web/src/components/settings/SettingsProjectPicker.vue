<script setup lang="ts">
// Project-scoped settings need a target project. Projects are grouped the same
// way conversations are (by task/category header), collapse, and page through
// the server catalog, so this mirrors the session picker instead of keeping a
// long flat list.
import { computed, onMounted, ref } from 'vue'
import { RiFolderLine } from '@remixicon/vue'

import GroupedPicker from '@/components/ui/GroupedPicker.vue'
import type { OptionMenuGroup } from '@/components/ui/optionMenu.types'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import { settingsText as st } from '@/i18n/settingsText'
import { useDirectorySessionStore } from '@/stores/directorySessionStore'
import { directoriesForPage, PROJECTS_PER_PAGE } from './projectCatalog'

const directories = useDirectorySessionStore()
const page = ref(1)
const loading = ref(false)

const visibleProjects = computed<DirectoryEntry[]>(() =>
  directoriesForPage(directories.directoryPageRows, directories.directoriesById),
)

// The sidebar store reports the fetched window; it can end one row short of a
// full page when more projects exist, so the total is rounded up to cover the
// row it reserved for "more".
const pageCount = computed(() => {
  const total = Math.max(0, directories.directoryPageTotal)
  const fetched = Math.max(0, directories.directoryPageRows.length)
  const pages = Math.ceil(total / PROJECTS_PER_PAGE)
  return Math.max(1, pages, fetched > 0 ? 1 : 0)
})

// Project labels come from their paths, so the picker groups them by the
// project's parent directory: that is the closest thing to a project category
// the instance stores, and it keeps long catalogs scannable.
const groups = computed<OptionMenuGroup[]>(() => {
  const buckets = new Map<string, DirectoryEntry[]>()
  for (const entry of visibleProjects.value) {
    const path = String(entry.path || '')
    const parent = path.replace(/\/+$/, '').split('/').slice(0, -1).join('/') || '/'
    const rows = buckets.get(parent) || []
    rows.push(entry)
    buckets.set(parent, rows)
  }
  return [...buckets.entries()].map(([parent, rows]) => ({
    id: `project-parent:${parent}`,
    title: parent,
    subtitle: st('{count} projects', { count: rows.length }),
    items: rows.map((entry) => ({
      id: String(entry.id),
      label: String(entry.label || entry.path),
      description: entry.path,
      checked: false,
      keywords: `${entry.id} ${entry.path}`,
    })),
  }))
})

async function load(target: number) {
  const next = Math.max(0, Math.floor(target) - 1)
  loading.value = true
  try {
    await directories.commandSetDirectoriesPage(next, { silent: true })
    page.value = next + 1
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  if (!directories.directoryPageRows.length) void load(1)
})

defineExpose({ load })
</script>

<template>
  <GroupedPicker
    block
    size="sm"
    :icon="RiFolderLine"
    :placeholder="$st('Select a project')"
    :groups="groups"
    :title="$st('Project')"
    :search-placeholder="$st('Search projects...')"
    :empty-text="$st('No projects found.')"
    :page="page"
    :page-count="pageCount"
    :page-loading="loading"
    @select="(id) => $emit('select', id)"
    @request-page="load"
  />
</template>
