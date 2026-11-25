<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { RiAddLine, RiEditLine, RiArchiveLine, RiCheckLine } from '@remixicon/vue'
import Dialog from '@/components/ui/Dialog.vue'
import Input from '@/components/ui/Input.vue'
import Button from '@/components/ui/Button.vue'
import AgentProfilesPanel from '@/components/settings/AgentProfilesPanel.vue'
import BotAgentProfilePicker from '@/features/bots/BotAgentProfilePicker.vue'
import BotPromptEditor from '@/features/bots/BotPromptEditor.vue'
import { archiveBot, createBot, emptyBotPersona, getBot, updateBot, type BotPersona, type BotProfile } from '@/lib/bots'
import { botDraftFields, instructionsBytes, MAX_INSTRUCTIONS_BYTES } from '@/features/bots/draft'
import BotAvatar from '@/features/bots/BotAvatar.vue'
import { BOT_AVATAR_ACCEPT, MAX_BOT_AVATAR_DIMENSION, MAX_BOT_AVATAR_PIXELS } from '@/features/bots/avatar'
import { useBotAvatarDraft } from '@/features/bots/avatarDraft'
import { SaveOwnership } from '@/features/bots/saveOwnership'
import { useBotsStore } from '@/stores/bots'
import { useChatStoreUnscoped } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import { useUiStore } from '@/stores/ui'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { handleOptionKeydown } from '@/lib/keyboardNavigation'
const bots = useBotsStore()
const chat = useChatStoreUnscoped()
const ui = useUiStore()
const toasts = useToastsStore()
const navigation = useWorkspaceNavigation()
const route = useRoute()
const { t } = useI18n()
const uid = useId()
const selectedId = computed(() => String(route.query.botId || ''))
const profiles = computed(() => bots.profiles)
const loading = computed(() => bots.loading)
const listError = computed(() => bots.error)
const returnFocus = ref<HTMLElement | null>(null)
const managerOpen = ref(false)
const editorOpen = ref(false)
const archiveTarget = ref<BotProfile | null>(null)
const busy = ref(false)
const error = ref('')
const editing = ref<BotProfile | null>(null)
const draftName = ref('')
const persona = ref<BotPersona>(emptyBotPersona())
const agentProfile = ref('')
const creatingAgentProfile = ref(false)
const avatarInput = ref<HTMLInputElement | null>(null)
const avatarDraft = useBotAvatarDraft()
const { avatar: draftAvatar, change: avatarChange, loading: avatarLoading, error: avatarError } = avatarDraft
const byteCount = computed(() => instructionsBytes(persona.value.instructions))
let saveOwnership: SaveOwnership | null = null
let sequence = 0
let navigationVersion = 0
let profileRequest: AbortController | null = null
watch(
  [() => route.fullPath, () => ui.activeWorkspaceWindowId],
  () => {
    navigationVersion++
    cancelProfileRead()
  },
  { flush: 'sync' },
)
let receipt: { signature: string; key: string } | null = null
function requestKey(operation: string, body: object) {
  const signature = JSON.stringify([operation, body])
  if (receipt?.signature !== signature) receipt = { signature, key: crypto.randomUUID() }
  return receipt.key
}
function cancelProfileRead() {
  if (!profileRequest) return
  sequence++
  profileRequest.abort()
  profileRequest = null
  busy.value = false
}
function setManagerOpen(value: boolean) {
  managerOpen.value = value
  if (!value) cancelProfileRead()
}
function cancelEditor() {
  cancelProfileRead()
  if (busy.value) saveOwnership?.cancel()
  avatarDraft.cancel()
  editorOpen.value = false
}
function setEditorOpen(value: boolean) {
  if (value) editorOpen.value = true
  else cancelEditor()
}
function uploadAvatar(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.item(0)
  input.value = ''
  if (file && editorOpen.value && !busy.value) void avatarDraft.upload(file)
}
function refresh() {
  return bots.refresh(true)
}
function choose(id: string) {
  managerOpen.value = false
  const profile = profiles.value.find((p) => p.id === id)
  void navigation.openWorkspaceLocation('bot', { query: { botId: id }, title: profile?.name, matchKeys: ['botId'] })
}
function create() {
  if (busy.value) return
  sequence++
  editing.value = null
  draftName.value = ''
  persona.value = emptyBotPersona()
  agentProfile.value = ''
  creatingAgentProfile.value = false
  avatarDraft.reset()
  error.value = ''
  receipt = null
  managerOpen.value = false
  editorOpen.value = true
}
async function edit(id?: string | null, rename = false) {
  if (!id || busy.value) return
  const current = ++sequence
  const abort = new AbortController()
  profileRequest = abort
  avatarDraft.cancel()
  error.value = ''
  busy.value = true
  try {
    const profile = await getBot(id, abort.signal)
    if (current !== sequence || abort.signal.aborted) return
    if (profile.archived) throw new Error(t('bot.profiles.archived'))
    editing.value = profile
    draftName.value = profile.name
    persona.value = { ...profile.persona }
    agentProfile.value = profile.agent_profile ?? ''
    creatingAgentProfile.value = false
    avatarDraft.reset(profile.avatar)
    receipt = null
    managerOpen.value = false
    editorOpen.value = true
    if (rename) void nextTick(() => document.querySelector<HTMLInputElement>('[data-testid="bot-edit-name"]')?.focus())
  } catch (cause) {
    if (current === sequence) {
      error.value = cause instanceof Error ? cause.message : String(cause)
      managerOpen.value = true
    }
  } finally {
    if (profileRequest === abort) profileRequest = null
    if (current === sequence) busy.value = false
  }
}
async function save() {
  if (busy.value || avatarLoading.value || !draftName.value.trim() || byteCount.value > MAX_INSTRUCTIONS_BYTES) return
  busy.value = true
  error.value = ''
  saveOwnership = new SaveOwnership(navigationVersion, route.fullPath, ui.activeWorkspaceWindowId)
  const current = editing.value
  const generation = sequence
  const base = botDraftFields(draftName.value, persona.value.instructions, avatarChange.value)
  const fields = {
    ...base,
    ...((current?.agent_profile ?? '') !== agentProfile.value ? { agent_profile: agentProfile.value } : {}),
    persona: { ...base.persona, ...(persona.value.prompts ? { prompts: persona.value.prompts } : {}) },
  }
  try {
    const profile = current
      ? await updateBot(current.id, {
          ...fields,
          expected_revision: current.revision,
          request_key: requestKey(`update:${current.id}`, { ...fields, expected_revision: current.revision }),
        })
      : await createBot({ ...fields, request_key: requestKey('create', fields) })
    if (generation !== sequence) return
    receipt = null
    editorOpen.value = false
    bots.apply(profile)
    if (current && current.agent_profile !== profile.agent_profile) {
      const sessions = new Set([
        profile.local_session_id,
        selectedId.value === profile.id ? route.query.sessionId : null,
      ])
      for (const id of sessions) {
        if (typeof id === 'string' || typeof id === 'number')
          void chat.refreshExecutionStatus(String(id), true).catch(() => {})
      }
    }
    if (!current && saveOwnership?.canNavigate(navigationVersion, route.fullPath, ui.activeWorkspaceWindowId))
      choose(profile.id)
    else if (!current) toasts.push('success', t('bot.profiles.created', { name: profile.name }))
    else if (saveOwnership?.cancelled) toasts.push('success', profile.name)
  } catch (cause) {
    if (generation === sequence) {
      const message = cause instanceof Error ? cause.message : String(cause)
      if (saveOwnership?.cancelled) toasts.push('error', message)
      else error.value = message
    }
  } finally {
    if (generation === sequence) busy.value = false
  }
}

