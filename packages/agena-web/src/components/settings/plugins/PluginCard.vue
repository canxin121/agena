<script setup lang="ts">
import { computed, useId } from 'vue'
import { RiArrowRightLine, RiSettings3Line } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import PluginAvatar from '@/components/settings/plugins/PluginAvatar.vue'
import { pluginStateLabel } from '@/components/settings/pluginLabels'
import type { PluginInteractionPage } from '@/features/pluginWeb/settingsEntry'

const props = defineProps<{
  pluginId: string
  summary: string
  kind: string
  state: string
  pages: PluginInteractionPage[]
}>()
const emit = defineEmits<{
  settings: []
  details: []
  'open-page': [page: PluginInteractionPage]
}>()
const headingId = useId()
const stateClass = computed(() => {
  if (props.state === 'running') return 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
  if (props.state === 'restarting') return 'bg-amber-500/10 text-amber-700 dark:text-amber-400'
  if (props.state === 'failed') return 'bg-destructive/10 text-destructive'
  return 'bg-muted text-muted-foreground'
})
</script>

<template>
  <article
    class="flex min-w-0 flex-col rounded-xl border border-border/60 bg-card p-4 transition-colors hover:border-primary/30 sm:p-5"
    :aria-labelledby="headingId"
    :data-plugin-card="pluginId"
    :data-plugin-kind="kind"
  >
    <header class="flex min-w-0 items-start gap-3">
      <PluginAvatar :plugin-id="pluginId" />
      <div class="min-w-0 flex-1">
        <h2 :id="headingId" class="break-all font-mono text-sm font-semibold leading-6">{{ pluginId }}</h2>
        <span
          class="mt-1 inline-flex max-w-full items-center gap-1.5 rounded-md px-2 py-0.5 text-[11px] leading-5"
          :class="stateClass"
        >
          <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-current" aria-hidden="true" />
          <span class="break-words">{{ pluginStateLabel(state) }}</span>
        </span>
      </div>
    </header>

    <p v-if="summary" class="mt-4 break-words text-sm leading-relaxed text-muted-foreground">{{ summary }}</p>

    <footer class="mt-auto pt-5">
      <div v-if="pages.length" class="mb-3 space-y-2" data-plugin-app-links>
        <Button
          v-for="page in pages"
          :key="page.page_id"
          variant="secondary"
          size="sm"
          class="h-auto min-h-9 w-full justify-between gap-2 whitespace-normal py-2 text-start"
          data-plugin-open-page
          :data-plugin-app="page.page_id"
          :disabled="state !== 'running'"
          @click="emit('open-page', page)"
        >
          <span class="min-w-0 break-words">{{ $t('pluginWeb.open') }} · {{ page.title }}</span>
          <RiArrowRightLine class="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        </Button>
      </div>
      <div class="flex flex-wrap items-center gap-2 border-t border-border/50 pt-3">
        <Button variant="outline" size="sm" class="min-w-0 flex-1" data-plugin-open-settings @click="emit('settings')">
          <RiSettings3Line class="mr-1.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          {{ $st('Settings') }}
        </Button>
        <Button variant="ghost" size="sm" data-plugin-open-details @click="emit('details')">
          {{ $st('Details') }}
          <RiArrowRightLine class="ml-1.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        </Button>
      </div>
    </footer>
  </article>
</template>
