<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import SettingsCard from './SettingsCard.vue'
import Button from '@/components/ui/Button.vue'
import { confirmAction } from '@/lib/appConfirm'
import { parsePermissionConfig, permissionConfigErrorMessage } from '@/lib/permissionConfig'
import {
  emptyPermissionCatalog,
  permissionCatalogSnapshot,
  permissionGrantFromFields,
  permissionGroupReferences,
  permissionIdentityKey,
  permissionProfileReferences,
  readPermissionCatalog,
  type PermissionCatalog,
  type PermissionCatalogSection,
} from '@/lib/permissionProfiles'
import { getRuntimeSettingLayer, setRuntimeSetting } from '@/lib/runtimeSettings'
import { settingsText as st } from '@/i18n/settingsText'
import type { JsonValue } from '@/types/json'

type Draft = {
  id: string
  name: string
  description: string
  priority: number
  profile_id: string
  platform: string
  tenant: string
  account: string
  actor: string
  resource: string
  group_ids: string[]
  permissionText: string
  grantEnabled: boolean
  toolsText: string
  hostsText: string
  mcpText: string
  privateMemory: boolean
  sourceWorkspace: boolean
}

const catalog = ref<PermissionCatalog>(emptyPermissionCatalog())
const revision = ref<string | undefined>()
const section = ref<PermissionCatalogSection>('profiles')
const selectedId = ref<string | null>(null)
const draft = ref<Draft | null>(null)
const original = ref('')
const search = ref('')
const loading = ref(false)
const loaded = ref(false)
const saving = ref(false)
const error = ref('')
const saved = ref(false)
let generation = 0

const tabs = computed(() => [
  { id: 'profiles' as const, label: st('Permission profiles') },
  { id: 'groups' as const, label: st('User groups') },
  { id: 'users' as const, label: st('Users') },
  { id: 'conversations' as const, label: st('Conversations') },
])
const dirty = computed(() => draft.value !== null && JSON.stringify(draft.value) !== original.value)
const identityFields = computed(() => [
  { key: 'platform' as const, label: st('Platform') },
  { key: 'tenant' as const, label: st('Tenant ID') },
  { key: 'account' as const, label: st('Bot account ID') },
  section.value === 'users'
    ? { key: 'actor' as const, label: st('Sender ID') }
    : { key: 'resource' as const, label: st('Conversation ID') },
])
const entries = computed(() => {
  switch (section.value) {
    case 'profiles':
      return Object.entries(catalog.value.profiles).map(([id, profile]) => ({
        id,
        title: profile.name || id,
        detail: profile.description || id,
      }))
    case 'groups':
      return Object.entries(catalog.value.groups).map(([id, group]) => ({
        id,
        title: group.name || id,
        detail: st('Priority: {priority}', { priority: group.priority }),
      }))
    case 'users':
      return catalog.value.users.map((user) => ({
        id: permissionIdentityKey(user, user.actor),
        title: user.actor,
        detail: [user.platform, user.tenant, user.account].join(' / '),
      }))
    case 'conversations':
      return catalog.value.conversations.map((conversation) => ({
        id: permissionIdentityKey(conversation, conversation.resource),
        title: conversation.resource,
        detail: [conversation.platform, conversation.tenant, conversation.account].join(' / '),
      }))
  }
})
const filteredEntries = computed(() => {
  const query = search.value.trim().toLowerCase()
  return entries.value.filter((entry) =>
    [entry.id, entry.title, entry.detail].some((text) => text.toLowerCase().includes(query)),
  )
})
const profileOptions = computed(() => Object.entries(catalog.value.profiles))
const groupOptions = computed(() => Object.entries(catalog.value.groups).sort((a, b) => a[1].priority - b[1].priority))
const profileUsage = computed(() =>
  selectedId.value && section.value === 'profiles'
    ? permissionProfileReferences(catalog.value, selectedId.value)
    : selectedId.value && section.value === 'groups'
      ? permissionGroupReferences(catalog.value, selectedId.value)
      : [],
)
const selectedGroups = computed(() => groupOptions.value.filter(([id]) => draft.value?.group_ids.includes(id)))

