<script setup lang="ts">
import { computed, onScopeDispose, type Component } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiAddLine,
  RiChat4Line,
  RiCloseCircleLine,
  RiFolder6Line,
  RiGitMergeLine,
  RiGlobalLine,
  RiLayoutLeftLine,
  RiRestartLine,
  RiMoonLine,
  RiPaletteLine,
  RiQuestionLine,
  RiSettings3Line,
  RiSunLine,
  RiTerminalBoxLine,
  RiText,
} from '@remixicon/vue'

import Dialog from '@/components/ui/Dialog.vue'
import KeyboardShortcutEditor from '@/components/KeyboardShortcutEditor.vue'
import { keyboardOverrides } from '@/app/runtime/keyboardPreferences'
import { formatShortcut, shortcutBindings, type ShortcutActionId } from '@/app/runtime/keyboardShortcutActions'
import { reloadAgenaRuntime } from '@/lib/reload'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { resolveTranscriptVimEnabled } from '@/pages/chat/transcriptVimPreference'

function getModifierLabel(): string {
  if (typeof navigator === 'undefined') return 'Ctrl'
  return /Macintosh|Mac OS X/.test(navigator.userAgent || '') ? 'Cmd' : 'Ctrl'
}

const ui = useUiStore()
const settings = useSettingsStore()

const { t } = useI18n()

const mod = computed(() => getModifierLabel())
const shortcutLabelKeys: Record<ShortcutActionId, string> = {
  help: 'showKeyboardShortcuts',
  settings: 'openSettings',
  sidebar: 'toggleSessionSidebar',
  composer: 'focusChatInput',
  'new-session': 'createNewSession',
  theme: 'cycleTheme',
  'tab-chat': 'openChat',
  'tab-files': 'openFiles',
  'tab-preview': 'openPreview',
  'tab-terminal': 'openTerminal',
  'tab-git': 'openGitPanel',
}
const shortcutLabels = computed(
  () =>
    Object.fromEntries(
      Object.entries(shortcutLabelKeys).map(([id, key]) => [id, String(t(`help.dialog.shortcuts.${key}`))]),
    ) as Record<ShortcutActionId, string>,
)
const keysFor = (action: ShortcutActionId) =>
  shortcutBindings(action, keyboardOverrides.value).map(formatShortcut).join(' / ') ||
  String(t('keyboard.shortcutDisabled'))

// Vim shortcuts only work while the transcript Vim mode is on.
const transcriptVimEnabled = computed(() =>
  resolveTranscriptVimEnabled(settings.data?.chatTranscriptVim, ui.isMobileDevice),
)

type ShortcutItem = { keys: string; description: string; icon?: Component; vimOnly?: boolean }
type ShortcutSection = { category: string; items: ShortcutItem[]; vimOnly?: boolean }

