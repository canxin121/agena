<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import InlineNameEditor from '@/components/ui/InlineNameEditor.vue'
import ListItemOverflowActionButton from '@/components/ui/ListItemOverflowActionButton.vue'
import ListItemSelectionIndicator from '@/components/ui/ListItemSelectionIndicator.vue'
import SidebarListItem from '@/components/ui/SidebarListItem.vue'

const props = defineProps<{
  sessionId: string
  subtitle: string
  statusLabel: string
  statusClass: string
  active: boolean
  multiSelect: boolean
  selected: boolean
  renaming: boolean
  renameBusy: boolean
  renameError: string
  canSaveRename: boolean
  compactTouch: boolean
}>()
const emit = defineEmits<{
  click: [event: MouseEvent]
  menu: [event: MouseEvent]
  rename: []
  save: []
  cancel: []
}>()
const draft = defineModel<string>('draft', { required: true })
const { t } = useI18n()
function requestRename(event: Event) {
  if (props.multiSelect || props.renaming) return
  event.preventDefault()
  emit('rename')
}
</script>

<template>
  <SidebarListItem
    :active="active"
    :as="renaming ? 'div' : 'button'"
    density="compact"
    :actions-always-visible="!multiSelect && (compactTouch || renaming)"
    :data-keyboard-item="sessionId"
    :data-keyboard-label="sessionId"
    :data-keyboard-selected="String(multiSelect ? selected : active)"
    :data-keyboard-multiselect="String(multiSelect)"
    :primary-attrs="{
      'aria-pressed': multiSelect ? selected : undefined,
      'aria-current': active && !renaming ? 'true' : undefined,
    }"
    @click="emit('click', $event)"
    @keyboard-rename="requestRename"
  >
    <template #icon>
      <div class="flex items-center gap-1.5">
        <ListItemSelectionIndicator v-if="multiSelect" :selected="selected" />
        <span
          class="size-1.5 shrink-0 rounded-full"
          :class="statusClass"
          :title="statusLabel"
          :aria-label="statusLabel"
        />
      </div>
    </template>

    <InlineNameEditor
      v-if="renaming"
      v-model="draft"
      monospace
      :label="t('common.rename')"
      :placeholder="t('workspaceDock.preview.emptyState.sessionIdPlaceholder')"
      :busy="renameBusy"
      :invalid="!canSaveRename"
      :error="renameError"
      @submit="emit('save')"
      @cancel="emit('cancel')"
    />
    <div v-else class="flex min-w-0 flex-1 flex-col justify-center text-start">
      <span class="typography-ui-label truncate" :title="sessionId">{{ sessionId }}</span>
      <span class="typography-micro truncate font-mono text-muted-foreground/70" :title="subtitle">{{ subtitle }}</span>
    </div>

    <template #actions>
      <ListItemOverflowActionButton
        v-if="!renaming && !multiSelect"
        data-keyboard-menu
        :mobile="compactTouch"
        :label="t('common.actions')"
        @trigger="emit('menu', $event)"
      />
    </template>
  </SidebarListItem>
</template>
