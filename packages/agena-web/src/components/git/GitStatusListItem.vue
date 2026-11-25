<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import { gitPaneContextKey } from '@/features/git/gitPaneContext'
import { fsPathStartsWith } from '@/lib/path'
import { RiFileTextLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import ListItemFrame from '@/components/ui/ListItemFrame.vue'
import ListItemOverflowActionButton from '@/components/ui/ListItemOverflowActionButton.vue'
import ListItemSelectionIndicator from '@/components/ui/ListItemSelectionIndicator.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import type { GitFileActionId } from './gitWorkingTreeActions'
import { writeWorkspaceWindowTemplateToDataTransfer } from '@/layout/workspaceWindowDrag'

const props = withDefaults(
  defineProps<{
    path: string
    active?: boolean
    statusLabel?: string
    statusClass?: string
    insertions?: number
    deletions?: number
    isMobileFormFactor?: boolean
    actions: Array<OptionMenuItem & { id: GitFileActionId }>
    showSelection?: boolean
    selected?: boolean
  }>(),
  {
    active: false,
    statusLabel: '',
    statusClass: 'text-muted-foreground',
    insertions: 0,
    deletions: 0,
    isMobileFormFactor: false,
    showSelection: false,
    selected: false,
  },
)

const { t } = useI18n()
const gitPane = inject(gitPaneContextKey, null)

const emit = defineEmits<{
  (e: 'select', event: MouseEvent): void
  (e: 'action', id: GitFileActionId): void
}>()

const hasStat = computed(() => (props.insertions ?? 0) > 0 || (props.deletions ?? 0) > 0)
const fileName = computed(() => props.path.split(/[\\/]/).pop() || props.path)
const parentPath = computed(() => props.path.slice(0, props.path.length - fileName.value.length).replace(/[\\/]$/, ''))
const actionMenuOpen = ref(false)
const actionMenuQuery = ref('')
const actionAnchor = ref<HTMLElement | null>(null)
const primaryAction = computed(() => props.actions.find((item) => item.id === 'stage' || item.id === 'unstage'))
const actionTitle = computed(() => t('git.ui.workingTree.fileActionsTitle'))
const actionGroups = computed<OptionMenuGroup[]>(() => [{ id: 'git-status-item-actions', items: props.actions }])

function openActionMenu() {
  actionMenuQuery.value = ''
  actionMenuOpen.value = true
}

function setActionMenuOpen(open: boolean) {
  actionMenuOpen.value = open
  if (!open) actionMenuQuery.value = ''
}

function onActionSelect(item: OptionMenuItem) {
  const action = props.actions.find(({ id }) => id === item.id)
  if (action) emit('action', action.id)
}

function handleRowDragStart(event: DragEvent) {
  const path = String(props.path || '').trim()
  if (!path) {
    event.preventDefault()
    return
  }

  const transfer = event.dataTransfer
  if (!transfer) {
    event.preventDefault()
    return
  }

  transfer.effectAllowed = 'copyMove'
  const fileName = path.split(/[\\/]/).filter(Boolean).pop() || path
  const root = gitPane?.repoRoot.value || ''
  const filePath = root ? `${root.replace(/\/+$/, '')}/${path.replace(/^\/+/, '')}` : path
  const project = gitPane?.projectRoot.value || ''
  const directory = fsPathStartsWith(filePath, project) ? project : root
  const ok = writeWorkspaceWindowTemplateToDataTransfer(transfer, {
    tab: 'files',
    query: { directory, filePath },
    title: fileName,
    matchKeys: ['directory', 'filePath'],
  })
  if (!ok) {
    event.preventDefault()
  }
}
</script>

<template>
  <ListItemFrame
    :data-keyboard-item="path"
    :data-keyboard-label="path"
    :data-keyboard-selected="String(showSelection ? selected : active)"
    :data-keyboard-multiselect="String(showSelection)"
    :primary-attrs="{
      'aria-pressed': showSelection ? selected : undefined,
      'aria-current': active ? 'true' : undefined,
    }"
    draggable="true"
    :active="active"
    :action-visibility="isMobileFormFactor ? 'always' : 'hover'"
    class="git-file-row select-none"
    :title="path"
    @click="emit('select', $event)"
    @contextmenu.prevent="openActionMenu"
    @dragstart="handleRowDragStart"
  >
    <template #leading>
      <div class="flex items-center gap-1.5">
        <ListItemSelectionIndicator v-if="showSelection" :selected="selected" />
        <RiFileTextLine class="h-3.5 w-3.5" />
      </div>
    </template>

    <div class="flex w-full min-w-0 items-center gap-2">
      <div class="min-w-0 flex-1">
        <div class="truncate text-xs text-foreground/90">{{ fileName }}</div>
        <div v-if="parentPath" class="truncate text-[10px] leading-4 text-muted-foreground/75">{{ parentPath }}</div>
      </div>
      <span v-if="statusLabel" class="shrink-0 font-mono text-[10px]" :class="statusClass">{{ statusLabel }}</span>
    </div>

    <template #meta>
      <span v-if="hasStat" class="inline-flex items-center justify-end gap-1 font-mono text-[10px] tabular-nums">
        <span v-if="insertions > 0" class="text-emerald-600 dark:text-emerald-400">+{{ insertions }}</span>
        <span v-if="deletions > 0" class="text-rose-600 dark:text-rose-400">−{{ deletions }}</span>
      </span>
    </template>

    <template #actions>
      <IconButton
        v-if="primaryAction && !showSelection && !isMobileFormFactor"
        size="xs"
        :title="primaryAction.label"
        :aria-label="primaryAction.label"
        :disabled="primaryAction.disabled"
        @click.stop="emit('action', primaryAction.id)"
      >
        <component :is="primaryAction.icon" class="size-3.5" />
      </IconButton>
      <span ref="actionAnchor" class="inline-flex">
        <ListItemOverflowActionButton
          data-keyboard-menu
          :mobile="isMobileFormFactor"
          :label="actionTitle"
          @trigger="openActionMenu"
        />
      </span>

      <OptionMenu
        :open="actionMenuOpen"
        :query="actionMenuQuery"
        :groups="actionGroups"
        :title="actionTitle"
        :mobile-title="actionTitle"
        :searchable="true"
        :is-compact-touch="isMobileFormFactor"
        :desktop-fixed="true"
        :desktop-anchor-el="actionAnchor"
        desktop-placement="bottom-end"
        @update:open="setActionMenuOpen"
        @update:query="(v) => (actionMenuQuery = v)"
        @select="onActionSelect"
      />
    </template>
  </ListItemFrame>
</template>
