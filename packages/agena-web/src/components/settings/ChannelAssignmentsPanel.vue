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
import {
  applyChannelAssignments,
  channelAssignmentRooms,
  mergeChannelParticipants,
  roomPermissionBindings,
  type ChannelRoom,
} from './channelAssignments'

const props = defineProps<{
  active: boolean
  catalog: PermissionCatalog
  profiles: PermissionCatalog['profiles']
  target: ChannelPermissionTarget | null
  targetSource: number | null
  disabled: boolean
  saving: boolean
  save: (value: PermissionCatalog) => Promise<void>
  canDiscard: () => Promise<boolean>
}>()
const emit = defineEmits<{ refresh: []; groups: []; edit: [target: ChannelPermissionTarget] }>()
const { t } = useI18n()

const bots = shallowRef<BotProfile[]>([])
const botId = ref('')
const source = computed(() => bots.value.find((bot) => bot.id === botId.value)?.local_session_id ?? null)
const rooms = shallowRef<ChannelRoom[]>([])
const roomId = ref('')
const roomQuery = ref('')
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
const groupEdits = shallowRef(new Map<string, string[]>())
const dirty = computed(() => profileDraft.value !== undefined || groupEdits.value.size > 0)
const saved = ref(false)
const saveError = ref('')
const groupOptions = computed(() =>
  Object.entries(props.catalog.groups).sort(([, a], [, b]) => a.priority - b.priority || a.name.localeCompare(b.name)),
)
const visibleChecked = computed(
  () => members.value.length > 0 && members.value.every((member) => selected.value.has(member.actor)),
)
const selectedActor = computed(() => (selected.value.size === 1 ? selected.value.values().next().value : undefined))
const selectedMember = computed(() => members.value.find((member) => member.actor === selectedActor.value))

function groupIds(actor: string) {
  return groupEdits.value.get(actor) ?? bindings.value?.members.get(actor)?.group_ids ?? []
}
function groupName(id: string) {
  return props.catalog.groups[id]?.name || id
}
function inheritedGroups(actor: string) {
  return bindings.value?.accountMembers.get(actor)?.group_ids ?? []
}
const editableGroups = computed(() =>
  groupOptions.value.map(([id, group]) => {
    const count = [...selected.value].filter((actor) => groupIds(actor).includes(id)).length
    return {
      id,
      group,
      checked: count > 0 && count === selected.value.size,
      mixed: count > 0 && count < selected.value.size,
    }
  }),
)

function discard() {
  profileDraft.value = undefined
  groupEdits.value = new Map()
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
    if (targetActor && members.value.some(({ actor }) => actor === targetActor)) selected.value = new Set([targetActor])
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
  members.value = []
  selected.value = new Set()
  memberQuery.value = actor ?? ''
  membersError.value = ''
  hasMore.value = false
  cursors = []
  if (key) void fetchMembers(false, actor)
}

async function chooseRoom(key: string) {
  if (key !== roomId.value && (await props.canDiscard())) showRoom(key)
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
  if (target && !match) {
    showRoom(rooms.value[0]?.key ?? '')
    emit('edit', target)
    return
  }
  showRoom(match?.key ?? rooms.value[0]?.key ?? '', target?.actor)
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
    if (target && !matchingBot) {
      emit('edit', target)
      return
    }
    initialized = true
    if (target) roomQuery.value = ''
    const next =
      matchingBot ??
      bots.value.find((entry) => entry.id === botId.value) ??
      bots.value.find((entry) => entry.local_session_id) ??
      bots.value[0]
    const preferred = botId.value === next?.id ? roomId.value : ''
    botId.value = next?.id ?? ''
    await loadRooms(controller, target, preferred)
  })
}