function blankDraft(): Draft {
  return {
    id: '',
    name: '',
    description: '',
    priority: 0,
    profile_id: '',
    platform: '',
    tenant: '',
    account: '',
    actor: '',
    resource: '',
    group_ids: [],
    permissionText: '{}',
    grantEnabled: false,
    toolsText: '',
    hostsText: '',
    mcpText: '{}',
    privateMemory: false,
    sourceWorkspace: false,
  }
}

function showEntry(id: string | null) {
  selectedId.value = id
  saved.value = false
  if (id === null) {
    draft.value = null
    original.value = ''
    return
  }
  const next = blankDraft()
  next.id = id
  if (section.value === 'profiles') {
    const profile = catalog.value.profiles[id]
    if (!profile) return showEntry(null)
    next.name = profile.name
    next.description = profile.description || ''
    next.permissionText = JSON.stringify(profile.permission || {}, null, 2)
    next.grantEnabled = Boolean(profile.grant)
    next.toolsText = (profile.grant?.tools || []).join('\n')
    next.hostsText = (profile.grant?.network_hosts || []).join('\n')
    next.mcpText = JSON.stringify(profile.grant?.mcp_tools || {}, null, 2)
    next.privateMemory = profile.grant?.private_memory || false
    next.sourceWorkspace = profile.grant?.source_workspace || false
  } else if (section.value === 'groups') {
    const group = catalog.value.groups[id]
    if (!group) return showEntry(null)
    Object.assign(next, { name: group.name, priority: group.priority, profile_id: group.profile_id || '' })
  } else if (section.value === 'users') {
    const user = catalog.value.users.find((item) => permissionIdentityKey(item, item.actor) === id)
    if (!user) return showEntry(null)
    Object.assign(next, user, {
      group_ids: [...user.group_ids],
      profile_id: user.profile_id || '',
      permissionText: JSON.stringify(user.overrides || {}, null, 2),
    })
  } else {
    const conversation = catalog.value.conversations.find((item) => permissionIdentityKey(item, item.resource) === id)
    if (!conversation) return showEntry(null)
    Object.assign(next, conversation, { profile_id: conversation.default_profile_id })
  }
  draft.value = next
  original.value = JSON.stringify(next)
}

