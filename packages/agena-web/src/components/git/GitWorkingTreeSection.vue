<script setup lang="ts">
import { computed } from 'vue'
import { createReusableTemplate } from '@vueuse/core'
import { useI18n } from 'vue-i18n'
import { RiListCheck3 } from '@remixicon/vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import SectionToggleButton from '@/components/ui/SectionToggleButton.vue'
import SelectionToolbar from '@/components/ui/SelectionToolbar.vue'
import SidebarIconButton from '@/components/ui/SidebarIconButton.vue'
import type { OptionMenuItem } from '@/components/ui/optionMenu.types'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import type { GitStatusFile } from '@/types/git'
import GitStatusListItem from './GitStatusListItem.vue'
import {
  gitFileActions,
  gitSectionActions,
  gitWorkingTreeSections,
  type GitActionDefinition,
  type GitFileActionId,
  type GitSectionActionId,
  type GitWorkingTreeSectionKind,
} from './gitWorkingTreeActions'

const props = defineProps<{
  kind: GitWorkingTreeSectionKind
  count: number
  files: GitStatusFile[]
  selectedFile: string | null
  diffSource: 'working' | 'staged'
  hasMore: boolean
  loading: boolean
  isTouchPointer: boolean
  isMobileFormFactor: boolean
  multiSelectMode: boolean
  selectedPaths: string[]
}>()
const expanded = defineModel<boolean>('expanded', { required: true })
const emit = defineEmits<{
  select: [path: string]
  toggleSelect: [path: string, event: MouseEvent]
  toggleMultiSelect: []
  selectAllSelected: []
  invertSelected: []
  fileAction: [action: GitFileActionId, path: string]
  sectionAction: [action: GitSectionActionId]
  showMore: []
}>()
const { t } = useI18n()
const section = computed(() => gitWorkingTreeSections[props.kind])
const [DefineActions, RenderActions] = createReusableTemplate<{ size: 'sm' | 'md' }>()
const selected = computed(() => new Set(props.selectedPaths))
const selectablePaths = computed(() => props.files.map(({ path }) => path))
const selectedCount = computed(() => props.files.filter((file) => selected.value.has(file.path)).length)
const source = computed(() => (props.kind === 'staged' ? 'staged' : 'working'))

function actionItem<Id extends string>(
  id: Id,
  definition: GitActionDefinition,
  count = 0,
): OptionMenuItem & { id: Id } {
  const { label, icon, confirmTitle, confirmDescription, confirmText } = definition
  return {
    id,
    label: t(label),
    icon,
    variant: confirmTitle ? 'destructive' : 'default',
    confirmTitle: confirmTitle ? t(confirmTitle) : undefined,
    confirmDescription: confirmDescription ? t(confirmDescription, { count }) : undefined,
    confirmText: confirmTitle ? t(confirmText || label) : undefined,
    cancelText: confirmTitle ? t('common.cancel') : undefined,
  }
}

const fileActions = computed(() => section.value.fileActions.map((id) => actionItem(id, gitFileActions[id])))
const sectionActions = computed(() => {
  const ids = props.multiSelectMode ? section.value.selectedActions : section.value.allActions
  return ids.map((id) => ({
    ...actionItem(id, gitSectionActions[id], selectedCount.value),
    disabled: props.multiSelectMode
      ? selectedCount.value === 0
      : (id === 'discardAll' || id === 'cleanUntracked') && props.count === 0,
  }))
})

const statusClasses: Record<string, string> = {
  A: 'oc-vscode-status-added',
  C: 'oc-vscode-status-added',
  D: 'oc-vscode-status-deleted',
  M: 'oc-vscode-status-modified',
  R: 'oc-vscode-status-modified',
  T: 'oc-vscode-status-modified',
  U: 'oc-vscode-status-untracked',
  '?': 'oc-vscode-status-untracked',
}

const rows = computed(() =>
  props.files.map((file) => {
    const code =
      props.kind === 'merge'
        ? 'U'
        : props.kind === 'untracked'
          ? '??'
          : (source.value === 'staged' ? file.index : file.workingDir)?.trim() || ''
    return {
      file,
      label: code || 'M',
      style: statusClasses[code.charAt(0).toUpperCase()] || 'text-muted-foreground',
      actions: fileActions.value.map((action) => ({ ...action, description: file.path, monospace: true })),
    }
  }),
)

