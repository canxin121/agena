<script setup lang="ts">
import { useSettingsDraftGuard } from './useSettingsDraftGuard'
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { RiAddLine, RiDeleteBinLine, RiRefreshLine, RiShieldCheckLine } from '@remixicon/vue'
import SettingsDisclosureRow from './SettingsDisclosureRow.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import ChannelAssignmentsPanel from './ChannelAssignmentsPanel.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { confirmAction } from '@/lib/appConfirm'
import {
  completePermissionConfig,
  parsePermissionConfig,
  permissionConfigErrorMessage,
  type PermissionConfig,
} from '@/lib/permissionConfig'
import {
  emptyPermissionCatalog,
  permissionCatalogSnapshot,
  permissionGrantFromFields,
  permissionProfileDisplayName,
  permissionProfileReferences,
  readChannelPermissionTarget,
  readPermissionCatalog,
  type PermissionCatalog,
} from '@/lib/permissionProfiles'
import { getRuntimeSettingLayer, setRuntimeSetting } from '@/lib/runtimeSettings'
import { settingsText as st } from '@/i18n/settingsText'
import type { JsonValue } from '@/types/json'

type Draft = {
  id: string
  name: string
  description: string
  permission: PermissionConfig
  complete: boolean
  grantEnabled: boolean
  hostsText: string
  mcp: Array<{ server: string; tools: string }>
  privateMemory: boolean
  sourceWorkspace: boolean
}

const props = defineProps<{ assignments?: boolean }>()
const route = useRoute()
const channelTarget = computed(() => (props.assignments ? readChannelPermissionTarget(route.query.assignment) : null))
const channelSource = computed(() => {
  const value = typeof route.query.channelSource === 'string' ? Number(route.query.channelSource) : 0
  return channelTarget.value && Number.isSafeInteger(value) && value > 0 ? value : null
})
const channelPanel = ref<InstanceType<typeof ChannelAssignmentsPanel> | null>(null)
const catalog = ref<PermissionCatalog>(emptyPermissionCatalog())
const factoryProfiles = ref<PermissionCatalog['profiles']>({})
const profiles = computed(() => ({ ...factoryProfiles.value, ...catalog.value.profiles }))
const revision = ref<string | undefined>()
const selectedId = ref<string | null>(null)
const draft = ref<Draft | null>(null)
const original = ref('')
const search = ref('')
const loading = ref(false)
const loaded = ref(false)
const saving = ref(false)
const error = ref('')
const saved = ref(false)
const PermissionStudioPanel = defineAsyncComponent(() => import('./PermissionStudioPanel.vue'))
const permissionEditors = ref<{ pendingRule: boolean; applyPendingRule: () => boolean }[]>([])
const editorKey = ref(0)
let generation = 0

const dirty = computed(() =>
  props.assignments
    ? channelPanel.value?.dirty === true
    : draft.value !== null &&
      (JSON.stringify(draft.value) !== original.value || permissionEditors.value[0]?.pendingRule === true),
)
const entries = computed(() =>
  Object.entries(profiles.value).map(([id, profile]) => ({
    id,
    title: permissionProfileDisplayName(id, profile),
    detail: profile.description || id,
  })),
)
const filteredEntries = computed(() => {
  const query = search.value.trim().toLowerCase()
  return entries.value
    .filter(
      (entry) =>
        (selectedId.value === entry.id && dirty.value) ||
        [entry.id, entry.title, entry.detail].some((text) => text.toLowerCase().includes(query)),
    )
    .sort(
      (left, right) =>
        Number(Object.hasOwn(factoryProfiles.value, right.id)) -
          Number(Object.hasOwn(factoryProfiles.value, left.id)) || left.title.localeCompare(right.title),
    )
})
const rows = computed(() => {
  const items = filteredEntries.value.map((entry) => ({ ...entry, key: entry.id, open: selectedId.value === entry.id }))
  if (draft.value && selectedId.value === null) {
    items.unshift({
      id: '',
      key: '',
      open: true,
      title: draft.value.name || st('New permission profile'),
      detail: draft.value.id,
    })
  }
  return items
})
const profileUsage = computed(() =>
  selectedId.value ? permissionProfileReferences(catalog.value, selectedId.value) : [],
)

function blankDraft(): Draft {
  return {
    id: '',
    name: '',
    description: '',
    permission: {},
    complete: false,
    grantEnabled: false,
    hostsText: '',
    mcp: [],
    privateMemory: false,
    sourceWorkspace: false,
  }
}

