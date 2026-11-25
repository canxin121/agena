<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiChat1Line, RiRefreshLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { listBots, type BotProfile } from '@/lib/bots'
import { integrationApi } from '@/lib/integrationApi'
import { permissionConfigErrorMessage } from '@/lib/permissionConfig'
import {
  permissionIdentityKey,
  permissionProfileDisplayName,
  type ChannelPermissionTarget,
  type PermissionCatalog,
} from '@/lib/permissionProfiles'
import { settingsText as st } from '@/i18n/settingsText'
import type { ChannelParticipant } from '@/generated/agenaState'
import SettingsSaveBar from './SettingsSaveBar.vue'
import SettingsListDetail from './workbench/SettingsListDetail.vue'
import {
  applyChannelAssignments,
  channelAssignmentRooms,
  mergeChannelParticipants,
  roomPermissionBindings,
  type ChannelRoom,
} from './channelAssignments'

const props = defineProps<{
  catalog: PermissionCatalog
  profiles: PermissionCatalog['profiles']
  target: ChannelPermissionTarget | null
  targetSource: number | null
  disabled: boolean
  saving: boolean
  save: (value: PermissionCatalog) => Promise<void>
  canDiscard: () => Promise<boolean>
}>()
const emit = defineEmits<{ refresh: [] }>()
const { t } = useI18n()

const bots = shallowRef<BotProfile[]>([])
const botId = ref('')
const linkedSource = ref<number | null>(null)
const source = computed(
  () => linkedSource.value ?? bots.value.find((bot) => bot.id === botId.value)?.local_session_id ?? null,
)
const rooms = shallowRef<ChannelRoom[]>([])
const roomId = ref('')
const roomQuery = ref('')
const detailOpen = ref(false)
const listDetail = ref<InstanceType<typeof SettingsListDetail> | null>(null)
watch(roomQuery, () => listDetail.value?.scrollListToTop())
const room = computed(() => rooms.value.find((entry) => entry.key === roomId.value))
const visibleRooms = computed(() => {
  const query = roomQuery.value.trim().toLowerCase()
  return rooms.value.filter((entry) =>
    [
      entry.title,
      entry.identity.resource,
      entry.identity.account,
      entry.platformTitle,
      t(`platforms.kind_${entry.kind}`),
    ].some((value) => value?.toLowerCase().includes(query)),
  )
})
const directoryLoading = ref(false)
const directoryError = ref('')
const missingRoom = ref(false)
const members = shallowRef<ChannelParticipant[]>([])
const memberQuery = ref('')
const membersLoading = ref(false)
const membersError = ref('')
const selected = shallowRef(new Set<string>())
const hasMore = ref(false)
let cursors: Array<string | undefined> = []
let directoryRequest: AbortController | null = null
let membersRequest: AbortController | null = null
let searchTimer: ReturnType<typeof setTimeout> | undefined
let initialized = false
let followedTarget = ''

const bindings = computed(() => (room.value ? roomPermissionBindings(props.catalog, room.value) : null))
const profileDraft = ref<string>()
const memberEdits = shallowRef(new Map<string, string>())
const dirty = computed(() => profileDraft.value !== undefined || memberEdits.value.size > 0)
const saved = ref(false)
const saveError = ref('')
const profileOptions = computed(() => Object.entries(props.profiles))
const roomProfile = computed(() => profileDraft.value ?? bindings.value?.profile ?? '')
const inheritRoomProfile = computed(() => {
  const profile = props.profiles[roomProfile.value]
  return profile
    ? t('channelAssignments.inheritProfile', { profile: permissionProfileDisplayName(roomProfile.value, profile) })
    : t('channelAssignments.inheritRoom')
})
const visibleMembers = computed(() => {
  const result = new Map<string, Pick<ChannelParticipant, 'actor' | 'display_name'>>(
    members.value.map((member) => [member.actor, member]),
  )
  const query = memberQuery.value.trim().toLowerCase()
  // A saved assignment remains editable even if the sender has no recent interaction.
  for (const actor of bindings.value?.members.keys() ?? []) {
    if (!result.has(actor) && actor.toLowerCase().includes(query)) result.set(actor, { actor })
  }
  return [...result.values()]
})
const visibleChecked = computed(
  () => visibleMembers.value.length > 0 && visibleMembers.value.every((member) => selected.value.has(member.actor)),
)
const bulkProfile = computed(() => {
  const profiles = new Set([...selected.value].map(memberProfile))
  return profiles.size === 1 ? [...profiles][0] : ' '
})

