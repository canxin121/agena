<script setup lang="ts">
import { RiUserSettingsLine } from '@remixicon/vue'
import { RouterLink } from 'vue-router'

defineProps<{ profile?: string | null }>()
</script>

<template>
  <RouterLink
    :to="{ path: '/settings/runtime-session', query: { view: 'agent-profiles', scope: 'session' } }"
    class="pointer-events-auto flex min-h-7 max-w-36 shrink-0 touch-manipulation items-center gap-1 rounded px-1 py-0.5 text-xs text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
    :title="$st('Choose the agent profile for the current session.')"
    :aria-label="$st('Current session profile')"
  >
    <RiUserSettingsLine class="h-3 w-3 shrink-0" />
    <span class="truncate">{{ profile || $st('Agent profiles') }}</span>
  </RouterLink>
</template>
