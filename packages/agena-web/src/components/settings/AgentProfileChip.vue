<script setup lang="ts">
import { RiArrowDownSLine, RiUserSettingsLine } from '@remixicon/vue'

defineProps<{ profile?: string | null; profileIsDefault?: boolean; open?: boolean }>()
defineEmits<{ (event: 'select'): void }>()
</script>

<template>
  <button
    type="button"
    data-testid="composer-profile-trigger"
    data-oc-keyboard-tap="blur"
    class="pointer-events-auto flex min-h-7 max-w-36 shrink-0 touch-manipulation items-center gap-1 rounded px-1 py-0.5 text-xs text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
    :class="open ? 'bg-secondary/60 text-foreground' : ''"
    :title="$st('Choose the agent profile for the current session.')"
    :aria-label="$st('Current session profile')"
    aria-haspopup="dialog"
    :aria-expanded="Boolean(open)"
    @mousedown.prevent
    @click.stop="$emit('select')"
  >
    <RiUserSettingsLine class="h-3 w-3 shrink-0" />
    <span class="truncate">{{ profile || $st('Agent profiles') }}</span>
    <RiArrowDownSLine class="h-3 w-3 shrink-0" />
  </button>
</template>
