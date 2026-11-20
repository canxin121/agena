<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { RouterLink } from 'vue-router'
import { useI18n } from 'vue-i18n'
import Button from '@/components/ui/Button.vue'
import type { ChannelParticipant, ConversationKind } from '@/generated/agenaState'
import { integrationApi } from '@/lib/integrationApi'
import { channelPermissionSettings, type AccessIdentity } from '@/lib/permissionProfiles'

const props = defineProps<{
  source: number
  bindingId: string
  identity: AccessIdentity
  resource: string
  kind: ConversationKind
}>()
const { t } = useI18n()
const details = ref<HTMLDetailsElement | null>(null)
const participants = ref<ChannelParticipant[]>([])
const after = ref<string>()
const loaded = ref(false)
const loading = ref(false)
const error = ref('')
const target = computed(() => ({ ...props.identity, resource: props.resource }))
let reading: AbortController | null = null

async function load(append = false) {
  if (loading.value || (append && after.value === undefined)) return
  const controller = new AbortController()
  reading = controller
  loading.value = true
  error.value = ''
  try {
    const page = await integrationApi.participants(props.source, props.bindingId, props.resource, props.kind, {
      after: append ? after.value : undefined,
      signal: controller.signal,
    })
    if (reading !== controller) return
    participants.value = append ? [...participants.value, ...page.participants] : page.participants
    after.value = page.next_after ?? undefined
    loaded.value = true
  } catch (reason) {
    if (reading === controller) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (reading === controller) {
      reading = null
      loading.value = false
    }
  }
}

function loadWhenOpened() {
  if (details.value?.open && !loaded.value) void load()
}

watch([() => props.source, () => props.bindingId, () => props.resource, () => props.kind], () => {
  reading?.abort()
  reading = null
  loading.value = false
  loaded.value = false
  participants.value = []
  after.value = undefined
  error.value = ''
  loadWhenOpened()
})
onBeforeUnmount(() => {
  reading?.abort()
  reading = null
})
</script>

<template>
  <details ref="details" class="mt-2 min-w-0" data-testid="conversation-members" @toggle="loadWhenOpened">
    <summary class="cursor-pointer text-sm font-medium">{{ t('platforms.participants') }}</summary>
    <div class="mt-2 space-y-2" :aria-busy="loading">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <RouterLink class="text-xs underline" :to="channelPermissionSettings(target, source)">
          {{ t('platforms.roomPermission') }}
        </RouterLink>
        <Button type="button" size="sm" variant="ghost" :disabled="loading" @click="load()">
          {{ t('platforms.reload') }}
        </Button>
      </div>
      <p v-if="error" role="alert" class="break-words text-xs text-destructive">{{ error }}</p>
      <p v-else-if="loaded && !participants.length" class="text-xs text-muted-foreground">
        {{ t('platforms.noParticipants') }}
      </p>
      <ul v-if="participants.length" class="max-h-80 divide-y divide-border overflow-y-auto border-y border-border">
        <li
          v-for="participant in participants"
          :key="participant.actor"
          class="flex flex-wrap items-center justify-between gap-2 py-2"
        >
          <div class="min-w-0">
            <p v-if="participant.display_name" class="break-words text-sm">{{ participant.display_name }}</p>
            <p class="break-all font-mono text-xs">{{ participant.actor }}</p>
            <p class="mt-0.5 text-xs text-muted-foreground">
              {{ t('platforms.lastInteraction', { time: new Date(participant.last_seen_at_ms).toLocaleString() }) }}
            </p>
          </div>
          <RouterLink
            class="text-xs underline"
            :to="channelPermissionSettings({ ...target, actor: participant.actor }, source)"
          >
            {{ t('platforms.memberPermission') }}
          </RouterLink>
        </li>
      </ul>
      <Button
        v-if="after !== undefined"
        type="button"
        size="sm"
        variant="outline"
        :disabled="loading"
        @click="load(true)"
      >
        {{ t('platforms.moreParticipants') }}
      </Button>
    </div>
  </details>
</template>