function confirmArchive(profile: BotProfile) {
  error.value = ''
  receipt = null
  managerOpen.value = false
  archiveTarget.value = profile
}
async function archive() {
  const profile = archiveTarget.value
  if (!profile || busy.value) return
  busy.value = true
  error.value = ''
  const generation = sequence
  try {
    const fields = { expected_revision: profile.revision }
    const result = await archiveBot(profile.id, { ...fields, request_key: requestKey(`archive:${profile.id}`, fields) })
    if (generation !== sequence) return
    archiveTarget.value = null
    receipt = null
    bots.apply(result)
  } catch (cause) {
    if (generation === sequence) error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    if (generation === sequence) busy.value = false
  }
}
watch(
  () => bots.request,
  (request) => {
    if (!request || busy.value) return
    returnFocus.value = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const fromDrawer = ui.isSessionSwitcherOpen
    ui.setSessionSwitcherOpen(false)
    if (fromDrawer) returnFocus.value = document.querySelector<HTMLElement>('main[data-keyboard-region]')
    if (request.kind === 'manager') {
      managerOpen.value = true
      void bots.refresh()
    } else if (request.kind === 'create') create()
    else if (request.kind === 'edit' || request.kind === 'rename') void edit(request.id, request.kind === 'rename')
    else {
      const profile = profiles.value.find((p) => p.id === request.id)
      if (profile) confirmArchive(profile)
    }
  },
)
watch(
  managerOpen,
  (value) => {
    bots.managerOpen = value
  },
  { flush: 'sync' },
)
watch(
  () => bots.version,
  () => {
    cancelProfileRead()
    sequence++
    saveOwnership = null
    receipt = null
    managerOpen.value = false
    editorOpen.value = false
    archiveTarget.value = null
    busy.value = false
    error.value = ''
    editing.value = null
    draftName.value = ''
    persona.value = emptyBotPersona()
    avatarDraft.reset()
  },
  { flush: 'sync' },
)
let stop = () => {}
onMounted(() => {
  stop = bots.start()
})
onBeforeUnmount(() => {
  cancelProfileRead()
  sequence++
  avatarDraft.cancel()
  stop()
})
</script>
<template>
  <Dialog
    :return-focus="returnFocus"
    :open="managerOpen"
    :title="t('bot.profiles.choose')"
    :description="t('bot.profiles.scope')"
    initial-focus="[data-bot-choices]"
    @update:open="setManagerOpen"
  >
    <div
      data-testid="bot-manager"
      data-bot-choices
      tabindex="-1"
      class="space-y-3 focus-visible:outline-2 focus-visible:outline-ring"
      @keydown="handleOptionKeydown($event, $event.currentTarget as HTMLElement)"
    >
      <p v-if="loading" role="status" class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>
      <div v-if="listError || error" role="alert" class="space-y-2 break-words text-sm text-destructive">
        <p>{{ listError || error }}</p>
        <Button variant="outline" @click="refresh">{{ t('common.retry') }}</Button>
      </div>
      <p v-if="!loading && !profiles.length && !listError" class="py-4 text-sm text-muted-foreground">
        {{ t('bot.profiles.empty') }}
      </p>
      <div
        v-for="profile in profiles"
        :key="profile.id"
        class="flex min-w-0 items-center gap-1 rounded-xl border p-1.5"
        :class="profile.id === selectedId ? 'border-primary/40 bg-primary/5' : 'border-border/70'"
        :data-bot-profile="profile.id"
      >
        <button
          type="button"
          data-keyboard-option
          class="flex min-h-11 min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-start hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
          :data-bot-select="profile.id"
          @click="choose(profile.id)"
        >
          <BotAvatar :name="profile.name" :avatar="profile.avatar" />
          <span class="min-w-0 flex-1 truncate text-sm font-medium">{{ profile.name }}</span
          ><RiCheckLine v-if="profile.id === selectedId" class="h-4 w-4 shrink-0 text-primary" />
        </button>
        <button
          type="button"
          class="flex h-11 w-10 shrink-0 items-center justify-center rounded-lg hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
          :disabled="busy"
          :aria-label="t('bot.profiles.editNamed', { name: profile.name })"
          :data-bot-edit="profile.id"
          @click="edit(profile.id)"
        >
          <RiEditLine class="h-4 w-4" />
        </button>
        <button
          type="button"
          class="flex h-11 w-10 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-destructive/10 hover:text-destructive focus-visible:ring-2 focus-visible:ring-ring"
          :disabled="busy"
          :aria-label="t('bot.profiles.archiveNamed', { name: profile.name })"
          :data-bot-archive="profile.id"
          @click="confirmArchive(profile)"
        >
          <RiArchiveLine class="h-4 w-4" />
        </button>
      </div>
    </div>
    <template #footer
      ><Button class="w-full" data-testid="bot-create" :disabled="busy" @click="create"
        ><RiAddLine class="mr-2 h-4 w-4" />{{ t('bot.profiles.create') }}</Button
      ></template
    >
  </Dialog>

  <Dialog
    :return-focus="returnFocus"
    :open="editorOpen"
    :title="
      creatingAgentProfile
        ? $st('New profile')
        : editing
          ? t('bot.profiles.editNamed', { name: editing.name })
          : t('bot.profiles.create')
    "
    :description="creatingAgentProfile ? undefined : t('bot.profiles.scope')"
    max-width="max-w-2xl"
    @update:open="setEditorOpen"
  >
    <AgentProfilesPanel
      v-if="creatingAgentProfile"
      scope="global"
      create-for-bot
      @cancel-create="creatingAgentProfile = false"
      @profile-created="(name) => ((agentProfile = name), (creatingAgentProfile = false))"
    />
    <form v-else :id="`${uid}-form`" class="space-y-4" data-testid="bot-persona-form" @submit.prevent="save">
      <div class="space-y-2">
        <label :for="`${uid}-avatar`" class="text-sm font-medium">{{ t('bot.profiles.avatar') }}</label>
        <div class="flex min-w-0 items-center gap-3">
          <BotAvatar :name="draftName" :avatar="draftAvatar" size="xl" data-testid="bot-avatar-preview" />
          <div class="min-w-0 flex-1 space-y-2">
            <input
              :id="`${uid}-avatar`"
              ref="avatarInput"
              type="file"
              class="sr-only"
              tabindex="-1"
              :accept="BOT_AVATAR_ACCEPT"
              :disabled="busy"
              :aria-describedby="`${uid}-avatar-hint`"
              data-testid="bot-avatar-file"
              @change="uploadAvatar"
            />
            <div class="flex flex-wrap gap-2">
              <Button
                type="button"
                variant="outline"
                :disabled="busy"
                data-testid="bot-avatar-upload"
                @click="avatarInput?.click()"
              >
                {{ draftAvatar ? t('bot.profiles.avatarReplace') : t('bot.profiles.avatarUpload') }}
              </Button>
              <Button
                v-if="draftAvatar || avatarLoading"
                type="button"
                variant="ghost"
                :disabled="busy"
                data-testid="bot-avatar-remove"
                @click="avatarDraft.remove"
              >
                {{ t('bot.profiles.avatarRemove') }}
              </Button>
            </div>
            <p :id="`${uid}-avatar-hint`" class="text-xs leading-relaxed text-muted-foreground">
              {{ t('bot.profiles.avatarHint') }}
            </p>
          </div>
        </div>
        <p v-if="avatarLoading" role="status" class="text-xs text-muted-foreground">
          {{ t('bot.profiles.avatarProcessing') }}
        </p>
        <p v-if="avatarError" role="alert" class="text-sm text-destructive" data-testid="bot-avatar-error">
          {{
            t(`bot.profiles.avatarErrors.${avatarError}`, {
              dimension: MAX_BOT_AVATAR_DIMENSION,
              pixels: MAX_BOT_AVATAR_PIXELS.toLocaleString(),
            })
          }}
        </p>
      </div>
      <div class="space-y-1.5">
        <label :for="`${uid}-name`" class="text-sm font-medium">{{ t('bot.profiles.name') }}</label
        ><Input
          :id="`${uid}-name`"
          v-model="draftName"
          required
          maxlength="80"
          autocomplete="off"
          :placeholder="t('bot.profiles.nameHint')"
          data-testid="bot-edit-name"
          :disabled="busy"
        />
      </div>
      <BotAgentProfilePicker
        v-if="editorOpen"
        v-model="agentProfile"
        :disabled="busy"
        @create="creatingAgentProfile = true"
      />
      <BotPromptEditor
        v-if="editorOpen"
        :key="editing ? `${editing.id}:${editing.revision}` : 'new'"
        v-model="persona"
        :disabled="busy"
      />
      <p v-if="error" role="alert" class="whitespace-pre-wrap break-words text-sm text-destructive">{{ error }}</p>
      <button
        v-if="error && editing"
        type="button"
        class="text-sm text-primary underline"
        :disabled="busy"
        @click="edit(editing.id)"
      >
        {{ t('bot.profiles.reload') }}
      </button>
    </form>
    <template v-if="!creatingAgentProfile" #footer
      ><Button variant="outline" @click="cancelEditor">{{ t('common.cancel') }}</Button
      ><Button
        type="submit"
        :form="`${uid}-form`"
        class="ms-auto"
        :disabled="busy || avatarLoading || !draftName.trim() || byteCount > MAX_INSTRUCTIONS_BYTES"
        data-testid="bot-save"
        >{{ busy ? t('common.saving') : t('common.save') }}</Button
      ></template
    >
  </Dialog>

  <Dialog
    :return-focus="returnFocus"
    :open="!!archiveTarget"
    :title="t('bot.profiles.archiveNamed', { name: archiveTarget?.name || '' })"
    :description="t('bot.profiles.archiveHint')"
    @update:open="
      (value) => {
        if (!value && !busy) archiveTarget = null
      }
    "
  >
    <p class="text-sm text-muted-foreground">{{ t('bot.profiles.archiveIdle') }}</p>
    <p v-if="error" role="alert" class="mt-3 whitespace-pre-wrap break-words text-sm text-destructive">{{ error }}</p>
    <template #footer
      ><Button variant="outline" :disabled="busy" @click="archiveTarget = null">{{ t('common.cancel') }}</Button
      ><Button
        variant="destructive"
        class="ms-auto"
        :disabled="busy"
        data-testid="bot-archive-confirm"
        @click="archive"
        >{{ t('bot.profiles.archive') }}</Button
      ></template
    >
  </Dialog>
</template>
