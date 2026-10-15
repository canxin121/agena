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
    isCompactTouch?: boolean
    mobileActionItems?: OptionMenuItem[]
    mobileActionTitle?: string
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
    isCompactTouch: false,
    mobileActionItems: () => [],
    mobileActionTitle: '',
    showSelection: false,
    selected: false,
  },
)

const { t } = useI18n()
const gitPane = inject(gitPaneContextKey, null)

const emit = defineEmits<{
  (e: 'select', event: MouseEvent): void
  (e: 'mobileAction', id: string): void
}>()

const hasStat = computed(() => (props.insertions ?? 0) > 0 || (props.deletions ?? 0) > 0)
const fileName = computed(() => props.path.split(/[\\/]/).pop() || props.path)
const parentPath = computed(() => props.path.slice(0, props.path.length - fileName.value.length).replace(/[\\/]$/, ''))
const mobileActionMenuOpen = ref(false)
const mobileActionMenuQuery = ref('')
const actionAnchor = ref<HTMLElement | null>(null)
const isMobileFormFactor = computed(() => Boolean(props.isMobileFormFactor ?? props.isCompactTouch))

const hasMobileActions = computed(() => Array.isArray(props.mobileActionItems) && props.mobileActionItems.length > 0)
const primaryAction = computed(() =>
  props.mobileActionItems.find((item) => item.id === 'stage' || item.id === 'unstage'),
)

const mobileActionTitleText = computed(
  () => (props.mobileActionTitle || '').trim() || t('git.ui.workingTree.fileActionsTitle'),
)

const mobileActionGroups = computed<OptionMenuGroup[]>(() => {
  if (!hasMobileActions.value) return []
  return [
    {
      id: 'git-status-item-actions',
      items: props.mobileActionItems,
    },
  ]
})

function openMobileActionMenu() {
  if (!hasMobileActions.value) return
  mobileActionMenuQuery.value = ''
  mobileActionMenuOpen.value = true
}

function setMobileActionMenuOpen(open: boolean) {
  mobileActionMenuOpen.value = open
  if (!open) {
    mobileActionMenuQuery.value = ''
  }
}

function onMobileActionSelect(item: OptionMenuItem) {
  emit('mobileAction', item.id)
}

function handleMobileActionTrigger() {
  openMobileActionMenu()
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
    :data-keyboard-multiselect="String(showSelection)"
    :primary-attrs="{ 'aria-pressed': showSelection ? selected : undefined }"
    draggable="true"
    :active="active"
    :action-visibility="isMobileFormFactor ? 'always' : 'hover'"
    class="git-file-row select-none"
    :title="path"
    @click="emit('select', $event)"
    @contextmenu.prevent="openMobileActionMenu"
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
      <template v-if="hasMobileActions">
        <IconButton
          v-if="primaryAction && !showSelection && !isMobileFormFactor"
          size="xs"
          :title="primaryAction.label"
          :aria-label="primaryAction.label"
          :disabled="primaryAction.disabled"
          @click.stop="emit('mobileAction', primaryAction.id)"
        >
          <component :is="primaryAction.icon" class="size-3.5" />
        </IconButton>
        <span ref="actionAnchor" class="inline-flex">
          <ListItemOverflowActionButton
            data-keyboard-menu
            :mobile="isMobileFormFactor"
            :label="mobileActionTitleText"
            @trigger="handleMobileActionTrigger"
          />
        </span>

        <OptionMenu
          :open="mobileActionMenuOpen"
          :query="mobileActionMenuQuery"
          :groups="mobileActionGroups"
          :title="mobileActionTitleText"
          :mobile-title="mobileActionTitleText"
          :searchable="true"
          :is-compact-touch="isMobileFormFactor"
          :desktop-fixed="true"
          :desktop-anchor-el="actionAnchor"
          desktop-placement="bottom-end"
          @update:open="setMobileActionMenuOpen"
          @update:query="(v) => (mobileActionMenuQuery = v)"
          @select="onMobileActionSelect"
        />
      </template>

      <slot v-else name="actions" />
    </template>
  </ListItemFrame>
</template>