const sections = computed((): ShortcutSection[] => {
  const m = mod.value
  const vim = transcriptVimEnabled.value
  const groups: ShortcutSection[] = [
    {
      category: String(t('help.dialog.sections.navigationCommands')),
      items: [
        {
          keys: keysFor('help'),
          description: String(t('help.dialog.shortcuts.showKeyboardShortcuts')),
          icon: RiQuestionLine,
        },
        {
          keys: keysFor('sidebar'),
          description: String(t('help.dialog.shortcuts.toggleSessionSidebar')),
          icon: RiLayoutLeftLine,
        },
        { keys: 'F6 / Shift+F6', description: String(t('help.dialog.shortcuts.keyboardRegions')) },
        { keys: 'Tab / Shift+Tab', description: String(t('help.dialog.shortcuts.focusControls')) },
        { keys: '↑ / ↓ · Home / End · Enter', description: String(t('help.dialog.shortcuts.optionNavigation')) },
        { keys: 'PageUp / PageDown · Home / End', description: String(t('help.dialog.shortcuts.scrollRegion')) },
        { keys: '← / → · Home / End · Delete', description: String(t('help.dialog.shortcuts.workspaceTabs')) },
        { keys: 'Shift+F10', description: String(t('help.dialog.shortcuts.tabContextMenu')) },
        { keys: '↑ / ↓ · ← / → · Home / End · F2', description: String(t('keyboard.treeHint')) },
        { keys: 'Alt+Shift+↑ / ↓ · Home / End', description: String(t('keyboard.transcriptHint')) },
        { keys: 'Shift+↑ / ↓ / ← / →', description: String(t('keyboard.panHint')) },
        { keys: 'F6', description: String(t('keyboard.previewExitHint')) },
        { keys: '↑ / ↓ / ← / → · Shift · Home / End', description: String(t('keyboard.resizeHint')) },
      ],
    },
    {
      category: String(t('help.dialog.sections.transcriptVim')),
      vimOnly: true,
      items: [
        { keys: 'i / Esc', description: String(t('help.dialog.shortcuts.vimModes')), icon: RiText },
        { keys: '[count] h/l · j/k', description: String(t('help.dialog.shortcuts.vimMotions')) },
        { keys: 'Ctrl+K / Ctrl+J', description: String(t('help.dialog.shortcuts.vimMessages')) },
        { keys: 'gg / [count]G / G', description: String(t('help.dialog.shortcuts.vimBounds')) },
        { keys: 'H / M / L · zt / zz / zb', description: String(t('help.dialog.shortcuts.vimViewport')) },
        { keys: 'Ctrl+U/D · Ctrl+B/F · Ctrl+E/Y', description: String(t('help.dialog.shortcuts.vimScroll')) },
        { keys: '/ / ? · n / N', description: String(t('help.dialog.shortcuts.vimSearch')) },
        { keys: 'Enter', description: String(t('help.dialog.shortcuts.vimStructure')) },
      ],
    },
    {
      category: String(t('help.dialog.sections.transcriptSelection')),
      vimOnly: true,
      items: [
        { keys: 'v / V / Ctrl+V', description: String(t('help.dialog.shortcuts.vimVisual')) },
        { keys: 'y / yy / Y', description: String(t('help.dialog.shortcuts.vimYank')) },
        {
          keys: 'iw / aw · im / am · iM / aM · ip / ap',
          description: String(t('help.dialog.shortcuts.vimTextObjects')),
        },
        { keys: 'w/b/e · W/B/E · ge/gE', description: String(t('help.dialog.shortcuts.vimWords')) },
        { keys: 'o / gv', description: String(t('help.dialog.shortcuts.vimReselect')) },
        { keys: 'f/F/t/T · ;/,', description: String(t('help.dialog.shortcuts.vimFind')) },
        { keys: 'Ctrl+O / Ctrl+I', description: String(t('help.dialog.shortcuts.vimJumps')) },
      ],
    },
    {
      category: String(t('help.dialog.sections.sessionManagement')),
      items: [
        {
          keys: keysFor('new-session'),
          description: String(t('help.dialog.shortcuts.createNewSession')),
          icon: RiAddLine,
        },
        { keys: `${m}+Enter`, description: String(t('help.dialog.shortcuts.sendMessage')) },
        {
          keys: keysFor('composer'),
          description: String(t('help.dialog.shortcuts.focusChatInput')),
          icon: RiText,
        },
        ...(vim
          ? [
              {
                keys: 'i',
                description: String(t('help.dialog.shortcuts.focusChatInput')),
                icon: RiText,
                vimOnly: true,
              },
            ]
          : []),
        { keys: `Ctrl+P`, description: String(t('help.dialog.shortcuts.openPlan')), vimOnly: true },
        {
          keys: `Ctrl+C`,
          description: String(t('help.dialog.shortcuts.abortActiveRunDouble')),
          icon: RiCloseCircleLine,
          vimOnly: true,
        },
      ],
    },
    {
      category: String(t('help.dialog.sections.interface')),
      items: [
        { keys: keysFor('theme'), description: String(t('help.dialog.shortcuts.cycleTheme')), icon: RiPaletteLine },
        { keys: keysFor('tab-chat'), description: String(t('help.dialog.shortcuts.openChat')), icon: RiChat4Line },
        {
          keys: keysFor('tab-files'),
          description: String(t('help.dialog.shortcuts.openFiles')),
          icon: RiFolder6Line,
        },
        {
          keys: keysFor('tab-preview'),
          description: String(t('help.dialog.shortcuts.openPreview')),
          icon: RiGlobalLine,
        },
        {
          keys: keysFor('tab-terminal'),
          description: String(t('help.dialog.shortcuts.openTerminal')),
          icon: RiTerminalBoxLine,
        },
        {
          keys: keysFor('tab-git'),
          description: String(t('help.dialog.shortcuts.openGitPanel')),
          icon: RiGitMergeLine,
        },
        {
          keys: keysFor('settings'),
          description: String(t('help.dialog.shortcuts.openSettings')),
          icon: RiSettings3Line,
        },
      ],
    },
  ]

  // Vim rows describe keys that do nothing while the mode is off, so the
  // dialog never advertises shortcuts the transcript would ignore.
  return groups
    .map((group) => ({ ...group, items: group.items.filter((item) => vim || !item.vimOnly) }))
    .filter((group) => vim || !group.vimOnly)
    .filter((group) => group.items.length > 0)
})

const reloadController = new AbortController()
onScopeDispose(() => reloadController.abort())

