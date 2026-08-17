<script setup lang="ts">
import { ref } from 'vue'
import { RiArrowDownLine, RiEditLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import AttachmentPicker from '@/components/chat/AttachmentPicker.vue'

const props = defineProps<{
  draft: string
  fullscreen: boolean
  modeLabel?: string
}>()

const emit = defineEmits<{
  (e: 'update:draft', value: string): void
  (e: 'toggleFullscreen'): void
  (e: 'drop', ev: DragEvent): void
  (e: 'paste', ev: ClipboardEvent): void
  (e: 'draftInput'): void
  (e: 'draftKeydown', ev: KeyboardEvent): void
  (e: 'filesSelected', files: FileList): void
}>()

const shellEl = ref<HTMLDivElement | null>(null)
const textareaEl = ref<HTMLTextAreaElement | null>(null)
const attachmentPickerRef = ref<InstanceType<typeof AttachmentPicker> | null>(null)

const { t } = useI18n()

function updateDraft(ev: Event) {
  const el = ev.target as HTMLTextAreaElement | null
  emit('update:draft', el?.value ?? '')
}

function openFilePicker() {
  attachmentPickerRef.value?.openFilePicker()
}

defineExpose({ shellEl, textareaEl, openFilePicker })
</script>

<template>
  <div
    ref="shellEl"
    class="composer-shell relative flex flex-col overflow-visible rounded-xl border border-input bg-background/85 shadow-sm"
    :class="fullscreen ? 'composer-fullscreen rounded-none' : ''"
    data-oc-keyboard-tap="keep"
    @dragover.prevent
    @drop.prevent="$emit('drop', $event)"
  >
    <div
      v-if="$slots.status"
      class="pointer-events-none absolute left-2 top-0 z-10 flex max-w-[calc(100%-3.5rem)] -translate-y-1/2 items-center gap-1.5 bg-background px-1 font-mono text-[10px]"
    >
      <slot name="status" />
    </div>

    <div
      v-if="$slots.topRight"
      class="pointer-events-none absolute right-2 top-0 z-10 flex max-w-[calc(100%-3.5rem)] -translate-y-1/2 items-center gap-1.5 bg-background px-1 font-mono text-[10px]"
    >
      <slot name="topRight" />
    </div>

    <div
      v-if="$slots.bottomLeft"
      class="pointer-events-none absolute bottom-0 left-2 z-10 flex max-w-[calc(100%-3.5rem)] translate-y-1/2 items-center gap-1.5 bg-background px-1 font-mono text-[10px]"
    >
      <slot name="bottomLeft" />
    </div>

    <div
      v-if="$slots.bottomRight"
      class="pointer-events-none absolute bottom-0 right-2 z-10 flex max-w-[calc(100%-3.5rem)] translate-y-1/2 items-center gap-1.5 bg-background px-1 font-mono text-[10px]"
    >
      <slot name="bottomRight" />
    </div>

    <slot name="overlay" />

    <div class="absolute top-1 right-1 z-10 flex items-center gap-1">
      <button
        type="button"
        :data-oc-keyboard-tap="fullscreen ? 'blur' : 'keep'"
        class="flex h-6 w-6 items-center justify-center rounded-md text-muted-foreground/80 hover:bg-secondary/60 hover:text-foreground"
        :title="fullscreen ? t('chat.composer.editor.collapse') : t('chat.composer.editor.open')"
        :aria-label="fullscreen ? t('chat.composer.editor.collapse') : t('chat.composer.editor.open')"
        @pointerdown.prevent
        @click="$emit('toggleFullscreen')"
      >
        <component :is="fullscreen ? RiArrowDownLine : RiEditLine" class="h-4 w-4" />
      </button>
    </div>

    <textarea
      ref="textareaEl"
      :value="draft ?? ''"
      data-chat-input="true"
      class="w-full min-h-[44px] flex-1 resize-none border-0 bg-transparent px-3 pb-2 pt-3 text-sm shadow-none placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-0 sm:min-h-[60px]"
      :class="fullscreen ? 'composer-textarea-full' : 'max-h-none'"
      :placeholder="t('chat.composer.input.placeholder')"
      spellcheck="false"
      @input="
        (ev) => {
          updateDraft(ev)
          $emit('draftInput')
        }
      "
      @click="$emit('draftInput')"
      @keyup="$emit('draftInput')"
      @paste="$emit('paste', $event)"
      @keydown="$emit('draftKeydown', $event)"
    />

    <AttachmentPicker ref="attachmentPickerRef" @filesSelected="$emit('filesSelected', $event)" />

    <slot name="controls" />
  </div>
</template>

<style scoped>
.composer-shell.composer-fullscreen {
  background-color: oklch(var(--background));
  height: 100%;
  flex: 1;
  min-height: 0;
}

.composer-textarea-full {
  flex: 1;
  min-height: 0;
  max-height: none;
}
</style>
