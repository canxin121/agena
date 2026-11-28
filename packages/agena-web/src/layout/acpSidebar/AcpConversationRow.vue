<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiArchiveLine,
  RiDeleteBinLine,
  RiInboxArchiveLine,
  RiInboxUnarchiveLine,
  RiLoader4Line,
} from '@remixicon/vue'

import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import InlineNameEditor from '@/components/ui/InlineNameEditor.vue'
import ListItemOverflowActionButton from '@/components/ui/ListItemOverflowActionButton.vue'
import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import { formatTime } from '@/features/sessions/model/labels'
import { acpSessionStatusDot, type AcpConversationRow } from './acpSidebarModel'

const { t } = useI18n()

const props = withDefaults(
  defineProps<{
    row: AcpConversationRow
    selected: boolean
    uiIsCompactLayout: boolean
    opening?: boolean
    busy?: boolean
    canRename?: boolean
    canDelete?: boolean
    renaming?: boolean
    renameDraft?: string
    renameBusy?: boolean
  }>(),
  {
    opening: false,
    busy: false,
    canRename: false,
    canDelete: false,
    renaming: false,
    renameDraft: '',
    renameBusy: false,
  },
)

const emit = defineEmits<{
  (e: 'open'): void
  (e: 'open-menu', event: MouseEvent | PointerEvent): void
  (e: 'toggle-archive'): void
  (e: 'delete'): void
  (e: 'request-rename'): void
  (e: 'update:renameDraft', value: string): void
  (e: 'rename-save'): void
  (e: 'rename-cancel'): void
}>()

const statusMeta = computed(() => acpSessionStatusDot(props.row.state))
const isInlineRename = computed(() => props.renaming && !props.uiIsCompactLayout)
const isBusyState = computed(() => props.row.state === 'running' || props.row.state === 'requires_action')

const canQuickArchive = computed(() => {
  if (!props.row.nativeId || !props.row.capabilities?.archive) return false
  if (props.row.capabilities.index && isBusyState.value) return false
  return true
})

const canQuickDelete = computed(() => props.canDelete && !isBusyState.value)

function handleContextMenu(event: MouseEvent) {
  if (isInlineRename.value) return
  event.preventDefault()
  event.stopPropagation()
  emit('open-menu', event)
}

function handleRowClick() {
  if (isInlineRename.value) return
  emit('open')
}

function handleKeyboardRename(event: Event) {
  if (!props.canRename || props.renaming || props.busy) return
  event.preventDefault()
  emit('request-rename')
}
</script>

<template>
  <SidebarListItem
    :data-keyboard-item="`acp-session:${row.key}`"
    :data-keyboard-label="row.title"
    data-keyboard-level="2"
    :data-keyboard-selected="String(selected)"
    :active="selected"
    :indent="12"
    :as="isInlineRename ? 'div' : 'button'"
    :actions-always-visible="isInlineRename || uiIsCompactLayout"
    :disabled="busy"
    class="gap-2 relative"
    @click="handleRowClick"
    @keyboard-rename="handleKeyboardRename"
    @contextmenu="handleContextMenu"
  >
    <template #icon>
      <RiLoader4Line
        v-if="opening"
        class="h-3.5 w-3.5 flex-shrink-0 animate-spin text-primary"
        aria-hidden="true"
      />
      <span
        v-else-if="statusMeta.dotClass"
        class="inline-flex h-1.5 w-1.5 rounded-full flex-shrink-0"
        :class="statusMeta.dotClass"
        :title="String(t(statusMeta.labelKey))"
        :aria-label="String(t(statusMeta.labelKey))"
      />
    </template>

    <div class="flex w-full items-center min-w-0 gap-2">
      <template v-if="!isInlineRename">
        <div class="flex min-w-0 flex-1 items-center gap-1.5">
          <RiArchiveLine
            v-if="row.archived"
            class="h-3 w-3 flex-shrink-0 text-muted-foreground/70"
            :title="String(t('chat.nativeAgent.archived'))"
          />
          <span class="min-w-0 flex-1 truncate typography-ui-label text-left" :title="row.title">
            {{ row.title }}
          </span>
        </div>

        <span
          v-if="row.updatedAtMs > 0"
          class="ml-auto font-mono text-[10px] text-muted-foreground/60 flex-shrink-0"
        >
          {{ formatTime(row.updatedAtMs) }}
        </span>
      </template>

      <InlineNameEditor
        v-else
        :model-value="renameDraft"
        :label="t('chat.sidebar.sessionActions.rename.label')"
        :placeholder="t('chat.sidebar.sessionRow.placeholders.sessionTitle')"
        :busy="renameBusy"
        :busy-label="t('chat.sidebar.sessionRow.rename.saving')"
        :submit-label="t('chat.sidebar.sessionRow.rename.save')"
        :cancel-label="t('chat.sidebar.sessionRow.rename.cancel')"
        @update:model-value="emit('update:renameDraft', $event)"
        @submit="emit('rename-save')"
        @cancel="emit('rename-cancel')"
      />
    </div>

    <template #actions>
      <template v-if="!isInlineRename">
        <ListItemOverflowActionButton
          data-keyboard-menu
          :mobile="uiIsCompactLayout"
          :label="String(t('chat.sidebar.sessionActions.menuTitle'))"
          @trigger="emit('open-menu', $event)"
        />

        <IconButton
          v-if="!uiIsCompactLayout && canQuickArchive"
          size="xs"
          class="text-muted-foreground hover:text-foreground hover:dark:bg-accent/40 hover:bg-primary/6"
          :title="String(t(row.archived ? 'chat.nativeAgent.unarchive' : 'chat.nativeAgent.archive'))"
          :aria-label="String(t(row.archived ? 'chat.nativeAgent.unarchive' : 'chat.nativeAgent.archive'))"
          :disabled="busy"
          @click.stop="emit('toggle-archive')"
        >
          <RiInboxUnarchiveLine v-if="row.archived" class="h-4 w-4" />
          <RiInboxArchiveLine v-else class="h-4 w-4" />
        </IconButton>

        <ConfirmPopover
          v-if="!uiIsCompactLayout && canQuickDelete"
          :title="String(t(row.kind === 'draft' ? 'chat.sidebar.sessionActions.delete.confirmTitle' : 'chat.nativeAgent.delete'))"
          :description="String(t(row.kind === 'draft' ? 'chat.sidebar.sessionActions.delete.confirmDescription' : 'chat.nativeAgent.deleteDescription'))"
          :confirm-text="String(t('common.delete'))"
          :cancel-text="String(t('common.cancel'))"
          variant="destructive"
          @confirm="emit('delete')"
        >
          <IconButton
            size="xs"
            class="text-muted-foreground hover:text-destructive hover:dark:bg-accent/40 hover:bg-primary/6"
            :title="String(t('chat.sidebar.sessionActions.delete.label'))"
            :aria-label="String(t('chat.sidebar.sessionActions.delete.label'))"
            :disabled="busy"
            @click.stop
          >
            <RiDeleteBinLine class="h-4 w-4" />
          </IconButton>
        </ConfirmPopover>
      </template>
    </template>
  </SidebarListItem>
</template>
