import { computed, onBeforeUnmount, ref, watchEffect, type ComputedRef } from 'vue'

import type { CommandItem } from './commandPalette'

const activeLocalCommands = ref<CommandItem[]>([])
const globalPaletteOpenHandler = ref<null | (() => void)>(null)

export const registeredLocalCommands = computed(() => activeLocalCommands.value)

export function useRegisteredCommandPaletteItems(commands: ComputedRef<CommandItem[]>) {
  const stop = watchEffect(() => {
    activeLocalCommands.value = commands.value
  })

  onBeforeUnmount(() => {
    stop()
    activeLocalCommands.value = []
  })
}

export function setGlobalCommandPaletteOpenHandler(handler: (() => void) | null) {
  globalPaletteOpenHandler.value = handler
}

export function openGlobalCommandPalette() {
  globalPaletteOpenHandler.value?.()
}
