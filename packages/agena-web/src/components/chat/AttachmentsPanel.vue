<script setup lang="ts">
import { useMobileSheetLayout } from '@/composables/useMobileSheetLayout'
import { createReusableTemplate } from '@vueuse/core'
import { rememberKeyboardOverlaySource } from '@/composables/useDialogReturnFocus'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { RiAttachmentLine, RiCloseLine, RiFileLine, RiFileUploadLine, RiLoader4Line } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import { DialogTitle } from 'radix-vue'

import Button from '@/components/ui/Button.vue'
import { escapeBelongsToPanel } from '@/components/chat/attachmentsPanelScope'
import IconButton from '@/components/ui/IconButton.vue'
import ModalSurface from '@/components/ui/ModalSurface.vue'
import PopoverSurface from '@/components/ui/PopoverSurface.vue'
import Skeleton from '@/components/ui/Skeleton.vue'
import { useUiStore, type ImageViewerItem } from '@/stores/ui'
import type { AttachedFile } from '@/pages/chat/useChatAttachments'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { resolveOverlayAnchorElement, type OverlayAnchor } from '@/lib/overlayAnchor'
import {
  enabledKeyboardItems,
  focusKeyboardItem,
  handleOptionKeydown,
  isVisibleKeyboardTarget,
} from '@/lib/keyboardNavigation'

const props = withDefaults(
  defineProps<{
    open: boolean
    attachedFiles: AttachedFile[]
    formatBytes: (bytes: number) => string
    busy?: boolean
    isCompactTouch?: boolean
    desktopAnchorEl?: OverlayAnchor
    desktopPlacement?: 'top-start' | 'top-end' | 'bottom-start' | 'bottom-end'
    desktopGapPx?: number
    desktopViewportMarginPx?: number
    title?: string
    providerLabel?: string
    allowProject?: boolean
  }>(),
  {
    busy: false,
    isCompactTouch: false,
    desktopAnchorEl: null,
    desktopPlacement: 'top-start',
    desktopGapPx: 8,
    desktopViewportMarginPx: 8,
    title: '',
    allowProject: true,
  },
)

const { t } = useI18n()
const [DefineAttachmentsBody, AttachmentsBody] = createReusableTemplate()
const ui = useUiStore()
const workspacePane = useWorkspacePaneContext()

const effectiveTitle = computed(() => String(props.title || '').trim() || String(t('chat.attachments.title')))

const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'remove', id: string): void
  (e: 'delivery', id: string, delivery: 'reference' | 'model_input'): void
  (e: 'clear'): void
  (e: 'attachLocal'): void
  (e: 'attachProject'): void
}>()

const panelEl = ref<HTMLElement | null>(null)
let returnFocusEl: HTMLElement | null = null
let lastPanelFocus: HTMLElement | null = null
let unmounting = false

const isMobileSheet = computed(() => Boolean(props.isCompactTouch))
const mobileSheetStyle = useMobileSheetLayout(() => props.open && isMobileSheet.value)
const toolbarSize = computed(() => (isMobileSheet.value ? 'sm' : 'xs'))
const panelClass = 'oc-overlay-panel fixed flex flex-col overflow-hidden'

const fileCount = computed(() => (Array.isArray(props.attachedFiles) ? props.attachedFiles.length : 0))
watch(
  [() => props.open, panelEl],
  ([open, panel]) => {
    if (open && panel && returnFocusEl) rememberKeyboardOverlaySource(panel, returnFocusEl)
    if (!open) lastPanelFocus = null
  },
  { flush: 'post' },
)
watch(
  fileCount,
  () => {
    if (!props.open || !lastPanelFocus || document.activeElement !== document.body) return
    if (lastPanelFocus.isConnected && !lastPanelFocus.matches(':disabled')) return
    const [first] = enabledKeyboardItems(panelEl.value)
    focusKeyboardItem(first)
  },
  { flush: 'post' },
)

function onPanelFocusin(event: FocusEvent) {
  if (event.target instanceof HTMLElement) lastPanelFocus = event.target
}