async function canDiscard(): Promise<boolean> {
  if (saving.value) return false
  if (!dirty.value) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    description: st('You have unsaved changes.'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}

async function load() {
  const token = ++generation
  loading.value = true
  error.value = ''
  try {
    const response = await getRuntimeSettingLayer('global', 'access_control')
    if (token !== generation) return
    catalog.value = readPermissionCatalog(response.value)
    revision.value = response.revision
    loaded.value = true
    showEntry(
      entries.value.some((entry) => entry.id === selectedId.value) ? selectedId.value : (entries.value[0]?.id ?? null),
    )
  } catch (reason) {
    if (token === generation) error.value = permissionConfigErrorMessage(reason, st)
  } finally {
    if (token === generation) loading.value = false
  }
}

async function refresh() {
  if (await canDiscard()) await load()
}

async function selectSection(next: PermissionCatalogSection) {
  if (next === section.value || !(await canDiscard())) return
  section.value = next
  search.value = ''
  error.value = ''
  showEntry(entries.value[0]?.id ?? null)
}

async function selectEntry(id: string) {
  if (id === selectedId.value || !(await canDiscard())) return
  error.value = ''
  showEntry(id)
}

async function addEntry(copy = false) {
  if (!(await canDiscard())) return
  const next = copy && draft.value ? { ...draft.value, group_ids: [...draft.value.group_ids] } : blankDraft()
  if (section.value === 'profiles' || section.value === 'groups') {
    next.id = (section.value === 'profiles' ? 'profile-' : 'group-') + crypto.randomUUID()
    if (copy) next.name = st('{name} copy', { name: next.name })
  }
  selectedId.value = null
  draft.value = next
  original.value = ''
  error.value = ''
  saved.value = false
}

function updatedCatalog(): { value: PermissionCatalog; id: string } {
  if (!draft.value) throw new Error(st('Select an entry to edit.'))
  const d = draft.value
  const next = readPermissionCatalog(catalog.value as unknown as JsonValue)
  const identity = {
    platform: d.platform.trim(),
    tenant: d.tenant.trim(),
    account: d.account.trim(),
  }
  if (section.value === 'users' || section.value === 'conversations') {
    if (
      Object.values(identity).some((part) => !part) ||
      !(section.value === 'users' ? d.actor.trim() : d.resource.trim())
    )
      throw new Error(st('Complete every identity field using the IDs from the incoming message.'))
  }
  let id = d.id.trim()
  if (section.value === 'profiles' || section.value === 'groups') {
    if (!id || !d.name.trim()) throw new Error(st('Enter an ID and a name.'))
    if (selectedId.value === null && Object.hasOwn(next[section.value], id))
      throw new Error(st('This ID already exists.'))
  }
  if (section.value === 'profiles') {
    const permission = parsePermissionConfig(JSON.parse(d.permissionText))
    const grant = d.grantEnabled
      ? permissionGrantFromFields(d.toolsText, d.hostsText, JSON.parse(d.mcpText), d.privateMemory, d.sourceWorkspace)
      : undefined
    if (grant === null) throw new Error(st('MCP grants must map server IDs to lists of tool names.'))
    next.profiles = {
      ...next.profiles,
      [id]: {
        name: d.name.trim(),
        description: d.description.trim(),
        revision: (selectedId.value ? next.profiles[selectedId.value]?.revision || 0 : 0) + 1,
        permission,
        ...(grant ? { grant } : {}),
      },
    }
  } else if (section.value === 'groups') {
    if (!Number.isInteger(d.priority)) throw new Error(st('Priority must be an integer.'))
    next.groups = {
      ...next.groups,
      [id]: { name: d.name.trim(), priority: d.priority, ...(d.profile_id ? { profile_id: d.profile_id } : {}) },
    }
  } else if (section.value === 'users') {
    const user = {
      ...identity,
      actor: d.actor.trim(),
      group_ids: [...d.group_ids],
      ...(d.profile_id ? { profile_id: d.profile_id } : {}),
      overrides: parsePermissionConfig(JSON.parse(d.permissionText)),
    }
    id = permissionIdentityKey(user, user.actor)
    if (next.users.some((item) => permissionIdentityKey(item, item.actor) === id && id !== selectedId.value))
      throw new Error(st('This identity already has a configuration.'))
    const index = next.users.findIndex((item) => permissionIdentityKey(item, item.actor) === selectedId.value)
    if (index < 0) next.users.push(user)
    else next.users[index] = user
  } else {
    if (!d.profile_id) throw new Error(st('Choose a permission profile.'))
    const conversation = { ...identity, resource: d.resource.trim(), default_profile_id: d.profile_id }
    id = permissionIdentityKey(conversation, conversation.resource)
    if (next.conversations.some((item) => permissionIdentityKey(item, item.resource) === id && id !== selectedId.value))
      throw new Error(st('This identity already has a configuration.'))
    const index = next.conversations.findIndex(
      (item) => permissionIdentityKey(item, item.resource) === selectedId.value,
    )
    if (index < 0) next.conversations.push(conversation)
    else next.conversations[index] = conversation
  }
  return { value: next, id }
}

async function persist(next: PermissionCatalog, id: string | null) {
  const latest = readPermissionCatalog((await getRuntimeSettingLayer('global', 'access_control')).value)
  if (permissionCatalogSnapshot(latest) !== permissionCatalogSnapshot(catalog.value))
    throw new Error(
      st('Permission configurations changed elsewhere. Your draft is preserved. Refresh before saving again.'),
    )
  if (revision.value === undefined) throw new Error(st('The configuration revision is missing. Refresh and try again.'))
  const response = await setRuntimeSetting(
    'access_control',
    next as unknown as JsonValue,
    { expected_revision: revision.value },
    'global',
  )
  catalog.value = response.current === undefined ? next : readPermissionCatalog(response.current)
  revision.value = response.after_revision
  showEntry(id)
  saved.value = true
}

async function save() {
  if (saving.value || !draft.value) return
  saving.value = true
  error.value = ''
  saved.value = false
  try {
    const next = updatedCatalog()
    await persist(next.value, next.id)
  } catch (reason) {
    error.value = permissionConfigErrorMessage(reason, st)
  } finally {
    saving.value = false
  }
}

async function removeEntry() {
  if (saving.value || selectedId.value === null) return
  if (profileUsage.value.length) {
    error.value = st('This entry is in use. Reassign or remove its bindings before deleting it.')
    return
  }
  const id = selectedId.value
  if (
    !(await confirmAction({
      title: st('Delete configuration?'),
      description: st('The selected configuration will be removed.'),
      confirmText: st('Delete'),
      cancelText: st('Cancel'),
    }))
  )
    return
  saving.value = true
  error.value = ''
  try {
    const next = readPermissionCatalog(catalog.value as unknown as JsonValue)
    if (section.value === 'profiles') delete next.profiles[id]
    else if (section.value === 'groups') delete next.groups[id]
    else if (section.value === 'users')
      next.users = next.users.filter((user) => permissionIdentityKey(user, user.actor) !== id)
    else next.conversations = next.conversations.filter((item) => permissionIdentityKey(item, item.resource) !== id)
    await persist(next, null)
  } catch (reason) {
    error.value = permissionConfigErrorMessage(reason, st)
  } finally {
    saving.value = false
  }
}

function beforeUnload(event: BeforeUnloadEvent) {
  if (dirty.value) {
    event.preventDefault()
    event.returnValue = ''
  }
}
onMounted(() => {
  void load()
  window.addEventListener('beforeunload', beforeUnload)
})
onBeforeUnmount(() => {
  generation += 1
  window.removeEventListener('beforeunload', beforeUnload)
})
onBeforeRouteLeave(canDiscard)
onBeforeRouteUpdate(canDiscard)
defineExpose({ refresh })
</script>

<template>
  <div class="grid min-w-0 gap-4" :aria-busy="loading || saving">
    <SettingsCard
      :title="$st('Permission profiles & users')"
      :description="
        $st('Manage reusable permissions and assign them to groups, people, and conversations across platforms.')
      "
    >
      <p class="mb-3 text-xs text-muted-foreground">
        {{
          $st(
            'Global configuration. Identity fields match exact platform, tenant, bot account, and sender or conversation IDs.',
          )
        }}
      </p>
      <details class="mb-4 rounded-md border border-border/60 p-3 text-xs text-muted-foreground">
        <summary class="cursor-pointer font-medium text-foreground">{{ $st('How permissions combine') }}</summary>
        <p class="mt-2">
          {{
            $st(
              'Order: existing permissions → conversation default → groups from lowest to highest priority → user profile → personal overrides. Existing permission ceilings still apply.',
            )
          }}
        </p>
        <p class="mt-2">
          {{
            $st(
              'Groups with the same priority must agree on overlapping settings. Conflicts and missing references are rejected when saving.',
            )
          }}
        </p>
      </details>
      <div class="mb-4 flex flex-wrap gap-2" :aria-label="$st('Permission configuration categories')">
        <Button
          v-for="tab in tabs"
          :key="tab.id"
          size="sm"
          :variant="section === tab.id ? 'secondary' : 'ghost'"
          :disabled="saving"
          :aria-pressed="section === tab.id"
          @click="selectSection(tab.id)"
          >{{ tab.label }}</Button
        >
      </div>
      <div v-if="error" class="mb-3 rounded-md border border-destructive/40 p-3 text-xs text-destructive" role="alert">
        {{ error }}
      </div>
      <p v-if="saved && !dirty" class="mb-3 text-xs text-muted-foreground" role="status">{{ $st('Saved') }}</p>
      <p v-if="loading" class="text-sm text-muted-foreground" role="status">{{ $st('Loading…') }}</p>
      <div v-else-if="loaded" class="grid min-w-0 gap-4 lg:grid-cols-[minmax(12rem,0.8fr)_minmax(0,1.6fr)]">
        <div class="grid min-w-0 content-start gap-3">
          <div class="flex flex-wrap justify-between gap-2">
            <Button size="sm" variant="outline" :disabled="saving" @click="addEntry()">{{ $st('Add') }}</Button>
            <Button size="sm" variant="ghost" :disabled="saving" @click="refresh">{{ $st('Refresh') }}</Button>
          </div>
          <input
            v-model="search"
            :aria-label="$st('Search configurations')"
            :placeholder="$st('Search configurations')"
            class="min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm"
          />
          <div class="grid max-h-[35rem] min-w-0 gap-1 overflow-auto">
            <button
              v-for="entry in filteredEntries"
              :key="entry.id"
              type="button"
              :disabled="saving"
              :aria-pressed="selectedId === entry.id"
              class="min-w-0 rounded-md border px-3 py-2 text-left hover:bg-muted/50"
              :class="selectedId === entry.id ? 'border-primary/50 bg-primary/10' : 'border-border/60'"
              @click="selectEntry(entry.id)"
            >
              <span class="block truncate text-sm font-medium">{{ entry.title }}</span>
              <span class="block truncate text-xs text-muted-foreground">{{ entry.detail }}</span>
            </button>
            <p v-if="!filteredEntries.length" class="p-2 text-sm text-muted-foreground">
              {{ $st('No configurations found. Add one to get started.') }}
            </p>
          </div>
        </div>

        <form v-if="draft" class="grid min-w-0 content-start gap-3" @submit.prevent="save">
          <fieldset :disabled="saving" class="grid min-w-0 gap-3">
            <template v-if="section === 'profiles' || section === 'groups'">
              <label class="grid gap-1 text-xs text-muted-foreground">
                {{ $st('ID') }}
                <input
                  v-model="draft.id"
                  :readonly="selectedId !== null"
                  required
                  class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                />
              </label>
              <label class="grid gap-1 text-xs text-muted-foreground">
                {{ $st('Name') }}
                <input
                  v-model="draft.name"
                  required
                  class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                />
              </label>
            </template>

            <template v-if="section === 'users' || section === 'conversations'">
              <p class="text-xs text-muted-foreground">
                {{
                  $st(
                    'Copy identity IDs from trusted incoming message metadata. All four fields are required; display names are not identity IDs.',
                  )
                }}
              </p>
              <div class="grid gap-3 sm:grid-cols-2">
                <label
                  v-for="field in identityFields"
                  :key="field.key"
                  class="grid gap-1 text-xs text-muted-foreground"
                >
                  {{ field.label }}
                  <input
                    v-model="draft[field.key]"
                    required
                    class="min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                  />
                </label>
              </div>
            </template>

            <label v-if="section === 'groups'" class="grid gap-1 text-xs text-muted-foreground">
              {{ $st('Priority') }}
              <input
                v-model.number="draft.priority"
                type="number"
                step="1"
                required
                class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
              />
              <span>{{ $st('Higher-priority groups apply later.') }}</span>
            </label>

            <template v-if="section === 'users'">
              <div class="grid gap-2 rounded-md border border-border/60 p-3">
                <span class="text-xs font-medium">{{ $st('Group memberships') }}</span>
                <label v-for="[id, group] in groupOptions" :key="id" class="flex items-center gap-2 text-sm">
                  <input v-model="draft.group_ids" type="checkbox" :value="id" />{{ group.name || id }}
                  <span class="text-xs text-muted-foreground">{{
                    $st('Priority: {priority}', { priority: group.priority })
                  }}</span>
                </label>
                <span v-if="!groupOptions.length" class="text-xs text-muted-foreground">{{
                  $st('Create a user group to assign memberships.')
                }}</span>
              </div>
              <p v-if="selectedGroups.length" class="text-xs text-muted-foreground">
                {{ $st('Group application order') }}:
                {{ selectedGroups.map(([id, group]) => group.name || id).join(' → ') }}
              </p>
            </template>

            <label v-if="section !== 'profiles'" class="grid gap-1 text-xs text-muted-foreground">
              {{ section === 'conversations' ? $st('Default permission profile') : $st('Permission profile') }}
              <select
                v-model="draft.profile_id"
                :required="section === 'conversations'"
                class="min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
              >
                <option value="">
                  {{ section === 'conversations' ? $st('Choose a permission profile.') : $st('Inherit') }}
                </option>
                <option v-for="[id, profile] in profileOptions" :key="id" :value="id">
                  {{ profile.name || id }} · {{ id }}
                </option>
              </select>
            </label>

            <label v-if="section === 'profiles'" class="grid gap-1 text-xs text-muted-foreground">
              {{ $st('Description') }}
              <textarea
                v-model="draft.description"
                rows="2"
                class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
              />
            </label>

            <label
              v-if="section === 'profiles' || section === 'users'"
              class="grid gap-1 text-xs text-muted-foreground"
            >
              {{
                section === 'profiles' ? $st('Permission rules (JSON)') : $st('Personal permission overrides (JSON)')
              }}
              <textarea
                v-model="draft.permissionText"
                rows="10"
                spellcheck="false"
                class="min-h-40 w-full resize-y rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground"
              />
              <span>{{
                $st(
                  'Uses the existing path, network, tools, and approval_model permission fields. Leave an empty object to inherit.',
                )
              }}</span>
            </label>

            <template v-if="section === 'profiles'">
              <label class="flex items-center gap-2 text-sm">
                <input v-model="draft.grantEnabled" type="checkbox" />{{
                  $st('Set channel capabilities for this profile')
                }}
              </label>
              <p class="text-xs text-muted-foreground">
                {{
                  $st(
                    'An unset capability grant inherits the earlier grant. When enabled, this profile replaces it. Granted tools still pass permission checks.',
                  )
                }}
              </p>
              <div v-if="draft.grantEnabled" class="grid gap-3 rounded-md border border-border/60 p-3">
                <label class="grid gap-1 text-xs text-muted-foreground">
                  {{ $st('Granted tool IDs') }}
                  <textarea
                    v-model="draft.toolsText"
                    rows="4"
                    :placeholder="$st('One full tool ID per line.')"
                    class="rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground"
                  />
                </label>
                <label class="grid gap-1 text-xs text-muted-foreground">
                  {{ $st('MCP tool grants (JSON)') }}
                  <textarea
                    v-model="draft.mcpText"
                    rows="4"
                    spellcheck="false"
                    class="rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground"
                  />
                  <span>{{ $st('Map each MCP server ID to an array of allowed tool names.') }}</span>
                </label>
                <label class="grid gap-1 text-xs text-muted-foreground">
                  {{ $st('Allowed network hosts') }}
                  <textarea
                    v-model="draft.hostsText"
                    rows="3"
                    :placeholder="$st('One allowed host per line.')"
                    class="rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground"
                  />
                </label>
                <label class="flex items-center gap-2 text-sm"
                  ><input v-model="draft.privateMemory" type="checkbox" />{{ $st('Allow private memory') }}</label
                >
                <label class="flex items-center gap-2 text-sm"
                  ><input v-model="draft.sourceWorkspace" type="checkbox" />{{
                    $st('Allow source workspace access')
                  }}</label
                >
              </div>
            </template>
          </fieldset>

          <p v-if="profileUsage.length" class="text-xs text-muted-foreground">
            {{ $st('Used by {count} bindings. Reassign them before deleting.', { count: profileUsage.length }) }}
          </p>
          <div class="flex flex-wrap gap-2">
            <Button type="submit" size="sm" :disabled="saving || !dirty">{{
              saving ? $st('Saving…') : $st('Save')
            }}</Button>
            <Button
              v-if="section === 'profiles'"
              type="button"
              size="sm"
              variant="outline"
              :disabled="saving"
              @click="addEntry(true)"
              >{{ $st('Copy profile') }}</Button
            >
            <Button
              v-if="selectedId !== null"
              type="button"
              size="sm"
              variant="ghost"
              :disabled="saving || profileUsage.length > 0"
              @click="removeEntry"
              >{{ $st('Delete') }}</Button
            >
          </div>
        </form>
        <p v-else class="p-3 text-sm text-muted-foreground">{{ $st('Select an entry to edit.') }}</p>
      </div>
      <Button v-else-if="!loading" size="sm" variant="outline" @click="load">{{ $st('Retry') }}</Button>
    </SettingsCard>
  </div>
</template>