function memberProfile(actor: string) {
  return memberEdits.value.get(actor) ?? bindings.value?.members.get(actor)?.profile_id ?? ''
}

function discard() {
  profileDraft.value = undefined
  memberEdits.value = new Map()
  saved.value = false
  saveError.value = ''
}

function stopMembers() {
  clearTimeout(searchTimer)
  membersRequest?.abort()
  membersRequest = null
  membersLoading.value = false
}

async function fetchMembers(append = false, targetActor?: string) {
  const current = room.value
  const session = source.value
  if (!current || !session || (append && (!hasMore.value || membersLoading.value))) return
  stopMembers()
  const controller = new AbortController()
  membersRequest = controller
  membersLoading.value = true
  membersError.value = ''
  const term = memberQuery.value.trim() || undefined
  try {
    const pages = await Promise.all(
      current.locations.map(({ bindingId, kind }, index) => {
        const after = append ? cursors[index] : undefined
        if (append && !after) return null
        return integrationApi.participants(session, bindingId, current.identity.resource, kind, {
          after,
          search: term,
          signal: controller.signal,
        })
      }),
    )
    if (membersRequest !== controller) return
    members.value = mergeChannelParticipants(
      append ? members.value : [],
      pages.flatMap((page) => page?.participants ?? []),
    )
    if (targetActor && visibleMembers.value.some(({ actor }) => actor === targetActor))
      selected.value = new Set([targetActor])
    cursors = pages.map((page) => page?.next_after ?? undefined)
    hasMore.value = cursors.some(Boolean)
  } catch (reason) {
    if (membersRequest === controller) {
      controller.abort()
      membersError.value = permissionConfigErrorMessage(reason, st)
    }
  } finally {
    if (membersRequest === controller) {
      membersRequest = null
      membersLoading.value = false
    }
  }
}

function changeMemberQuery(value: string) {
  memberQuery.value = value
  stopMembers()
  members.value = []
  hasMore.value = false
  membersError.value = ''
  membersLoading.value = true
  searchTimer = setTimeout(() => void fetchMembers(), 180)
}

function showRoom(key: string, actor?: string) {
  stopMembers()
  discard()
  roomId.value = key
  missingRoom.value = false
  members.value = []
  selected.value = new Set()
  memberQuery.value = actor ?? ''
  membersError.value = ''
  hasMore.value = false
  cursors = []
  if (key) void fetchMembers(false, actor)
}

async function chooseRoom(key: string) {
  if (key !== roomId.value) {
    if (!(await props.canDiscard()) || !rooms.value.some((entry) => entry.key === key)) return
    showRoom(key)
  }
  detailOpen.value = true
}

async function loadRooms(controller: AbortController, target: ChannelPermissionTarget | null, preferred = '') {
  stopMembers()
  rooms.value = []
  showRoom('')
  const session = source.value
  if (!session) return
  const inventory = await integrationApi.platforms(session, controller.signal)
  if (directoryRequest !== controller) return
  rooms.value = channelAssignmentRooms(inventory)
  const key = target ? permissionIdentityKey(target, target.resource) : preferred
  const match = rooms.value.find((entry) => entry.key === key)
  showRoom(match?.key ?? rooms.value[0]?.key ?? '', match ? target?.actor : undefined)
  detailOpen.value = Boolean(match && (target || detailOpen.value))
  missingRoom.value = Boolean(target && !match)
}

async function readDirectory(work: (controller: AbortController) => Promise<void>) {
  directoryRequest?.abort()
  const controller = new AbortController()
  directoryRequest = controller
  directoryLoading.value = true
  directoryError.value = ''
  try {
    await work(controller)
  } catch (reason) {
    if (directoryRequest === controller) directoryError.value = permissionConfigErrorMessage(reason, st)
  } finally {
    if (directoryRequest === controller) {
      directoryRequest = null
      directoryLoading.value = false
    }
  }
}