function showEntry(id: string | null) {
  editorKey.value++
  selectedId.value = id
  saved.value = false
  if (id === null) {
    draft.value = null
    original.value = ''
    return
  }
  const next = blankDraft()
  next.id = id
  const profile = profiles.value[id]
  if (!profile) return showEntry(null)
  next.name = profile.name
  next.description = profile.description || ''
  next.complete = profile.complete === true
  next.permission = parsePermissionConfig(profile.permission)
  if (next.complete) next.permission = completePermissionConfig(next.permission)
  next.grantEnabled = next.complete || Boolean(profile.grant)
  next.hostsText = (profile.grant?.network_hosts || []).join('\n')
  next.mcp = Object.entries(profile.grant?.mcp_tools || {}).map(([server, tools]) => ({
    server,
    tools: tools.join('\n'),
  }))
  next.privateMemory = profile.grant?.private_memory || false
  next.sourceWorkspace = profile.grant?.source_workspace || false
  draft.value = next
  original.value = JSON.stringify(next)
}

const { canDiscard } = useSettingsDraftGuard({
  dirty: () => dirty.value,
  busy: () => saving.value,
  unload: () => dirty.value,
  dialog: () => ({ description: st('You have unsaved changes.') }),
})

async function load() {
  const token = ++generation
  loading.value = true
  error.value = ''
  try {
    const response = await getRuntimeSettingLayer('global', 'access_control')
    if (token !== generation) return
    catalog.value = readPermissionCatalog(response.value)
    factoryProfiles.value = readPermissionCatalog(response.default_value).profiles
    revision.value = response.revision
    if (!props.assignments) {
      showEntry(entries.value.some((entry) => entry.id === selectedId.value) ? selectedId.value : null)
    }
    loaded.value = true
  } catch (reason) {
    if (token === generation) error.value = permissionConfigErrorMessage(reason, st)
  } finally {
    if (token === generation) loading.value = false
  }
}

async function refresh() {
  if (!(await canDiscard())) return
  const channel = channelPanel.value
  channel?.discard()
  await Promise.all([load(), channel?.refresh()])
}

async function selectEntry(id: string) {
  if (id === selectedId.value || !(await canDiscard())) return
  error.value = ''
  showEntry(id)
}

async function toggleEntry(id: string) {
  if (id && id !== selectedId.value) return selectEntry(id)
  if (await canDiscard()) showEntry(null)
}

async function addEntry(copy = false) {
  if (!(await canDiscard())) return
  const next: Draft = copy && draft.value ? JSON.parse(JSON.stringify(draft.value)) : blankDraft()
  next.id = 'profile-' + crypto.randomUUID()
  if (copy) next.name = st('{name} copy', { name: next.name })
  setComplete(next)
  editorKey.value++
  selectedId.value = null
  draft.value = next
  original.value = ''
  error.value = ''
  saved.value = false
}

function setComplete(entry: Draft) {
  entry.complete = true
  entry.grantEnabled = true
  entry.permission = completePermissionConfig(entry.permission)
}

function updatedCatalog(): { value: PermissionCatalog; id: string } {
  if (!draft.value) throw new Error(st('Select an entry to edit.'))
  const d = draft.value
  const next = readPermissionCatalog(catalog.value as unknown as JsonValue)
  const id = d.id.trim()
  if (!id || !d.name.trim()) throw new Error(st('Enter an ID and a name.'))
  if (selectedId.value === null && Object.hasOwn(profiles.value, id)) throw new Error(st('This ID already exists.'))
  const permission = parsePermissionConfig(d.complete ? completePermissionConfig(d.permission) : d.permission)
  const grants = d.mcp.map(
    ({ server, tools }) =>
      [
        server.trim(),
        tools
          .split(/\r?\n/)
          .map((tool) => tool.trim())
          .filter(Boolean),
      ] as const,
  )
  if (
    d.grantEnabled &&
    (new Set(grants.map(([server]) => server)).size !== grants.length || grants.some(([server]) => !server))
  )
    throw new Error(st('Enter a distinct server ID for each MCP grant.'))
  const grant = d.grantEnabled
    ? permissionGrantFromFields(d.hostsText, Object.fromEntries(grants), d.privateMemory, d.sourceWorkspace)
    : undefined
  if (grant === null) throw new Error(st('MCP grants must map server IDs to lists of tool names.'))
  next.profiles = {
    ...next.profiles,
    [id]: {
      name: d.name.trim(),
      description: d.description.trim(),
      revision: (selectedId.value ? profiles.value[selectedId.value]?.revision || 0 : 0) + 1,
      permission,
      ...(d.complete ? { complete: true } : {}),
      ...(grant ? { grant } : {}),
    },
  }
  return { value: next, id }
}

