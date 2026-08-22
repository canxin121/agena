<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiSparkling2Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import { useUiStore } from '@/stores/ui'

const ui = useUiStore()
const { t } = useI18n()

const emptyDescription = computed(() =>
  ui.isCompactLayout
    ? String(t('chat.messages.empty.description'))
    : String(t('chat.messages.empty.desktopDescription')),
)

function openSessionSidebar() {
  if (ui.isCompactLayout) {
    ui.setSessionSwitcherOpen(true)
    return
  }

  ui.setSidebarOpen(true, { preserveWidth: true })
}
</script>

<template>
  <section class="flex h-full min-h-0 items-center justify-center overflow-auto bg-background px-4">
    <div class="w-full max-w-sm text-center">
      <RiSparkling2Line class="mx-auto h-8 w-8 text-muted-foreground/35" aria-hidden="true" />
      <h1 class="typography-ui-label mt-3 font-semibold">{{ t('chat.messages.empty.title') }}</h1>
      <p class="typography-meta mt-1">{{ emptyDescription }}</p>
      <Button variant="outline" size="sm" class="mt-4" @click="openSessionSidebar">
        {{ t('chat.messages.empty.actionLabel') }}
      </Button>
    </div>
  </section>
</template>