async function refresh(target: ChannelPermissionTarget | null = null, targetSource: number | null = null) {
  return readDirectory(async (controller) => {
    const result = await listBots(controller.signal)
    if (directoryRequest !== controller) return
    bots.value = result.items.filter((entry) => !entry.archived).sort((a, b) => a.name.localeCompare(b.name))
    const matchingBot = targetSource ? bots.value.find((entry) => entry.local_session_id === targetSource) : undefined
    if (target && targetSource) linkedSource.value = matchingBot ? null : targetSource
    initialized = true
    if (target) roomQuery.value = ''
    const next = linkedSource.value
      ? undefined
      : (matchingBot ??
        bots.value.find((entry) => entry.id === botId.value) ??
        bots.value.find((entry) => entry.local_session_id) ??
        bots.value[0])
    const nextId = next?.id ?? ''
    const preferred = botId.value === nextId ? roomId.value : ''
    botId.value = nextId
    await loadRooms(controller, target, preferred)
  })
}

async function chooseBot(event: Event) {
  const input = event.target as HTMLSelectElement
  const id = input.value
  input.value = botId.value
  if (id === botId.value || !(await props.canDiscard())) return
  linkedSource.value = null
  botId.value = id
  roomQuery.value = ''
  detailOpen.value = false
  await readDirectory((controller) => loadRooms(controller, null))
}

function selectMembers(actors: string[], checked: boolean) {
  const next = new Set(selected.value)
  for (const actor of actors) {
    if (checked) next.add(actor)
    else next.delete(actor)
  }
  selected.value = next
}

function setMemberProfiles(actors: Iterable<string>, profileId: string) {
  const edits = new Map(memberEdits.value)
  for (const actor of actors) {
    const previous = bindings.value?.members.get(actor)?.profile_id ?? ''
    if (profileId === previous) edits.delete(actor)
    else edits.set(actor, profileId)
  }
  memberEdits.value = edits
  saved.value = false
}

function setProfile(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  profileDraft.value = value === bindings.value?.profile ? undefined : value
  saved.value = false
}

async function save() {
  if (!room.value || !dirty.value || props.saving || props.disabled) return
  saveError.value = ''
  try {
    await props.save(applyChannelAssignments(props.catalog, room.value, profileDraft.value, memberEdits.value))
    discard()
    saved.value = true
  } catch (reason) {
    saveError.value = permissionConfigErrorMessage(reason, st)
  }
}

watch(
  [() => props.target, () => props.targetSource],
  ([target, targetSource]) => {
    if (target) {
      const key = JSON.stringify([targetSource, target])
      if (key !== followedTarget) {
        followedTarget = key
        void refresh(target, targetSource)
        return
      }
    } else {
      followedTarget = ''
    }
    if (!initialized && !directoryRequest) void refresh()
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  directoryRequest?.abort()
  directoryRequest = null
  stopMembers()
})
defineExpose({ refresh, dirty, discard })
</script>