function selectFile(path: string, event: MouseEvent) {
  if (props.multiSelectMode) emit('toggleSelect', path, event)
  else emit('select', path)
}
</script>

<template>
  <div data-selection-context class="oc-vscode-section select-none" :data-git-working-tree-section="kind">
    <DefineActions v-slot="{ size }">
      <template v-for="action in sectionActions" :key="action.id">
        <ConfirmPopover
          v-if="action.confirmTitle"
          :title="action.confirmTitle"
          :description="action.confirmDescription"
          :confirm-text="action.confirmText"
          :cancel-text="action.cancelText"
          variant="destructive"
          @confirm="emit('sectionAction', action.id)"
        >
          <SidebarIconButton
            data-keyboard-option
            :size="size"
            destructive
            :tooltip="action.label"
            :is-touch-pointer="isTouchPointer"
            :disabled="action.disabled"
            @click.stop
          >
            <component :is="action.icon" class="size-3.5" aria-hidden="true" />
          </SidebarIconButton>
        </ConfirmPopover>
        <SidebarIconButton
          v-else
          data-keyboard-option
          :size="size"
          :tooltip="action.label"
          :is-touch-pointer="isTouchPointer"
          :disabled="action.disabled"
          @click.stop="emit('sectionAction', action.id)"
        >
          <component :is="action.icon" class="size-3.5" aria-hidden="true" />
        </SidebarIconButton>
      </template>
    </DefineActions>
    <SectionToggleButton
      :open="expanded"
      :label="t(`git.ui.workingTree.sections.${section.label}`)"
      :count="multiSelectMode ? null : count"
      @toggle="expanded = !expanded"
    >
      <template #actions>
        <SelectionToolbar
          v-if="multiSelectMode"
          compact
          :selectable-ids="selectablePaths"
          :selected-ids="selected"
          :selection-label="t('git.ui.workingTree.multiSelect.selectedCount', { count: selectedCount })"
          :count-text="`${selectedCount}/${count}`"
          :exit-label="t('git.ui.workingTree.actions.exitMultiSelect')"
          :is-touch-pointer="isTouchPointer"
          @select-all="emit('selectAllSelected')"
          @invert="emit('invertSelected')"
          @exit="emit('toggleMultiSelect')"
        >
          <template #default="{ size }">
            <RenderActions :size="size" />
          </template>
        </SelectionToolbar>
        <template v-else>
          <RenderActions size="sm" />
          <SidebarIconButton
            data-selection-trigger
            size="sm"
            :tooltip="t('git.ui.workingTree.actions.enterMultiSelect')"
            :is-touch-pointer="isTouchPointer"
            :disabled="!files.length"
            @click.stop="emit('toggleMultiSelect')"
          >
            <RiListCheck3 class="size-3.5" aria-hidden="true" />
          </SidebarIconButton>
        </template>
      </template>
    </SectionToggleButton>

    <div v-if="expanded" v-keyboard-collection class="space-y-0.5 px-1 pb-1" :aria-busy="loading || undefined">
      <p v-if="!files.length" class="oc-vscode-empty">
        {{ loading ? t('common.loading') : t(`git.ui.workingTree.empty.${section.label}`) }}
      </p>
      <GitStatusListItem
        v-for="{ file, label, style, actions } in rows"
        :key="file.path"
        :path="file.path"
        :active="!multiSelectMode && selectedFile === file.path && diffSource === source"
        :status-label="label"
        :status-class="style"
        :insertions="kind !== 'merge' ? file.insertions : undefined"
        :deletions="kind === 'changes' || kind === 'staged' ? file.deletions : undefined"
        :show-selection="multiSelectMode"
        :selected="selected.has(file.path)"
        :is-mobile-form-factor="isMobileFormFactor"
        :actions="actions"
        @select="selectFile(file.path, $event)"
        @action="emit('fileAction', $event, file.path)"
      />
      <button
        v-if="files.length && hasMore"
        type="button"
        class="ms-5 min-h-7 rounded-sm px-2 text-start text-[11px] text-muted-foreground hover:bg-sidebar-accent/45 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-60"
        :disabled="loading"
        @click="emit('showMore')"
      >
        {{ t('git.ui.workingTree.showMoreCount', { shown: files.length, total: count }) }}
      </button>
    </div>
  </div>
</template>
