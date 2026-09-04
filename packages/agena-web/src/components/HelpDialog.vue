<script setup lang="ts">
import { computed, type Component } from 'vue'
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
import { reloadAgenaRuntime } from '@/lib/reload'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'

function getModifierLabel(): string {
  if (typeof navigator === 'undefined') return 'Ctrl'
  return /Macintosh|Mac OS X/.test(navigator.userAgent || '') ? 'Cmd' : 'Ctrl'
}

const ui = useUiStore()
const settings = useSettingsStore()

const { t } = useI18n()

const mod = computed(() => getModifierLabel())

type ShortcutItem = { keys: string; description: string; icon?: Component }

const sections = computed((): Array<{ category: string; items: ShortcutItem[] }> => {
  const m = mod.value
  return [
    {
      category: String(t('help.dialog.sections.navigationCommands')),
      items: [
        {
          keys: `Ctrl+H / ${m}+.`,
          description: String(t('help.dialog.shortcuts.showKeyboardShortcuts')),
          icon: RiQuestionLine,
        },
        {
          keys: `${m}+L`,
          description: String(t('help.dialog.shortcuts.toggleSessionSidebar')),
          icon: RiLayoutLeftLine,
        },
      ],
    },
    {
      category: String(t('help.dialog.sections.transcriptVim')),
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
          keys: `${m}+N`,
          description: String(t('help.dialog.shortcuts.createNewSession')),
          icon: RiAddLine,
        },
        { keys: `${m}+Enter`, description: String(t('help.dialog.shortcuts.sendMessage')) },
        { keys: `i`, description: String(t('help.dialog.shortcuts.focusChatInput')), icon: RiText },
        { keys: `Ctrl+P`, description: String(t('help.dialog.shortcuts.openPlan')) },
        {
          keys: `Ctrl+C`,
          description: String(t('help.dialog.shortcuts.abortActiveRunDouble')),
          icon: RiCloseCircleLine,
        },
      ],
    },
    {
      category: String(t('help.dialog.sections.interface')),
      items: [
        { keys: `${m}+/`, description: String(t('help.dialog.shortcuts.cycleTheme')), icon: RiPaletteLine },
        { keys: `${m}+1`, description: String(t('help.dialog.shortcuts.openChat')), icon: RiChat4Line },
        { keys: `${m}+2`, description: String(t('help.dialog.shortcuts.openFiles')), icon: RiFolder6Line },
        { keys: `${m}+3`, description: String(t('help.dialog.shortcuts.openPreview')), icon: RiGlobalLine },
        { keys: `${m}+4`, description: String(t('help.dialog.shortcuts.openTerminal')), icon: RiTerminalBoxLine },
        { keys: `${m}+5`, description: String(t('help.dialog.shortcuts.openGitPanel')), icon: RiGitMergeLine },
        { keys: `${m}+,`, description: String(t('help.dialog.shortcuts.openSettings')), icon: RiSettings3Line },
      ],
    },
  ]
})

async function reloadConfiguration() {
  await reloadAgenaRuntime().catch(() => {})
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
    <div class="max-h-[65dvh] overflow-auto pr-1">
      <div class="space-y-4">
        <section>
          <div class="text-[11px] font-semibold tracking-wider uppercase text-muted-foreground mb-2">
            {{ t('help.dialog.quickActions.title') }}
          </div>
          <div class="space-y-1">
            <button
              type="button"
              class="w-full flex items-center justify-between gap-3 rounded-md px-2 py-1 hover:bg-secondary/40 text-left"
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
              class="w-full flex items-center justify-between gap-3 rounded-md px-2 py-1 hover:bg-secondary/40 text-left"
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
              class="w-full flex items-center justify-between gap-3 rounded-md px-2 py-1 hover:bg-secondary/40 text-left"
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
              class="w-full flex items-center justify-between gap-3 rounded-md px-2 py-1 hover:bg-secondary/40 text-left"
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

        <section v-for="section in sections" :key="section.category">
          <div class="text-[11px] font-semibold tracking-wider uppercase text-muted-foreground mb-2">
            {{ section.category }}
          </div>
          <div class="space-y-1">
            <div
              v-for="item in section.items"
              :key="item.keys + item.description"
              class="flex items-center justify-between gap-3 rounded-md px-2 py-1 hover:bg-secondary/40"
            >
              <div class="flex items-center gap-2 min-w-0">
                <component v-if="item.icon" :is="item.icon" class="h-4 w-4 text-muted-foreground" />
                <span class="typography-meta text-foreground/90 truncate">{{ item.description }}</span>
              </div>
              <kbd
                class="shrink-0 inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-mono bg-muted rounded border border-border/30"
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