const countLabel = computed(() => {
  const n = fileCount.value
  if (n === 1) return String(t('chat.attachments.countLabelOne'))
  return String(t('chat.attachments.countLabelMany', { count: n }))
})

function isImageFile(f: AttachedFile): boolean {
  const mime = String(f?.mime || '')
    .trim()
    .toLowerCase()
  const url = typeof f?.url === 'string' ? f.url : ''
  return mime.startsWith('image/') && url.startsWith('data:')
}

function imageViewerItemsFromAttachments(files: AttachedFile[]): ImageViewerItem[] {
  const out: ImageViewerItem[] = []
  for (const file of files || []) {
    if (!isImageFile(file)) continue
    const src = String(file.url || '').trim()
    if (!src) continue
    const title = String(file.filename || '').trim()
    const key = String(file.id || file.filename || src).trim()
    out.push({
      src,
      ...(title ? { title, alt: title } : {}),
      ...(key ? { key } : {}),
    })
  }
  return out
}

function openAttachmentImage(file: AttachedFile) {
  if (!isImageFile(file)) return
  const items = imageViewerItemsFromAttachments(props.attachedFiles)
  if (!items.length) return

  const targetKey = String(file.id || file.filename || file.url || '').trim()
  const index = items.findIndex((item) => String(item.key || '').trim() === targetKey)
  ui.openImageViewer(items, index >= 0 ? index : 0)
}

function badgeTextForFilename(filename: string): string {
  const name = String(filename || '').trim()
  const lower = name.toLowerCase()

  if (lower.endsWith('.tar.gz')) return 'TGZ'
  if (lower.endsWith('.tar.bz2')) return 'TBZ2'
  if (lower.endsWith('.tar.xz')) return 'TXZ'

  const dot = name.lastIndexOf('.')
  if (dot <= 0 || dot >= name.length - 1) return 'FILE'
  const ext = name
    .slice(dot + 1)
    .trim()
    .toUpperCase()
  if (!ext) return 'FILE'
  return ext.length > 5 ? ext.slice(0, 5) : ext
}

function close() {
  emit('update:open', false)
}

const desktopAnchor = computed(() => resolveOverlayAnchorElement(props.desktopAnchorEl))

watch(
  () => props.open,
  (open) => {
    if (!open || typeof document === 'undefined') return
    const active = document.activeElement
    returnFocusEl = active instanceof HTMLElement && active !== document.body ? active : desktopAnchor.value
  },
  { immediate: true, flush: 'sync' },
)

function onCloseAutoFocus(event: Event) {
  event.preventDefault()
  if (props.open && !unmounting) return

  const target = returnFocusEl
  returnFocusEl = null
  void nextTick(() => {
    if (props.open && !unmounting) return
    const active = document.activeElement
    if (active instanceof HTMLElement && active !== document.body && active !== document.documentElement) return
    if (target && isVisibleKeyboardTarget(target)) target.focus({ preventScroll: true })
  })
}

function onDesktopInteractOutside(event: CustomEvent<{ originalEvent: Event }>) {
  const target = event.detail.originalEvent.target
  if (!(target instanceof Element)) return
  if (
    target === document.body ||
    desktopAnchor.value?.contains(target) ||
    target.closest('[role="dialog"], [role="alertdialog"], [data-keyboard-overlay]')
  ) {
    event.preventDefault()
  }
}

function onDesktopEscape(event: KeyboardEvent) {
  const target = event.target instanceof Element ? event.target : null
  const overlay = target?.closest('[role="dialog"], [role="alertdialog"], [data-keyboard-overlay]')
  if (
    ui.isImageViewerOpen ||
    (overlay && overlay !== panelEl.value) ||
    (!panelEl.value?.contains(target) &&
      !escapeBelongsToPanel({
        panel: desktopAnchor.value,
        target,
        paneFocused: !workspacePane || workspacePane.isFocused.value,
      }))
  ) {
    event.preventDefault()
  }
}

function onSurfaceOpenChange(open: boolean) {
  if (!open && props.open) close()
}

onBeforeUnmount(() => {
  unmounting = true
})
</script>