<template>
  <section
    class="assignment-workbench grid min-w-0 gap-3"
    data-channel-assignments
    :aria-busy="directoryLoading || saving"
  >
    <h2 class="text-sm font-semibold">{{ t('channelAssignments.title') }}</h2>
    <div class="settings-toolbar rounded-lg border border-border/70 bg-card/30 p-2.5">
      <label class="flex min-w-0 items-center gap-2 text-sm font-medium">
        {{ t('channelAssignments.bot') }}
        <select
          data-assignment-bot
          :value="botId"
          :disabled="!bots.length || directoryLoading || disabled || saving"
          class="min-w-0 rounded-md border border-input bg-background px-2.5 py-1.5 text-sm"
          @change="chooseBot"
        >
          <option v-if="linkedSource" value="">{{ t('channelAssignments.linkedSource') }}</option>
          <option v-for="bot in bots" :key="bot.id" :value="bot.id">{{ bot.name }}</option>
        </select>
      </label>
      <span v-if="saved && !dirty" class="text-xs text-muted-foreground" role="status">{{ $st('Saved') }}</span>
      <IconButton
        size="sm"
        :loading="directoryLoading"
        :disabled="disabled || saving"
        :aria-label="$st('Refresh')"
        :tooltip="$st('Refresh')"
        @click="emit('refresh')"
      >
        <RiRefreshLine class="h-4 w-4" aria-hidden="true" />
      </IconButton>
    </div>
    <p
      v-if="directoryError"
      role="alert"
      class="break-words rounded-md border border-destructive/40 p-2.5 text-sm text-destructive"
    >
      {{ directoryError }}
    </p>
    <p v-if="missingRoom" role="status" class="text-sm text-muted-foreground">
      {{ t('channelAssignments.missingRoom') }}
    </p>
    <p v-if="directoryLoading" role="status" class="text-sm text-muted-foreground">{{ $st('Loading…') }}</p>
    <p
      v-else-if="!rooms.length && !directoryError"
      class="rounded-md border border-dashed border-border/60 p-3 text-sm text-muted-foreground"
    >
      {{ t(source || bots.length ? 'channelAssignments.noRooms' : 'channelAssignments.noBots') }}
    </p>
    <SettingsListDetail
      v-else-if="rooms.length"
      ref="listDetail"
      v-model:detail-open="detailOpen"
      :list-label="t('channelAssignments.rooms')"
      :detail-label="t('channelAssignments.title')"
      :back-label="t('channelAssignments.rooms')"
      list-scroll-class="settings-list"
    >
      <template #list-header>
        <h2 data-settings-list-heading tabindex="-1" class="text-sm font-medium outline-none">
          {{ t('channelAssignments.rooms') }}
        </h2>
        <SearchInput
          v-model="roomQuery"
          :show-search-button="false"
          :placeholder="t('channelAssignments.roomSearch')"
          :input-aria-label="t('channelAssignments.roomSearch')"
        />
      </template>
      <template #list>
        <p v-if="!visibleRooms.length" class="p-1.5 text-sm text-muted-foreground">
          {{ t('channelAssignments.noMatchingRooms') }}
        </p>
        <button
          v-for="entry in visibleRooms"
          :key="entry.key"
          type="button"
          :aria-pressed="entry.key === roomId"
          :disabled="disabled || saving"
          :data-assignment-room="entry.identity.resource"
          :data-keyboard-item="entry.key"
          :data-keyboard-label="entry.title || entry.identity.resource"
          :data-keyboard-selected="entry.key === roomId"
          class="flex w-full min-w-0 items-start gap-2 rounded-md px-2 py-2 text-start hover:bg-muted/60 focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring disabled:opacity-60 aria-pressed:bg-muted"
          @click="chooseRoom(entry.key)"
        >
          <RiChat1Line class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span class="min-w-0">
            <span class="block truncate text-sm font-medium" :title="entry.title || entry.identity.resource">
              {{ entry.title || entry.identity.resource }}
            </span>
            <span class="block truncate text-xs text-muted-foreground">
              {{ entry.platformTitle }} · {{ t(`platforms.kind_${entry.kind}`) }} · {{ entry.identity.account }}
            </span>
          </span>
        </button>
      </template>
      <template #detail>
        <article v-if="room" class="settings-viewport-scroll grid content-start gap-3 p-3">
          <header class="min-w-0">
            <h2 data-settings-detail-heading tabindex="-1" class="break-words text-base font-semibold outline-none">
              {{ room.title || room.identity.resource }}
            </h2>
            <p class="break-all text-xs text-muted-foreground">
              {{ t(`platforms.kind_${room.kind}`) }} · {{ room.platformTitle }} · {{ room.identity.tenant }} /
              {{ room.identity.account }}
              <span v-if="room.title"> · {{ room.identity.resource }}</span>
            </p>
          </header>

          <label class="grid min-w-0 gap-1 text-xs font-medium text-muted-foreground">
            {{ $st('Default permission profile') }}
            <select
              data-assignment-room-profile
              :value="roomProfile"
              :disabled="disabled || saving"
              class="min-w-0 rounded-md border border-input bg-background px-2.5 py-1.5 text-sm text-foreground"
              @change="setProfile"
            >
              <option value="">{{ $st('Inherit') }}</option>
              <option v-for="[id, profile] in profileOptions" :key="id" :value="id">
                {{ permissionProfileDisplayName(id, profile) }}
              </option>
            </select>
          </label>

          <section class="grid min-w-0 gap-2 border-t border-border/70 pt-3" :aria-busy="membersLoading">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <h3 class="text-sm font-medium">{{ t('channelAssignments.members') }}</h3>
              <Button
                v-if="selected.size"
                size="sm"
                variant="ghost"
                type="button"
                :disabled="saving"
                @click="selected = new Set()"
                >{{ t('channelAssignments.clearSelection') }}</Button
              >
            </div>
            <SearchInput
              :model-value="memberQuery"
              :show-search-button="false"
              :placeholder="t('channelAssignments.memberSearch')"
              :input-aria-label="t('channelAssignments.memberSearch')"
              @update:model-value="changeMemberQuery"
            />
            <label v-if="selected.size > 1" class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              {{ t('channelAssignments.selected', { count: selected.size }) }}
              <select
                data-assignment-bulk-profile
                :value="bulkProfile"
                :aria-label="t('channelAssignments.bulkProfile')"
                :disabled="disabled || saving"
                class="min-w-0 max-w-full rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                @change="setMemberProfiles(selected, ($event.target as HTMLSelectElement).value)"
              >
                <option value=" " disabled>{{ t('channelAssignments.bulkProfile') }}</option>
                <option value="">{{ inheritRoomProfile }}</option>
                <option v-for="[id, profile] in profileOptions" :key="id" :value="id">
                  {{ permissionProfileDisplayName(id, profile) }}
                </option>
              </select>
            </label>
            <p v-if="membersError" role="alert" class="break-words text-xs text-destructive">{{ membersError }}</p>
            <label
              v-if="visibleMembers.length"
              class="flex items-center gap-2 border-b border-border/60 px-1.5 pb-1.5 text-xs text-muted-foreground"
            >
              <input
                type="checkbox"
                :checked="visibleChecked"
                :indeterminate="!visibleChecked && visibleMembers.some((member) => selected.has(member.actor))"
                :disabled="saving"
                class="accent-primary"
                @change="
                  selectMembers(
                    visibleMembers.map((member) => member.actor),
                    ($event.target as HTMLInputElement).checked,
                  )
                "
              />
              {{ t('channelAssignments.selectVisible') }}
            </label>
            <ul v-if="visibleMembers.length" class="max-h-72 divide-y divide-border/60 overflow-y-auto">
              <li
                v-for="member in visibleMembers"
                :key="member.actor"
                class="flex flex-wrap items-center justify-between gap-x-3 gap-y-2 px-1.5 py-2"
              >
                <label class="flex min-w-0 flex-1 cursor-pointer items-start gap-2 rounded-sm text-start">
                  <input
                    type="checkbox"
                    :data-assignment-member="member.actor"
                    :checked="selected.has(member.actor)"
                    :disabled="disabled || saving"
                    class="mt-0.5 shrink-0 accent-primary"
                    @change="selectMembers([member.actor], ($event.target as HTMLInputElement).checked)"
                  />
                  <span class="min-w-0">
                    <span class="block break-words text-sm font-medium">{{ member.display_name || member.actor }}</span>
                    <span v-if="member.display_name" class="block break-all text-xs text-muted-foreground">{{
                      member.actor
                    }}</span>
                  </span>
                </label>
                <select
                  :data-assignment-member-profile="member.actor"
                  :value="memberProfile(member.actor)"
                  :aria-label="t('channelAssignments.memberProfile', { name: member.display_name || member.actor })"
                  :disabled="disabled || saving"
                  class="min-w-0 max-w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  @change="setMemberProfiles([member.actor], ($event.target as HTMLSelectElement).value)"
                >
                  <option value="">{{ inheritRoomProfile }}</option>
                  <option v-for="[id, profile] in profileOptions" :key="id" :value="id">
                    {{ permissionProfileDisplayName(id, profile) }}
                  </option>
                </select>
              </li>
            </ul>
            <p v-else-if="membersLoading" class="text-xs text-muted-foreground" role="status">{{ $st('Loading…') }}</p>
            <p v-else-if="!membersError" class="text-xs text-muted-foreground">
              {{ t(memberQuery.trim() ? 'channelAssignments.noMatchingMembers' : 'platforms.noParticipants') }}
            </p>
            <Button
              v-if="hasMore"
              type="button"
              size="sm"
              variant="outline"
              class="justify-self-start"
              :disabled="membersLoading"
              @click="fetchMembers(true)"
              >{{ t('platforms.moreParticipants') }}</Button
            >
          </section>
          <SettingsSaveBar
            v-if="dirty || saving"
            data-assignment-save
            :dirty="dirty"
            :saving="saving"
            :disabled="disabled"
            :error="saveError"
            sticky
            @save="save"
            @discard="discard"
          />
        </article>
      </template>
    </SettingsListDetail>
  </section>
</template>

<style scoped>
.assignment-workbench {
  container: settings-workbench / inline-size;
}
</style>
