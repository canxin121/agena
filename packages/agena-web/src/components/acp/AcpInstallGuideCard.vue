<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import {
  RiAlertLine,
  RiCheckLine,
  RiFileCopyLine,
  RiRefreshLine,
  RiSettings3Line,
  RiTerminalBoxLine,
} from '@remixicon/vue'

import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { acpAgentsSettingsDestination } from '@/components/settings/settingsDestination'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { getAcpInstallGuide } from '@/lib/acpInstallGuide'
import { copyTextToClipboard } from '@/lib/clipboard'
import { useToastsStore } from '@/stores/toasts'
import { useUiStore } from '@/stores/ui'
import { formatExternalAgentLabel } from '@/types/chat'

const props = withDefaults(
  defineProps<{
    agentId: string
    agentName?: string
    command?: string
    rawError?: string
    compact?: boolean
    showConfigure?: boolean
    showRetry?: boolean
    showUseNpx?: boolean
    retrying?: boolean
  }>(),
  {
    compact: false,
    showConfigure: true,
    showRetry: false,
    showUseNpx: false,
    retrying: false,
  },
)

const emit = defineEmits<{
  (event: 'configure', agentId: string): void
  (event: 'retry'): void
  (event: 'use-npx', preset: { command: string; args: readonly string[] }): void
}>()

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const ui = useUiStore()
const toasts = useToastsStore()
const navigation = useWorkspaceNavigation()

const copiedCommand = ref('')
let copyTimer: number | undefined

const guide = computed(() => getAcpInstallGuide(props.agentId, props.command))
const displayName = computed(() => props.agentName?.trim() || formatExternalAgentLabel(guide.value.agentId))

const summaryText = computed(() => {
  if (guide.value.kind === 'claude-code') {
    return t('chat.nativeAgent.installGuide.claudeSummary')
  }
  if (guide.value.kind === 'antigravity') {
    return t('chat.nativeAgent.installGuide.antigravitySummary', { command: guide.value.expectedCommand })
  }
  return t('chat.nativeAgent.installGuide.customSummary', { command: guide.value.expectedCommand })
})

async function copyCommand(command: string) {
  const ok = await copyTextToClipboard(command)
  if (!ok) return
  copiedCommand.value = command
  toasts.push('success', String(t('common.copied')))
  window.clearTimeout(copyTimer)
  copyTimer = window.setTimeout(() => {
    copiedCommand.value = ''
  }, 1800)
}

function openConfigure() {
  emit('configure', guide.value.agentId)
  const destination = acpAgentsSettingsDestination(route.query, guide.value.agentId)
  if (ui.isCompactLayout) {
    ui.setSessionSwitcherOpen(false)
    void router.push(destination)
  } else {
    void navigation.openMainTab('settings', destination)
  }
}

function applyNpx() {
  if (guide.value.npxPreset) {
    emit('use-npx', guide.value.npxPreset)
  }
}

onBeforeUnmount(() => {
  window.clearTimeout(copyTimer)
})
</script>

<template>
  <section
    role="status"
    class="rounded-lg border border-amber-500/35 bg-amber-500/8 text-foreground"
    :class="compact ? 'space-y-2 p-2.5 text-xs' : 'space-y-3 p-3.5 text-xs'"
  >
    <div class="flex items-start gap-2">
      <RiAlertLine class="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />
      <div class="min-w-0 flex-1 space-y-1">
        <div class="flex flex-wrap items-center gap-1.5">
          <p class="font-semibold text-foreground">
            {{ t('chat.nativeAgent.installGuide.title', { agent: displayName }) }}
          </p>
          <code class="rounded border border-amber-500/30 bg-background/70 px-1.5 py-0.5 font-mono text-[11px] text-amber-700 dark:text-amber-300">
            {{ guide.expectedCommand }}
          </code>
        </div>
        <p class="leading-relaxed text-muted-foreground">
          {{ summaryText }}
        </p>
      </div>
    </div>

    <div v-if="guide.commands.length" class="space-y-1.5">
      <div v-for="step in guide.commands" :key="step.id" class="space-y-0.5">
        <p class="text-[11px] font-medium text-muted-foreground">{{ t(step.labelKey) }}</p>
        <div class="flex items-center gap-1 rounded-md border border-border/70 bg-background/85 px-2 py-1">
          <code class="min-w-0 flex-1 select-all break-all font-mono text-[11px] text-foreground">
            {{ step.command }}
          </code>
          <IconButton
            size="sm"
            variant="ghost"
            class="h-6 w-6 shrink-0"
            :tooltip="t('common.copy')"
            :aria-label="t('common.copy')"
            @click="copyCommand(step.command)"
          >
            <RiCheckLine v-if="copiedCommand === step.command" class="h-3.5 w-3.5 text-emerald-500" />
            <RiFileCopyLine v-else class="h-3.5 w-3.5" />
          </IconButton>
        </div>
      </div>
    </div>

    <p v-if="guide.npxPreset && !showUseNpx" class="text-[11px] leading-relaxed text-muted-foreground">
      {{ t('chat.nativeAgent.installGuide.claudeNpxTip', { pkg: guide.npxPreset.args[1] ?? '@agentclientprotocol/claude-agent-acp' }) }}
    </p>

    <p class="text-[11px] leading-relaxed text-muted-foreground">
      {{ t('chat.nativeAgent.installGuide.serverHostHint') }}
    </p>

    <div v-if="showConfigure || showRetry || (showUseNpx && guide.npxPreset)" class="flex flex-wrap items-center gap-1.5">
      <Button
        v-if="showUseNpx && guide.npxPreset"
        size="sm"
        variant="outline"
        class="h-7 px-2.5 text-xs"
        @click="applyNpx"
      >
        <RiTerminalBoxLine class="mr-1.5 h-3.5 w-3.5" />
        {{ t('chat.nativeAgent.installGuide.useNpxAction') }}
      </Button>
      <Button
        v-if="showConfigure"
        size="sm"
        variant="outline"
        class="h-7 px-2.5 text-xs"
        @click="openConfigure"
      >
        <RiSettings3Line class="mr-1.5 h-3.5 w-3.5" />
        {{ t('chat.nativeAgent.installGuide.configureAction') }}
      </Button>
      <Button
        v-if="showRetry"
        size="sm"
        variant="ghost"
        class="h-7 px-2.5 text-xs"
        :disabled="retrying"
        @click="emit('retry')"
      >
        <RiRefreshLine class="mr-1.5 h-3.5 w-3.5" :class="retrying ? 'animate-spin' : ''" />
        {{ t('chat.nativeAgent.installGuide.recheckAction') }}
      </Button>
    </div>

    <details v-if="rawError" class="rounded border border-border/60 bg-background/60 px-2 py-1.5 text-[11px]">
      <summary class="cursor-pointer select-none font-medium text-muted-foreground">
        {{ t('chat.nativeAgent.installGuide.errorDetails') }}
      </summary>
      <p class="mt-1.5 whitespace-pre-wrap break-words font-mono text-[11px] text-destructive">
        {{ rawError }}
      </p>
    </details>
  </section>
</template>