<template>
  <DefineAttachmentsBody>
    <header class="flex shrink-0 items-center justify-between gap-3 border-b border-border/50 px-3 py-2">
      <div class="flex min-w-0 items-center gap-2">
        <RiAttachmentLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <div class="min-w-0">
          <component :is="isMobileSheet ? DialogTitle : 'div'" class="truncate text-sm font-semibold">
            {{ effectiveTitle }}
          </component>
          <div class="mt-0.5 text-xs text-muted-foreground" role="status">
            <span v-if="busy" class="inline-flex items-center gap-1.5">
              <RiLoader4Line class="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
              {{ t('chat.attachments.attaching') }}
            </span>
            <span v-else>{{ countLabel }}</span>
          </div>
        </div>
      </div>
      <div class="flex shrink-0 items-center gap-1">
        <Button
          v-if="fileCount > 0"
          data-keyboard-option
          :size="toolbarSize"
          variant="ghost-destructive"
          :class="isMobileSheet && 'min-h-10'"
          @click="emit('clear')"
        >
          {{ t(isMobileSheet ? 'chat.attachments.actions.clear' : 'chat.attachments.actions.clearAll') }}
        </Button>
        <IconButton
          size="sm"
          :class="isMobileSheet && 'size-10'"
          :tooltip="t('common.close')"
          :is-touch-pointer="isMobileSheet"
          :aria-label="t('common.close')"
          @click="close"
        >
          <RiCloseLine class="h-4 w-4" aria-hidden="true" />
        </IconButton>
      </div>
    </header>

    <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden p-3">
      <div class="flex shrink-0 flex-wrap items-center gap-2">
        <Button
          data-attachment-first
          data-keyboard-option
          :size="toolbarSize"
          variant="outline"
          :class="isMobileSheet ? 'min-h-9' : 'min-h-8'"
          @click="emit('attachLocal')"
        >
          <RiFileUploadLine class="me-1.5 h-4 w-4" aria-hidden="true" />
          {{ t('chat.attachments.actions.addFromComputer') }}
        </Button>
        <Button
          v-if="allowProject"
          data-keyboard-option
          :size="toolbarSize"
          variant="outline"
          :class="isMobileSheet ? 'min-h-9' : 'min-h-8'"
          @click="emit('attachProject')"
        >
          <RiFileLine class="me-1.5 h-4 w-4" aria-hidden="true" />
          {{ t('chat.attachments.actions.addFromProject') }}
        </Button>
      </div>

      <div class="min-h-0 flex-1 overflow-auto overscroll-contain" :aria-busy="busy">
        <div v-if="fileCount === 0" class="space-y-2 rounded-lg border border-border/60 bg-muted/15 p-3">
          <template v-if="busy">
            <Skeleton class="h-3 w-2/3" />
            <Skeleton class="h-3 w-1/2" />
            <Skeleton v-for="row in 3" :key="row" class="h-8 w-full" />
          </template>
          <template v-else>
            <div class="text-sm font-medium">{{ t('chat.attachments.empty.title') }}</div>
            <div class="text-xs text-muted-foreground">{{ t('chat.attachments.empty.description') }}</div>
          </template>
        </div>

        <ul v-else class="space-y-2">
          <li
            v-for="f in attachedFiles"
            :key="f.id"
            class="flex items-center gap-2.5 rounded-lg border border-border/60 p-2.5"
          >
            <button
              v-if="isImageFile(f)"
              data-keyboard-option
              type="button"
              class="size-9 shrink-0 overflow-hidden rounded-md border border-border/50 bg-muted/20 outline-none focus-visible:ring-2 focus-visible:ring-ring"
              :aria-label="`${t('common.open')}: ${f.filename}`"
              @click.stop="openAttachmentImage(f)"
            >
              <img :src="f.url" alt="" class="h-full w-full cursor-zoom-in object-cover" draggable="false" />
            </button>
            <div
              v-else
              class="flex size-9 shrink-0 items-center justify-center rounded-md border border-border/50 bg-muted/20"
              aria-hidden="true"
            >
              <span class="font-mono text-[10px] uppercase text-muted-foreground">
                {{ f.composerKind === 'skill_ref' ? 'SKILL' : badgeTextForFilename(f.filename) }}
              </span>
            </div>

            <div class="min-w-0 flex-1">
              <div
                class="truncate font-medium"
                :class="[isMobileSheet ? 'text-sm' : 'text-xs', f.pastePreview === undefined && 'font-mono']"
                :title="f.filename"
              >
                {{ f.pastePreview !== undefined ? t('chat.attachments.pastedText') : f.filename }}
              </div>
              <p
                v-if="f.pastePreview !== undefined"
                class="mt-1 line-clamp-2 break-words text-xs text-muted-foreground"
                :title="f.pastePreview"
              >
                {{ f.pastePreview }}
              </p>
              <label
                v-if="f.composerKind !== 'skill_ref'"
                class="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground"
              >
                <input
                  data-keyboard-option
                  class="accent-primary"
                  type="checkbox"
                  :checked="f.delivery === 'model_input'"
                  :disabled="busy"
                  @change="
                    emit('delivery', f.id, ($event.target as HTMLInputElement).checked ? 'model_input' : 'reference')
                  "
                />
                {{ t('chat.attachments.sendContents') }}
              </label>
              <div
                v-if="f.serverPath"
                class="mt-0.5 truncate font-mono text-[11px] text-muted-foreground"
                :title="f.serverPath"
              >
                {{ f.serverPath }}
              </div>
            </div>

            <span class="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">
              {{ f.serverPath ? t('chat.attachments.repo') : formatBytes(f.size) }}
            </span>
            <IconButton
              data-keyboard-option
              :size="toolbarSize"
              class="shrink-0 text-muted-foreground"
              :tooltip="t('common.remove')"
              :is-touch-pointer="isMobileSheet"
              :title="t('common.remove')"
              :aria-label="t('chat.attachments.removeAttachmentAria')"
              @click="emit('remove', f.id)"
            >
              <RiCloseLine class="h-4 w-4" aria-hidden="true" />
            </IconButton>
          </li>
        </ul>
      </div>
    </div>
  </DefineAttachmentsBody>

  <PopoverSurface
    v-if="!isMobileSheet"
    :open="open"
    :anchor="desktopAnchorEl"
    :layer="60"
    as-child
    :side="desktopPlacement.startsWith('top') ? 'top' : 'bottom'"
    :align="desktopPlacement.endsWith('end') ? 'end' : 'start'"
    :side-offset="Math.max(0, desktopGapPx)"
    :collision-padding="Math.max(0, desktopViewportMarginPx)"
    update-position-strategy="always"
    :prioritize-position="true"
    initial-focus="[data-attachment-first]"
    @update:open="onSurfaceOpenChange"
    @close-auto-focus="onCloseAutoFocus"
    @interact-outside="onDesktopInteractOutside"
    @escape-key-down="onDesktopEscape"
  >
    <div
      ref="panelEl"
      :aria-label="effectiveTitle"
      class="flex w-[min(420px,calc(100vw-1rem))] flex-col overflow-hidden"
      style="max-height: min(var(--radix-popover-content-available-height, 100dvh), 72dvh, 560px)"
      @pointerdown.stop
      @click.stop
      @keydown="handleOptionKeydown($event, panelEl)"
      @focusin="onPanelFocusin"
    >
      <AttachmentsBody />
    </div>
  </PopoverSurface>

  <ModalSurface
    v-else
    :open="open"
    :layer="64"
    as-child
    :aria-describedby="undefined"
    initial-focus="[data-attachment-first]"
    @close-auto-focus="onCloseAutoFocus"
    @update:open="onSurfaceOpenChange"
  >
    <div
      ref="panelEl"
      data-keyboard-overlay="true"
      tabindex="-1"
      :class="[panelClass, 'max-w-[24rem] -translate-x-1/2 -translate-y-1/2']"
      :style="mobileSheetStyle"
      @pointerdown.stop
      @click.stop
      @keydown="handleOptionKeydown($event, panelEl)"
      @focusin="onPanelFocusin"
    >
      <AttachmentsBody />
    </div>
  </ModalSurface>
</template>