async function reloadConfiguration() {
  await reloadAgenaRuntime(reloadController.signal).catch(() => {})
  if (reloadController.signal.aborted) return
  window.location.reload()
}

async function setTheme(mode: 'light' | 'dark' | 'system') {
  if (mode === 'system') {
    await settings.save({ useSystemTheme: true }).catch(() => {})
  } else {
    await settings.save({ useSystemTheme: false, themeVariant: mode }).catch(() => {})
  }
}
</script>

<template>
  <Dialog
    :open="ui.isHelpDialogOpen"
    :title="t('help.dialog.title')"
    :description="t('help.dialog.description')"
    max-width="max-w-2xl"
    @update:open="(v) => (ui.isHelpDialogOpen = v)"
  >
    <div class="min-w-0">
      <div class="space-y-4">
        <section>
          <div class="text-[11px] font-semibold tracking-wider uppercase text-muted-foreground mb-2">
            {{ t('help.dialog.quickActions.title') }}
          </div>
          <div class="space-y-1">
            <button
              type="button"
              class="w-full flex items-center justify-between gap-3 rounded-md px-2 py-1 hover:bg-secondary/40 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              @click="reloadConfiguration"
            >
              <div class="flex items-center gap-2 min-w-0">
                <RiRestartLine class="h-4 w-4 text-muted-foreground" />
                <span class="typography-meta text-foreground/90 truncate">{{
                  t('help.dialog.quickActions.reloadConfig')
                }}</span>
              </div>
              <kbd
                class="shrink-0 inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-mono bg-muted rounded border border-border/30"
              >
                {{ t('help.dialog.quickActions.badge') }}
              </kbd>
            </button>

            <button
              type="button"
              class="w-full flex items-center justify-between gap-3 rounded-md px-2 py-1 hover:bg-secondary/40 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              @click="setTheme('light')"
            >
              <div class="flex items-center gap-2 min-w-0">
                <RiSunLine class="h-4 w-4 text-muted-foreground" />
                <span class="typography-meta text-foreground/90 truncate">{{
                  t('help.dialog.quickActions.themeLight')
                }}</span>
              </div>
              <kbd
                class="shrink-0 inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-mono bg-muted rounded border border-border/30"
              >
                {{ t('help.dialog.quickActions.badge') }}
              </kbd>
            </button>

            <button
              type="button"
              class="w-full flex items-center justify-between gap-3 rounded-md px-2 py-1 hover:bg-secondary/40 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              @click="setTheme('dark')"
            >
              <div class="flex items-center gap-2 min-w-0">
                <RiMoonLine class="h-4 w-4 text-muted-foreground" />
                <span class="typography-meta text-foreground/90 truncate">{{
                  t('help.dialog.quickActions.themeDark')
                }}</span>
              </div>
              <kbd
                class="shrink-0 inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-mono bg-muted rounded border border-border/30"
              >
                {{ t('help.dialog.quickActions.badge') }}
              </kbd>
            </button>

            <button
              type="button"
              class="w-full flex items-center justify-between gap-3 rounded-md px-2 py-1 hover:bg-secondary/40 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              @click="setTheme('system')"
            >
              <div class="flex items-center gap-2 min-w-0">
                <RiPaletteLine class="h-4 w-4 text-muted-foreground" />
                <span class="typography-meta text-foreground/90 truncate">{{
                  t('help.dialog.quickActions.themeSystem')
                }}</span>
              </div>
              <kbd
                class="shrink-0 inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-mono bg-muted rounded border border-border/30"
              >
                {{ t('help.dialog.quickActions.badge') }}
              </kbd>
            </button>
          </div>
        </section>

        <KeyboardShortcutEditor :labels="shortcutLabels" />

        <section v-for="section in sections" :key="section.category">
          <div class="text-[11px] font-semibold tracking-wider uppercase text-muted-foreground mb-2">
            {{ section.category }}
          </div>
          <div class="space-y-1">
            <div
              v-for="item in section.items"
              :key="item.keys + item.description"
              class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 rounded-md px-2 py-1 hover:bg-secondary/40"
            >
              <div class="flex items-center gap-2 min-w-0">
                <component v-if="item.icon" :is="item.icon" class="h-4 w-4 text-muted-foreground" />
                <span class="typography-meta text-foreground/90 break-words">{{ item.description }}</span>
              </div>
              <kbd
                class="inline-flex max-w-full items-center gap-1 break-words px-2 py-0.5 text-[11px] font-mono bg-muted rounded border border-border/30"
              >
                {{ item.keys }}
              </kbd>
            </div>
          </div>
        </section>
      </div>
    </div>
  </Dialog>
</template>
