<script setup lang="ts">
// Session-scoped settings need a target conversation. Every scoped page shows
// this one control so choosing the conversation always looks and behaves the
// same: conversations are grouped by project, groups collapse, and the pager
// asks the server for another page instead of truncating the list.
import { RiChat3Line } from '@remixicon/vue'

import GroupedPicker from '@/components/ui/GroupedPicker.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { useSettingsSessionTarget } from './settingsSessionScope'

const target = useSettingsSessionTarget()
const searchPlaceholder = st('Search conversations...')
</script>

<template>
  <section
    class="flex min-w-0 flex-wrap items-center gap-2 rounded-xl border border-border/60 bg-muted/15 px-3 py-2"
    :aria-label="$st('Session for this setting')"
  >
    <span class="text-xs font-medium text-muted-foreground">{{ $st('Conversation') }}</span>
    <GroupedPicker
      class="min-w-0 flex-1"
      block
      size="sm"
      collapsible-groups
      :icon="RiChat3Line"
      :label="target.selected.value?.title || ''"
      :placeholder="$st('Select a conversation')"
      :groups="target.pickerGroups.value"
      :title="$st('Session for this setting')"
      :search-placeholder="searchPlaceholder"
      :empty-text="$st('No conversations found.')"
      :page="target.catalog.page.value"
      :page-count="target.catalog.pageCount.value"
      :page-loading="target.catalog.loading.value"
      :selected-id="target.sessionId.value"
      @select="target.select"
      @request-page="(page) => target.catalog.load(page)"
    />
    <span
      v-if="target.selected.value?.project"
      class="min-w-0 truncate font-mono text-[10px] text-muted-foreground"
      :title="target.selected.value.project"
    >
      {{ target.selected.value.project }}
    </span>
  </section>
</template>