async function persist(next: PermissionCatalog) {
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
}

async function saveChannelAssignments(next: PermissionCatalog) {
  if (saving.value || loading.value) throw new Error(saving.value ? st('Saving changes…') : st('Loading…'))
  saving.value = true
  error.value = ''
  try {
    await persist(next)
  } finally {
    saving.value = false
  }
}

async function save() {
  if (saving.value || !draft.value) return
  if (permissionEditors.value[0]?.applyPendingRule() === false) return
  saving.value = true
  error.value = ''
  saved.value = false
  try {
    const next = updatedCatalog()
    await persist(next.value)
    showEntry(next.id)
    saved.value = true
  } catch (reason) {
    error.value = permissionConfigErrorMessage(reason, st)
  } finally {
    saving.value = false
  }
}

async function removeEntry() {
  if (saving.value || selectedId.value === null) return
  if (Object.hasOwn(factoryProfiles.value, selectedId.value)) return
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
  if (saving.value || selectedId.value !== id) return
  saving.value = true
  error.value = ''
  try {
    const next = readPermissionCatalog(catalog.value as unknown as JsonValue)
    delete next.profiles[id]
    await persist(next)
    showEntry(null)
    saved.value = true
  } catch (reason) {
    error.value = permissionConfigErrorMessage(reason, st)
  } finally {
    saving.value = false
  }
}

onMounted(() => {
  void load()
})
onBeforeUnmount(() => {
  generation += 1
})
defineExpose({ refresh })
</script>