async function chooseBot(event: Event) {
  const input = event.target as HTMLSelectElement
  const id = input.value
  input.value = botId.value
  if (id === botId.value || !(await props.canDiscard())) return
  botId.value = id
  roomQuery.value = ''
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

function setGroup(id: string, checked: boolean) {
  const edits = new Map(groupEdits.value)
  for (const actor of selected.value) {
    const previous = bindings.value?.members.get(actor)?.group_ids || []
    const next = new Set(groupIds(actor))
    if (checked) next.add(id)
    else next.delete(id)
    if (next.size === previous.length && previous.every((group) => next.has(group))) edits.delete(actor)
    else edits.set(actor, [...next])
  }
  groupEdits.value = edits
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
    await props.save(applyChannelAssignments(props.catalog, room.value, profileDraft.value, groupEdits.value))
    discard()
    saved.value = true
  } catch (reason) {
    saveError.value = permissionConfigErrorMessage(reason, st)
  }
}

watch(
  [() => props.active, () => props.target, () => props.targetSource],
  ([active, target, targetSource]) => {
    if (!active) return
    if (target && targetSource) {
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
  <section class="@container grid min-w-0 gap-3" data-channel-assignments :aria-busy="directoryLoading || saving">
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
    <p v-else-if="directoryLoading" role="status" class="text-sm text-muted-foreground">{{ $st('Loading…') }}</p>
    <p
      v-else-if="!bots.length || !rooms.length"
      class="rounded-md border border-dashed border-border/60 p-3 text-sm text-muted-foreground"
    >
      {{ t(bots.length ? 'channelAssignments.noRooms' : 'channelAssignments.noBots') }}
    </p>
    <div v-else class="assignment-layout grid min-w-0 items-start gap-3">
      <section class="grid min-w-0 gap-2 rounded-lg border border-border/70 bg-card/30 p-2.5">
        <h2 class="text-sm font-medium">{{ t('channelAssignments.rooms') }}</h2>
        <SearchInput
          v-model="roomQuery"
          :show-search-button="false"
          :placeholder="t('channelAssignments.roomSearch')"
          :input-aria-label="t('channelAssignments.roomSearch')"
        />
        <p v-if="!visibleRooms.length" class="p-1.5 text-sm text-muted-foreground">
          {{ t('channelAssignments.noMatchingRooms') }}
        </p>
        <ul v-else class="max-h-80 space-y-0.5 overflow-y-auto" :aria-label="t('channelAssignments.rooms')">
          <li v-for="entry in visibleRooms" :key="entry.key">
            <button
              type="button"
              :aria-pressed="entry.key === roomId"
              :disabled="disabled || saving"
              :data-assignment-room="entry.identity.resource"
              class="flex w-full min-w-0 items-start gap-2 rounded-md px-2 py-2 text-start hover:bg-muted/60 focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-60"
              :class="entry.key === roomId ? 'bg-muted' : ''"
              @click="chooseRoom(entry.key)"
            >
              <RiChat1Line class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
              <span class="min-w-0">
                <span class="block break-words text-sm font-medium">{{ entry.title || entry.identity.resource }}</span>
                <span class="block break-words text-xs text-muted-foreground"
                  >{{ entry.platformTitle }} · {{ t(`platforms.kind_${entry.kind}`) }} ·
                  {{ entry.identity.account }}</span
                >
              </span>
            </button>
          </li>
        </ul>
      </section>
      <article v-if="room" class="grid min-w-0 gap-3 rounded-lg border border-border/70 bg-card/30 p-3">
        <header class="min-w-0">
          <h2 class="break-all text-base font-semibold">{{ room.title || room.identity.resource }}</h2>
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
            :value="profileDraft ?? bindings?.profile ?? ''"
            :disabled="disabled || saving"
            class="min-w-0 rounded-md border border-input bg-background px-2.5 py-1.5 text-sm text-foreground"
            @change="setProfile"
          >
            <option value="">{{ $st('Inherit') }}</option>
            <option v-for="(profile, id) in profiles" :key="id" :value="id">
              {{ permissionProfileDisplayName(id, profile) }}
            </option>
          </select>
        </label>

        <section class="grid min-w-0 gap-2 border-t border-border/70 pt-3" :aria-busy="membersLoading">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <h3 class="text-sm font-medium">{{ t('platforms.participants') }}</h3>
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
          <div
            v-if="selected.size"
            class="grid gap-2 rounded-md border border-border/70 bg-background/60 p-2.5"
            data-selected-member-groups
          >
            <div class="flex flex-wrap items-center justify-between gap-1.5">
              <div>
                <h4 class="text-sm font-medium">{{ t('channelAssignments.roomGroups') }}</h4>
                <p class="text-xs text-muted-foreground">
                  {{
                    selectedActor
                      ? selectedMember?.display_name || selectedActor
                      : t('channelAssignments.selected', { count: selected.size })
                  }}
                </p>
              </div>
              <Button
                v-if="selectedActor"
                size="sm"
                variant="ghost"
                type="button"
                :disabled="disabled || saving"
                @click="emit('edit', { ...room.identity, actor: selectedActor })"
                >{{ t('channelAssignments.editIndividual') }}</Button
              >
            </div>
            <div v-if="editableGroups.length" class="grid gap-1.5 sm:grid-cols-2">
              <label
                v-for="{ id, group, checked, mixed } in editableGroups"
                :key="id"
                class="flex min-w-0 items-start gap-2 rounded-md px-1.5 py-1 text-sm hover:bg-muted/50"
              >
                <input
                  type="checkbox"
                  :data-assignment-group="id"
                  :checked="checked"
                  :indeterminate="mixed"
                  :disabled="disabled || saving"
                  class="mt-0.5 shrink-0 accent-primary"
                  @change="setGroup(id, ($event.target as HTMLInputElement).checked)"
                />
                <span class="break-words">{{ group.name || id }}</span>
              </label>
            </div>
            <Button
              v-else
              size="sm"
              variant="outline"
              type="button"
              class="justify-self-start"
              :disabled="saving"
              @click="emit('groups')"
              >{{ t('channelAssignments.manageGroups') }}</Button
            >
          </div>

          <p v-if="membersError" role="alert" class="break-words text-xs text-destructive">{{ membersError }}</p>
          <label
            v-if="members.length"
            class="flex items-center gap-2 border-b border-border/60 px-1.5 pb-1.5 text-xs text-muted-foreground"
          >
            <input
              type="checkbox"
              :checked="visibleChecked"
              :indeterminate="!visibleChecked && members.some((member) => selected.has(member.actor))"
              :disabled="saving"
              class="accent-primary"
              @change="
                selectMembers(
                  members.map((member) => member.actor),
                  ($event.target as HTMLInputElement).checked,
                )
              "
            />
            {{ t('channelAssignments.selectVisible') }}
          </label>
          <ul v-if="members.length" class="max-h-72 divide-y divide-border/60 overflow-y-auto">
            <li v-for="member in members" :key="member.actor">
              <label class="flex min-w-0 cursor-pointer items-start gap-2 rounded-sm px-1.5 py-2 hover:bg-muted/40">
                <input
                  type="checkbox"
                  :data-assignment-member="member.actor"
                  :checked="selected.has(member.actor)"
                  :disabled="saving"
                  class="mt-0.5 shrink-0 accent-primary"
                  @change="selectMembers([member.actor], ($event.target as HTMLInputElement).checked)"
                />
                <span class="grid min-w-0 gap-0.5">
                  <span class="break-words text-sm font-medium">{{ member.display_name || member.actor }}</span>
                  <span v-if="member.display_name" class="break-all text-xs text-muted-foreground">{{
                    member.actor
                  }}</span>
                  <span v-if="groupIds(member.actor).length" class="flex flex-wrap gap-1">
                    <span
                      v-for="id in groupIds(member.actor)"
                      :key="id"
                      class="break-words rounded bg-muted px-1.5 py-0.5 text-xs"
                      >{{ groupName(id) }}</span
                    >
                  </span>
                  <span v-if="inheritedGroups(member.actor).length" class="break-words text-xs text-muted-foreground">{{
                    t('channelAssignments.accountGroups', {
                      groups: inheritedGroups(member.actor).map(groupName).join(', '),
                    })
                  }}</span>
                </span>
              </label>
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
          :dirty="dirty"
          :saving="saving"
          :disabled="disabled"
          :error="saveError"
          sticky
          @save="save"
          @discard="discard"
        />
      </article>
    </div>
  </section>
</template>

<style scoped>
@container (min-width: 37rem) {
  .assignment-layout {
    grid-template-columns: minmax(11rem, 1fr) minmax(0, 2.1fr);
  }
}
</style>
