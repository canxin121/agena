<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCheckLine, RiMore2Line } from '@remixicon/vue'

import MiniActionButton from '@/components/ui/MiniActionButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'

type Gitmoji = { emoji: string; label: string }

const { t } = useI18n()

const props = defineProps<{
  message: string
  committing: boolean
  gitmojiEnabled: boolean
  gitmojis: Gitmoji[]
  selectedGitmoji: string
  stagedCount?: number
  pendingCount?: number
  busy?: boolean
  amend?: boolean
}>()

const emit = defineEmits<{
  (e: 'update:message', value: string): void
  (e: 'update:selectedGitmoji', value: string): void
  (e: 'insertGitmoji'): void
  (e: 'messageKeydown', ev: KeyboardEvent): void
  (e: 'commit'): void
  (e: 'stageAll'): void
  (e: 'options'): void
}>()

function updateMessage(ev: Event) {
  const el = ev.target as HTMLTextAreaElement | null
  emit('update:message', el?.value ?? '')
}

function updateSelectedGitmoji(value: string | number) {
  const v = String(value || '')
  emit('update:selectedGitmoji', v)
  if (v) emit('insertGitmoji')
}

const canCommit = computed(
  () =>
    Boolean(props.message.trim()) &&
    !props.committing &&
    !props.busy &&
    (props.stagedCount === undefined || props.stagedCount > 0 || props.amend),
)

function submit() {
  if (canCommit.value) emit('commit')
}

const gitmojiPickerOptions = computed(() =>
  (Array.isArray(props.gitmojis) ? props.gitmojis : []).map((g) => ({
    value: g.emoji,
    label: `${g.emoji} ${g.label}`,
  })),
)
</script>

<template>
  <div class="git-commit-composer rounded-lg border border-border/65 bg-background/45 p-2">
    <textarea
      :value="message"
      rows="2"
      class="min-h-14 max-h-40 w-full resize-y rounded-md border-0 bg-transparent px-1 py-1 text-xs leading-relaxed shadow-none focus-visible:outline-none"
      :aria-label="t('git.ui.commitBox.messagePlaceholder')"
      :placeholder="t('git.ui.commitBox.messagePlaceholder')"
      @input="updateMessage"
      @keydown.meta.enter.prevent="submit"
      @keydown.ctrl.enter.prevent="submit"
      @keydown="$emit('messageKeydown', $event)"
    />

    <div class="mt-1 flex min-w-0 items-center gap-2 border-t border-border/40 pt-2">
      <span
        v-if="stagedCount !== undefined"
        class="min-w-0 flex-1 truncate text-[11px] text-muted-foreground"
        :title="stagedCount ? t('git.sidebar.stagedHint', { count: stagedCount }) : t('git.sidebar.noStaged')"
      >
        {{ stagedCount ? t('git.sidebar.stagedHint', { count: stagedCount }) : t('git.sidebar.noStaged') }}
      </span>
      <button
        v-if="!stagedCount && pendingCount"
        type="button"
        class="shrink-0 text-[11px] text-primary hover:underline disabled:opacity-50"
        :disabled="committing || busy"
        @click="emit('stageAll')"
      >
        {{ t('git.sidebar.stageAll') }}
      </button>
      <div v-if="gitmojiEnabled" class="flex items-center gap-1">
        <div class="w-24">
          <OptionPicker
            :model-value="selectedGitmoji"
            :options="gitmojiPickerOptions"
            :title="t('git.ui.commitBox.gitmojiTitle')"
            :search-placeholder="t('git.ui.commitBox.searchEmojis')"
            :empty-label="t('git.ui.commitBox.emojiEmptyLabel')"
            :empty-disabled="true"
            trigger-class="h-6 px-1 text-[10px] rounded-sm border-sidebar-border/70 bg-sidebar-accent/20 shadow-none"
            size="sm"
            @update:model-value="updateSelectedGitmoji"
          />
        </div>
      </div>

      <button
        type="button"
        class="inline-flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted"
        :aria-label="t('git.sidebar.moreActions')"
        :title="t('git.sidebar.moreActions')"
        @click="emit('options')"
      >
        <RiMore2Line class="size-3.5" />
      </button>
      <MiniActionButton variant="default" class="ml-auto shrink-0 gap-1 px-2.5" :disabled="!canCommit" @click="submit">
        <RiCheckLine class="size-3.5" />
        {{ t('git.actionsMenu.groups.commit') }}
      </MiniActionButton>
    </div>
  </div>
</template>
