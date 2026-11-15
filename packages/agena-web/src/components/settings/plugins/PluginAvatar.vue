<script setup lang="ts">
import { computed, type Component } from 'vue'
import {
  RiBook2Line,
  RiBrainLine,
  RiCalendarScheduleLine,
  RiChat3Line,
  RiCodeSSlashLine,
  RiDashboard3Line,
  RiFileList2Line,
  RiFolder3Line,
  RiGitBranchLine,
  RiGlobalLine,
  RiNotification3Line,
  RiPlugLine,
  RiRobot2Line,
  RiSettings3Line,
  RiTelegramLine,
  RiTerminalBoxLine,
} from '@remixicon/vue'

const props = defineProps<{ pluginId: string }>()

const icons: Record<string, Component> = {
  'agena.code': RiCodeSSlashLine,
  'agena.cron': RiCalendarScheduleLine,
  'agena.fs': RiFolder3Line,
  'agena.interaction': RiChat3Line,
  'agena.lsp': RiCodeSSlashLine,
  'agena.mcp': RiPlugLine,
  'agena.memory': RiBrainLine,
  'agena.monitor': RiDashboard3Line,
  'agena.notebook': RiBook2Line,
  'agena.notifications': RiNotification3Line,
  'agena.plan': RiGitBranchLine,
  'agena.report': RiFileList2Line,
  'agena.session': RiChat3Line,
  'agena.settings': RiSettings3Line,
  'agena.shell': RiTerminalBoxLine,
  'agena.telegram': RiTelegramLine,
  'agena.web': RiGlobalLine,
  'agena.claude': RiRobot2Line,
  'agena.chatgpt': RiRobot2Line,
  'agena.gemini': RiRobot2Line,
}
const icon = computed(() => (Object.hasOwn(icons, props.pluginId) ? icons[props.pluginId]! : RiPlugLine))
</script>

<template>
  <span
    class="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-primary/10 bg-primary/5 text-primary"
    aria-hidden="true"
  >
    <component :is="icon" class="h-5 w-5" />
  </span>
</template>
