import {
  RiAddLine,
  RiArrowGoBackLine,
  RiDeleteBinLine,
  RiEyeOffLine,
  RiHistoryLine,
  RiPencilLine,
  RiSubtractLine,
} from '@remixicon/vue'
import type { Component } from 'vue'

export type GitActionDefinition = {
  label: string
  icon: Component
  confirmTitle?: string
  confirmDescription?: string
  confirmText?: string
}

const tree = 'git.ui.workingTree'
const actions = `${tree}.actions`
const cleanup = 'git.actionsMenu.cleanup'

export const gitFileActions = {
  stage: { label: `${actions}.stage`, icon: RiAddLine },
  unstage: { label: `${actions}.unstage`, icon: RiSubtractLine },
  history: { label: `${actions}.history`, icon: RiHistoryLine },
  rename: { label: 'common.rename', icon: RiPencilLine },
  ignore: { label: `${actions}.ignore`, icon: RiEyeOffLine },
  discard: {
    label: `${actions}.discardChanges`,
    icon: RiArrowGoBackLine,
    confirmTitle: `${tree}.confirmDiscard.title`,
    confirmDescription: `${tree}.confirmDiscard.description`,
    confirmText: 'common.discard',
  },
  delete: {
    label: 'common.delete',
    icon: RiDeleteBinLine,
    confirmTitle: `${tree}.confirmDeleteTracked.title`,
    confirmDescription: `${tree}.confirmDeleteTracked.description`,
    confirmText: `${actions}.deleteFile`,
  },
  deleteUntracked: {
    label: `${actions}.deleteFile`,
    icon: RiDeleteBinLine,
    confirmTitle: `${tree}.confirmDeleteUntracked.title`,
    confirmDescription: `${tree}.confirmDeleteUntracked.description`,
    confirmText: `${actions}.deleteFile`,
  },
} satisfies Record<string, GitActionDefinition>

export const gitSectionActions = {
  stageAll: { label: `${actions}.stageAll`, icon: RiAddLine },
  unstageAll: { label: `${actions}.unstageAll`, icon: RiSubtractLine },
  stageSelected: { label: `${actions}.stageSelected`, icon: RiAddLine },
  unstageSelected: { label: `${actions}.unstageSelected`, icon: RiSubtractLine },
  ignoreSelected: { label: `${actions}.ignore`, icon: RiEyeOffLine },
  discardSelected: {
    label: `${actions}.discardSelected`,
    icon: RiArrowGoBackLine,
    confirmTitle: `${tree}.confirmDiscardSelected.title`,
    confirmDescription: `${tree}.confirmDiscardSelected.description`,
  },
  deleteSelected: {
    label: `${actions}.deleteSelected`,
    icon: RiDeleteBinLine,
    confirmTitle: `${tree}.confirmDeleteSelected.title`,
    confirmDescription: `${tree}.confirmDeleteSelected.description`,
  },
  discardAll: {
    label: `${cleanup}.discardAllTracked.label`,
    icon: RiArrowGoBackLine,
    confirmTitle: `${cleanup}.discardAllTracked.confirmTitle`,
    confirmDescription: `${cleanup}.discardAllTracked.confirmDescription`,
    confirmText: 'common.discard',
  },
  cleanUntracked: {
    label: `${cleanup}.cleanUntracked.label`,
    icon: RiDeleteBinLine,
    confirmTitle: `${cleanup}.cleanUntracked.confirmTitle`,
    confirmDescription: `${cleanup}.cleanUntracked.confirmDescription`,
    confirmText: 'common.clean',
  },
} satisfies Record<string, GitActionDefinition>

export type GitFileActionId = keyof typeof gitFileActions
export type GitSectionActionId = keyof typeof gitSectionActions

type SectionDefinition = {
  label: string
  fileActions: GitFileActionId[]
  allActions: GitSectionActionId[]
  selectedActions: GitSectionActionId[]
}

export const gitWorkingTreeSections = {
  merge: {
    label: 'mergeChanges',
    fileActions: ['history', 'discard'],
    allActions: [],
    selectedActions: ['stageSelected', 'discardSelected'],
  },
  staged: {
    label: 'stagedChanges',
    fileActions: ['unstage', 'history', 'rename', 'delete'],
    allActions: ['unstageAll'],
    selectedActions: ['unstageSelected', 'discardSelected', 'deleteSelected'],
  },
  changes: {
    label: 'changes',
    fileActions: ['stage', 'history', 'rename', 'discard', 'delete'],
    allActions: ['discardAll', 'stageAll'],
    selectedActions: ['stageSelected', 'discardSelected', 'deleteSelected'],
  },
  untracked: {
    label: 'untracked',
    fileActions: ['stage', 'rename', 'ignore', 'deleteUntracked'],
    allActions: ['cleanUntracked', 'stageAll'],
    selectedActions: ['stageSelected', 'ignoreSelected', 'deleteSelected'],
  },
} satisfies Record<string, SectionDefinition>

export type GitWorkingTreeSectionKind = keyof typeof gitWorkingTreeSections
