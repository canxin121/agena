<script setup lang="ts">
import { computed, ref, useId } from 'vue'
import { RiApps2Line, RiArrowDownSLine, RiArrowRightLine, RiSettings3Line } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import PluginAvatar from '@/components/settings/plugins/PluginAvatar.vue'
import { pluginStateLabel } from '@/components/settings/pluginLabels'
import type { PluginInteractionPage } from '@/features/pluginWeb/settingsEntry'
import { useUiStore } from '@/stores/ui'

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
const ui = useUiStore()
const interactionAnchor = ref<HTMLElement | null>(null)
const interactionMenuOpen = ref(false)
const interactionGroups = computed<OptionMenuGroup[]>(() => [
  {
    items: props.pages.map((page) => ({
      id: page.page_id,
      label: page.title,
      disabled: props.state !== 'running',
    })),
  },
])
const stateClass = computed(() => {
  if (props.state === 'running') return 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
  if (props.state === 'restarting') return 'bg-amber-500/10 text-amber-700 dark:text-amber-400'
  if (props.state === 'failed') return 'bg-destructive/10 text-destructive'
  return 'bg-muted text-muted-foreground'
})

function openInteraction() {
  if (props.state !== 'running') return
  if (props.pages.length === 1) emit('open-page', props.pages[0]!)
  else interactionMenuOpen.value = !interactionMenuOpen.value
}

function selectInteraction(item: OptionMenuItem) {
  const page = props.pages.find((page) => page.page_id === item.id)
  if (page && props.state === 'running') emit('open-page', page)
}
</script>

<template>
  <article
    class="flex h-56 min-w-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-card p-4 transition-colors hover:border-primary/30 sm:p-5"
    :aria-labelledby="headingId"
    :data-plugin-card="pluginId"
    :data-plugin-kind="kind"
  >
    <header class="flex min-w-0 shrink-0 items-start gap-3">
      <PluginAvatar :plugin-id="pluginId" />
      <div class="min-w-0 flex-1">
        <h2 :id="headingId" class="truncate font-mono text-sm font-semibold leading-6" :title="pluginId">
          {{ pluginId }}
        </h2>
        <span
          class="mt-1 inline-flex max-w-full items-center gap-1.5 rounded-md px-2 py-0.5 text-[11px] leading-5"
          :class="stateClass"
        >
          <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-current" aria-hidden="true" />
          <span class="min-w-0 truncate">{{ pluginStateLabel(state) }}</span>
        </span>
      </div>
    </header>

    <p
      v-if="summary"
      class="mt-3 line-clamp-3 shrink-0 break-words text-sm leading-5 text-muted-foreground"
      :title="summary"
    >
      {{ summary }}
    </p>

    <footer class="mt-auto flex shrink-0 items-center gap-2 border-t border-border/50 pt-3">
      <div v-if="pages.length" ref="interactionAnchor" class="min-w-0 flex-1" data-plugin-app-links>
        <Button
          variant="secondary"
          size="sm"
          class="w-full min-w-0 gap-1.5 px-2"
          data-plugin-interaction
          :data-plugin-open-page="pages.length === 1 ? '' : undefined"
          :data-plugin-app="pages.length === 1 ? pages[0]!.page_id : undefined"
          :aria-label="
            pages.length === 1 ? `${$t('pluginWeb.open')} · ${pages[0]!.title}` : $t('pluginWeb.interactivePages')
          "
          :aria-haspopup="pages.length > 1 ? 'dialog' : undefined"
          :aria-expanded="pages.length > 1 ? interactionMenuOpen : undefined"
          :disabled="state !== 'running'"
          @click="openInteraction"
        >
          <RiApps2Line class="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          <span class="min-w-0 truncate">{{ $t('pluginWeb.openApp') }}</span>
          <RiArrowDownSLine v-if="pages.length > 1" class="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        </Button>
      </div>
      <Button
        variant="outline"
        size="sm"
        class="min-w-0 flex-1 gap-1.5 px-2"
        data-plugin-open-settings
        @click="emit('settings')"
      >
        <RiSettings3Line class="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span class="min-w-0 truncate">{{ $st('Settings') }}</span>
      </Button>
      <Button
        variant="ghost"
        size="sm"
        class="min-w-0 max-w-[30%] shrink-0 gap-1 px-2"
        data-plugin-open-details
        @click="emit('details')"
      >
        <span class="min-w-0 truncate">{{ $st('Details') }}</span>
        <RiArrowRightLine class="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      </Button>
      <OptionMenu
        v-if="pages.length > 1"
        v-model:open="interactionMenuOpen"
        :groups="interactionGroups"
        :title="$t('pluginWeb.interactivePages')"
        :searchable="false"
        :is-compact-touch="ui.isCompactTouch"
        :desktop-anchor-el="interactionAnchor"
        @select="selectInteraction"
      />
    </footer>
  </article>
</template>
