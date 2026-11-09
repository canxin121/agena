<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowDownSLine, RiSettings3Line, RiRobot2Line } from '@remixicon/vue'
import { useBotsStore } from '@/stores/bots'
import type { BotProfile } from '@/lib/bots'
const props = defineProps<{ botId: string | null; name: string; disabled?: boolean }>()
defineEmits<{ select: [id: string]; updated: [profile: BotProfile]; unavailable: [] }>()
const { t } = useI18n()
const bots = useBotsStore()
const initials = computed(() => Array.from(props.name.trim()).slice(0, 2).join('').toLocaleUpperCase())
function edit(id = props.botId) {
  if (id) bots.open('edit', id)
}
defineExpose({ edit })
</script>

<template>
  <div class="flex w-full min-w-0 items-center gap-2" data-testid="bot-controls" data-transcript-keyboard-owner="true">
    <button
      type="button"
      class="group flex min-h-11 min-w-0 flex-1 items-center gap-2.5 rounded-xl border border-primary/20 bg-gradient-to-br from-primary/10 to-primary/5 px-2.5 py-1.5 text-start transition-colors hover:border-primary/40 hover:bg-primary/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      :aria-label="name ? `${t('bot.profiles.choose')}: ${name}` : t('bot.profiles.choose')"
      aria-haspopup="dialog"
      :aria-expanded="bots.managerOpen"
      data-testid="bot-selector"
      @click="bots.open('manager')"
    >
      <span
        class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-xs font-semibold text-primary"
        aria-hidden="true"
        ><span v-if="initials">{{ initials }}</span
        ><RiRobot2Line v-else class="h-5 w-5"
      /></span>
      <span class="min-w-0 flex-1"
        ><span class="block truncate text-sm font-semibold" data-testid="bot-name">{{
          name || t('bot.profiles.choose')
        }}</span
        ><span class="block truncate text-[10px] text-muted-foreground">{{ t('bot.profiles.identity') }}</span></span
      >
      <RiArrowDownSLine class="h-4 w-4 shrink-0 text-muted-foreground" />
    </button>
    <button
      type="button"
      class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-border/70 hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40"
      :disabled="!botId || disabled"
      :title="t('bot.profiles.edit')"
      :aria-label="t('bot.profiles.edit')"
      data-testid="bot-settings"
      @click="edit()"
    >
      <RiSettings3Line class="h-4 w-4" />
    </button>
  </div>
</template>
