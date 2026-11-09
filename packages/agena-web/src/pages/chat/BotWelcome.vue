<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { RiRobot2Line, RiArrowRightUpLine, RiFolderAddLine, RiChatHistoryLine, RiCompass3Line } from '@remixicon/vue'
const { t } = useI18n()
defineProps<{ name: string }>()
defineEmits<{ suggestion: [text: string] }>()
const suggestions = [
  { key: 'explore', icon: RiCompass3Line },
  { key: 'create', icon: RiFolderAddLine },
  { key: 'continue', icon: RiChatHistoryLine },
] as const
</script>

<template>
  <div class="mx-auto max-w-2xl px-2 py-6 sm:py-10" data-testid="bot-welcome">
    <div
      class="mb-4 flex h-12 w-12 items-center justify-center rounded-2xl border border-primary/20 bg-gradient-to-br from-primary/15 to-primary/5 text-primary shadow-sm"
    >
      <RiRobot2Line class="h-6 w-6" />
    </div>
    <h1 class="text-xl font-semibold tracking-tight sm:text-2xl">{{ t('bot.profiles.welcome', { name }) }}</h1>
    <p class="mt-2 max-w-lg text-sm leading-relaxed text-muted-foreground">{{ t('bot.description') }}</p>
    <div class="mt-5 grid gap-2 sm:grid-cols-3">
      <button
        v-for="item in suggestions"
        :key="item.key"
        type="button"
        :data-bot-suggestion="item.key"
        class="group flex min-h-12 items-center gap-2.5 rounded-xl border border-border/80 bg-muted/20 px-3 py-3 text-start text-xs font-medium transition-colors hover:border-primary/30 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:flex-wrap"
        @click="$emit('suggestion', String(t(`bot.prompts.${item.key}`)))"
      >
        <component :is="item.icon" class="h-4 w-4 shrink-0 text-primary/80" />
        <span class="min-w-0 flex-1">{{ t(`bot.suggestions.${item.key}`) }}</span>
        <RiArrowRightUpLine class="h-3.5 w-3.5 shrink-0 text-muted-foreground group-hover:text-primary" />
      </button>
    </div>
    <p class="mt-3 text-xs leading-relaxed text-muted-foreground">{{ t('bot.suggestionHint') }}</p>
  </div>
</template>