<template>
  <section class="settings-stack min-w-0" :aria-busy="loading || saving" data-permission-profiles>
    <ChannelAssignmentsPanel
      v-if="assignments && loaded"
      ref="channelPanel"
      :catalog="catalog"
      :profiles="profiles"
      :target="channelTarget"
      :target-source="channelSource"
      :disabled="loading"
      :saving="saving"
      :save="saveChannelAssignments"
      :can-discard="canDiscard"
      @refresh="refresh"
    />
    <div v-if="!assignments" class="settings-toolbar rounded-lg border border-border/70 bg-card/30 p-2.5">
      <SearchInput
        v-model="search"
        class="settings-toolbar-search"
        :placeholder="$st('Search configurations')"
        :input-aria-label="$st('Search configurations')"
      />
      <span v-if="saved && !dirty" class="text-xs text-muted-foreground" role="status">{{ $st('Saved') }}</span>
      <IconButton
        size="sm"
        :loading="loading"
        :disabled="saving"
        :aria-label="$st('Refresh')"
        :tooltip="$st('Refresh')"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" aria-hidden="true" />
      </IconButton>
      <Button size="sm" :disabled="loading || saving || !loaded" @click="addEntry()">
        <RiAddLine class="me-1.5 h-4 w-4" aria-hidden="true" />
        {{ $st('New permission profile') }}
      </Button>
    </div>
    <p
      v-if="error && (!dirty || assignments)"
      class="break-words rounded-md border border-destructive/40 p-2.5 text-sm text-destructive"
      role="alert"
    >
      {{ error }}
    </p>
    <p v-if="loading && !loaded" class="text-sm text-muted-foreground" role="status">{{ $st('Loading…') }}</p>
    <div v-else-if="loaded && !assignments" class="grid min-w-0 gap-2">
      <SettingsDisclosureRow
        v-for="row in rows"
        :key="row.key"
        :data-permission-profile="row.id"
        class="min-w-0 bg-card/30"
        body-class="min-w-0 p-2.5"
        :disabled="loading || saving"
        :open="row.open"
        :label="row.title"
        :summary="row.detail"
        :tone="row.open && dirty ? 'dirty' : 'default'"
        @toggle="toggleEntry(row.id)"
      >
        <template #leading>
          <span
            class="hidden h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/30 text-muted-foreground sm:flex"
          >
            <RiShieldCheckLine class="h-4 w-4" aria-hidden="true" />
          </span>
        </template>
        <template v-if="row.open && selectedId !== null" #actions>
          <Button size="sm" variant="ghost" :disabled="saving" @click="addEntry(true)">{{
            $st('Copy profile')
          }}</Button>
          <IconButton
            v-if="!Object.hasOwn(factoryProfiles, row.id)"
            size="sm"
            :disabled="saving || profileUsage.length > 0"
            :aria-label="$st('Delete')"
            :tooltip="$st('Delete')"
            @click="removeEntry"
          >
            <RiDeleteBinLine class="h-4 w-4 text-destructive" aria-hidden="true" />
          </IconButton>
        </template>
        <template v-if="row.open && draft">
          <fieldset :disabled="saving" class="grid min-w-0 gap-3">
            <div class="settings-control-grid">
              <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
                {{ $st('ID') }}
                <input
                  v-model="draft.id"
                  :readonly="selectedId !== null"
                  required
                  class="min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                />
              </label>
              <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
                {{ $st('Name') }}
                <input
                  v-model="draft.name"
                  required
                  class="min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                />
              </label>
            </div>

            <label class="grid gap-1 text-xs text-muted-foreground">
              {{ $st('Description') }}
              <textarea
                v-model="draft.description"
                rows="2"
                class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
              />
            </label>

            <Button
              v-if="!draft.complete"
              type="button"
              size="sm"
              variant="outline"
              class="justify-self-start"
              @click="setComplete(draft)"
            >
              {{ $st('Convert to complete configuration') }}
            </Button>
            <PermissionStudioPanel
              :key="editorKey"
              ref="permissionEditors"
              v-model:draft-permission="draft.permission"
              :standalone-draft="draft.complete"
              :draft-disabled="saving"
            />

            <details class="grid min-w-0 gap-2 rounded-md border border-border/60 p-2.5">
              <summary class="cursor-pointer text-sm font-medium">{{ $st('Channel resource access') }}</summary>
              <label v-if="!draft.complete" class="mt-2 flex items-center gap-2 text-sm">
                <input v-model="draft.grantEnabled" type="checkbox" />{{
                  $st('Set channel resource access for this profile')
                }}
              </label>
              <p v-if="!draft.complete" class="mt-2 text-xs text-muted-foreground">
                {{ $st('Channel resource access is inherited unless this profile specifies it.') }}
              </p>
              <div v-if="draft.grantEnabled" class="mt-2 grid min-w-0 gap-2.5">
                <div v-for="(grant, index) in draft.mcp" :key="index" class="settings-toolbar items-end">
                  <label class="grid min-w-0 flex-1 gap-1 text-xs text-muted-foreground">
                    {{ $st('MCP server ID') }}
                    <input
                      v-model="grant.server"
                      class="min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-xs text-foreground"
                    />
                  </label>
                  <label class="grid min-w-0 flex-1 gap-1 text-xs text-muted-foreground">
                    {{ $st('Allowed MCP tools') }}
                    <textarea
                      v-model="grant.tools"
                      rows="2"
                      :placeholder="$st('One tool name per line.')"
                      class="min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-xs text-foreground"
                    />
                  </label>
                  <Button type="button" size="sm" variant="ghost" @click="draft.mcp.splice(index, 1)">{{
                    $st('Remove')
                  }}</Button>
                </div>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  class="justify-self-start"
                  @click="draft.mcp.push({ server: '', tools: '' })"
                  >{{ $st('Add MCP grant') }}</Button
                >
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
            </details>
          </fieldset>
          <p v-if="profileUsage.length" class="mt-2 text-xs text-muted-foreground">
            {{ $st('Used by {count} bindings. Reassign them before deleting.', { count: profileUsage.length }) }}
          </p>
        </template>
      </SettingsDisclosureRow>
      <p
        v-if="!rows.length && !loading"
        class="rounded-md border border-dashed border-border/60 p-3 text-sm text-muted-foreground"
      >
        {{ $st('No configurations found. Add one to get started.') }}
      </p>
    </div>
    <Button v-else-if="!loaded && !loading" size="sm" variant="outline" @click="load">{{ $st('Retry') }}</Button>
    <SettingsSaveBar
      v-if="!assignments && (dirty || saving)"
      data-permission-profile-save
      :dirty="dirty"
      :saving="saving"
      :disabled="loading"
      :save-disabled="!draft?.id.trim() || !draft.name.trim()"
      :error="error"
      sticky
      @save="save"
      @discard="showEntry(selectedId)"
    />
  </section>
</template>
