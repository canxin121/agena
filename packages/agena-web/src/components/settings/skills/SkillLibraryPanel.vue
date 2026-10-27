<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate, useRoute } from 'vue-router'
import {
  RiAddLine,
  RiArrowLeftLine,
  RiBookOpenLine,
  RiDeleteBinLine,
  RiDownloadCloud2Line,
  RiFileTextLine,
  RiRefreshLine,
} from '@remixicon/vue'
import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import SettingsSaveBar from '../SettingsSaveBar.vue'
import Button from '@/components/ui/Button.vue'
import Dialog from '@/components/ui/Dialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { confirmAction } from '@/lib/appConfirm'
import {
  createSkillsClient,
  type SkillCatalog,
  type SkillDetail,
  type SkillInstallSource,
  type SkillScope,
  type SkillValidation,
} from '@/lib/skillsApi'
import { filterSkills, newSkillDocument, safeSkillResourcePath, skillMarkdownBody, validSkillName } from '@/lib/skills'
import { useDirectoryStore } from '@/stores/directory'
import { useToastsStore } from '@/stores/toasts'
import { resolveWorkspace } from '@/stores/chat/api'

const emit = defineEmits<{
  install: []
  'install-from-source': [value: { source: SkillInstallSource; scope: SkillScope }]
}>()
type Draft = { content: string; saved: string; revision: string }
const route = useRoute()
const directory = useDirectoryStore()
const toasts = useToastsStore()
const catalog = ref<SkillCatalog | null>(null)
const selectedId = ref(typeof route.query.skill === 'string' ? route.query.skill : '')
const detail = ref<SkillDetail | null>(null)
const path = ref('SKILL.md')
const content = ref('')
const saved = ref('')
const revision = ref('')
const query = ref('')
const scopeFilter = ref('all')
const statusFilter = ref('all')
const loading = ref(false)
const detailLoading = ref(false)
const resourceUnavailable = ref(false)
const action = ref('')
const error = ref('')
const editorError = ref('')
const validation = ref<SkillValidation | null>(null)
const preview = ref(false)
const showDetail = ref(false)
const cachedDrafts = ref(new Map<string, Draft>())
const context = computed(() => directory.currentDirectory || '')
const currentKey = computed(() => JSON.stringify([context.value, selectedId.value, path.value]))
const loadedKey = ref('')
let listGeneration = 0
let detailGeneration = 0
let listController: AbortController | undefined
let detailController: AbortController | undefined
let disposed = false
const ready = computed(() => Boolean(detail.value) && loadedKey.value === currentKey.value && !detailLoading.value)
const dirty = computed(() => Boolean(loadedKey.value) && content.value !== saved.value)
const busy = computed(() => Boolean(action.value))
const canEdit = computed(() => ready.value && detail.value?.editable && !busy.value)
const canManage = computed(() => Boolean(detail.value?.editable) && !detailLoading.value && !busy.value)
const canDeleteFile = computed(
  () =>
    canManage.value &&
    !dirty.value &&
    path.value !== 'SKILL.md' &&
    Boolean(detail.value?.files.some((file) => file.path === path.value)),
)
const items = computed(() =>
  filterSkills(catalog.value?.skills || [], query.value, scopeFilter.value, statusFilter.value),
)
const scopeOptions = computed(() => [
  { value: 'all', label: st('All scopes') },
  { value: 'workspace', label: st('Workspace') },
  { value: 'global', label: st('Global') },
])
const statusOptions = computed(() => [
  { value: 'all', label: st('All statuses') },
  { value: 'enabled', label: st('Enabled') },
  { value: 'disabled', label: st('Disabled') },
  { value: 'invalid', label: st('Needs attention') },
  { value: 'shadowed', label: st('Overridden') },
])
const fileOptions = computed(() =>
  (detail.value?.files || []).map((file) => ({
    value: file.path,
    label: file.path,
    description: formatBytes(file.size),
  })),
)
const pendingDrafts = computed(
  () => cachedDrafts.value.size - (cachedDrafts.value.has(loadedKey.value) ? 1 : 0) + (dirty.value ? 1 : 0),
)
const isMarkdown = computed(() => /\.mdx?$/i.test(path.value))
const sourceInstall = computed<SkillInstallSource | null>(() => {
  const source = detail.value?.source
  if (!source) return null
  if (source.kind === 'github')
    return {
      kind: 'github',
      url: source.location,
      ...(source.reference ? { reference: source.reference } : {}),
      ...(source.subpath ? { subpath: source.subpath } : {}),
    }
  if (source.kind === 'folder') return { kind: 'folder', path: source.location }
  if (source.kind === 'archive')
    return /^https?:\/\//i.test(source.location)
      ? { kind: 'archive', url: source.location }
      : { kind: 'archive', path: source.location }
  return null
})
function scopeLabel(scope: string) {
  return scope === 'global' ? st('Global') : st('Workspace')
}
function formatBytes(size: number) {
  return size < 1024
    ? size + ' B'
    : size < 1024 * 1024
      ? (size / 1024).toFixed(1) + ' KiB'
      : (size / 1024 / 1024).toFixed(1) + ' MiB'
}
function message(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason)
}
async function scopedApi(directoryPath: string) {
  return createSkillsClient(directoryPath ? (await resolveWorkspace(directoryPath)).id : undefined)
}

function rememberDraft() {
  if (!loadedKey.value) return
  const next = new Map(cachedDrafts.value)
  if (dirty.value) next.set(loadedKey.value, { content: content.value, saved: saved.value, revision: revision.value })
  else next.delete(loadedKey.value)
  cachedDrafts.value = next
}
function acceptContent(text: string, version: string, key: string) {
  const draft = cachedDrafts.value.get(key)
  saved.value = draft?.saved ?? text
  content.value = draft?.content ?? text
  revision.value = draft?.revision ?? version
  loadedKey.value = key
}
function discardDraft() {
  content.value = saved.value
  const next = new Map(cachedDrafts.value)
  next.delete(loadedKey.value)
  cachedDrafts.value = next
  validation.value = null
  editorError.value = ''
}
async function canSwitch() {
  if (busy.value) return false
  if (!dirty.value) return true
  const confirmed = await confirmAction({
    title: st('Discard unsaved changes?'),
    description: st('You have unsaved changes.'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
  if (confirmed) discardDraft()
  return confirmed
}

async function loadDetail(id: string, resourcePath = 'SKILL.md') {
  rememberDraft()
  selectedId.value = id
  path.value = resourcePath
  const key = currentKey.value
  const requestedContext = context.value
  const generation = ++detailGeneration
  detailController?.abort()
  const controller = new AbortController()
  detailController = controller
  detailLoading.value = true
  resourceUnavailable.value = false
  editorError.value = ''
  validation.value = null
  let fetched: SkillDetail | null = null
  try {
    const api = await scopedApi(requestedContext)
    const next = await api.readSkill(id, controller.signal)
    fetched = next
    const resource =
      resourcePath === 'SKILL.md'
        ? { content: next.document, revision: next.revision }
        : await api.readSkillResource(id, resourcePath, controller.signal)
    if (disposed || generation !== detailGeneration || key !== currentKey.value) return
    detail.value = next
    acceptContent(resource.content, resource.revision, key)
  } catch (reason) {
    if (disposed || generation !== detailGeneration || key !== currentKey.value || controller.signal.aborted) return
    editorError.value = message(reason)
    detail.value = fetched
    resourceUnavailable.value = Boolean(fetched)
    content.value = ''
    saved.value = ''
    loadedKey.value = ''
  } finally {
    if (generation === detailGeneration) detailLoading.value = false
  }
}
async function selectSkill(id: string) {
  if (id !== selectedId.value && !(await canSwitch())) return
  showDetail.value = true
  await loadDetail(id)
}
async function selectResource(nextPath: string) {
  if (nextPath === path.value || !(await canSwitch())) return
  await loadDetail(selectedId.value, nextPath)
}
async function refresh() {
  if (busy.value) return
  rememberDraft()
  const generation = ++listGeneration
  const requestedContext = context.value
  listController?.abort()
  const controller = new AbortController()
  listController = controller
  loading.value = true
  error.value = ''
  try {
    const api = await scopedApi(requestedContext)
    const next = await api.listSkills(controller.signal)
    if (disposed || generation !== listGeneration || requestedContext !== context.value) return
    catalog.value = next
    const requested = selectedId.value
    const id = next.skills.some((skill) => skill.id === requested) ? requested : next.skills[0]?.id
    if (id) {
      const same = id === selectedId.value
      await loadDetail(
        id,
        same && detail.value?.files.some((file) => file.path === path.value) ? path.value : 'SKILL.md',
      )
      if (route.query.skill === id) showDetail.value = true
    } else {
      selectedId.value = ''
      detail.value = null
      loadedKey.value = ''
      content.value = ''
      saved.value = ''
      showDetail.value = false
    }
  } catch (reason) {
    if (!disposed && generation === listGeneration && !controller.signal.aborted) error.value = message(reason)
  } finally {
    if (generation === listGeneration) loading.value = false
  }
}
async function runAction(key: string, execute: () => Promise<void>) {
  if (busy.value) return false
  action.value = key
  editorError.value = ''
  try {
    await execute()
    return true
  } catch (reason) {
    if (!disposed) editorError.value = message(reason)
    return false
  } finally {
    action.value = ''
  }
}
async function save() {
  if (!canEdit.value || !dirty.value) return
  const id = selectedId.value
  const filePath = path.value
  const text = content.value
  const version = revision.value
  const key = currentKey.value
  const requestedContext = context.value
  const succeeded = await runAction('save', async () => {
    const api = await scopedApi(requestedContext)
    const updated =
      filePath === 'SKILL.md'
        ? await api.updateSkill(id, text, version)
        : await api.writeSkillResource(id, filePath, text, version)
    const nextDrafts = new Map(cachedDrafts.value)
    nextDrafts.delete(key)
    cachedDrafts.value = nextDrafts
    // A workspace switch can leave the completed save's editor mounted until
    // refresh. Retire that old draft before rememberDraft can cache it again.
    if (loadedKey.value === key) {
      saved.value = text
      content.value = text
      revision.value = updated.revision
      if (key !== currentKey.value) loadedKey.value = ''
    }
    if (key === currentKey.value) await loadDetail(id, filePath)
    toasts.push('success', st('Skill saved'))
  })
  if (succeeded) await refresh()
}
async function discardAndReload() {
  if (!ready.value || busy.value) return
  const id = selectedId.value
  const filePath = path.value
  const key = currentKey.value
  const requestedContext = context.value
  await runAction('reload', async () => {
    const api = await scopedApi(requestedContext)
    const next = await api.readSkill(id)
    const resource =
      filePath === 'SKILL.md'
        ? { content: next.document, revision: next.revision }
        : await api.readSkillResource(id, filePath)
    if (key !== currentKey.value) return
    const drafts = new Map(cachedDrafts.value)
    drafts.delete(key)
    cachedDrafts.value = drafts
    detail.value = next
    acceptContent(resource.content, resource.revision, key)
    validation.value = null
  })
}
async function validateDocument() {
  if (!ready.value || path.value !== 'SKILL.md') return
  const requestedContext = context.value
  const key = currentKey.value
  const document = content.value
  await runAction('validate', async () => {
    const api = await scopedApi(requestedContext)
    const result = await api.validateSkill(document)
    if (key === currentKey.value) validation.value = result
  })
}
async function toggleEnabled() {
  if (!detail.value || dirty.value || !canManage.value) return
  const current = detail.value
  const requestedContext = context.value
  const succeeded = await runAction('toggle', async () => {
    const api = await scopedApi(requestedContext)
    await api.setSkillEnabled(current.id, !current.enabled, current.revision)
    if (requestedContext === context.value) await loadDetail(current.id, path.value)
  })
  if (succeeded) await refresh()
}
async function deleteSelected() {
  const current = detail.value
  if (!current || !canManage.value || dirty.value) return
  const requestedContext = context.value
  if (
    !(await confirmAction({
      title: st('Delete skill?'),
      description: st('Delete {name} and all files in its skill folder.', { name: current.name }),
      confirmText: st('Delete'),
      cancelText: st('Cancel'),
    }))
  )
    return
  const succeeded = await runAction('delete', async () => {
    const api = await scopedApi(requestedContext)
    await api.removeSkill(current.id, current.revision)
    if (requestedContext === context.value && selectedId.value === current.id) {
      detail.value = null
      selectedId.value = ''
      loadedKey.value = ''
    }
    toasts.push('success', st('Skill deleted'))
  })
  if (succeeded) await refresh()
}
async function deleteResource() {
  if (!detail.value || !canDeleteFile.value) return
  const requestedContext = context.value
  const id = selectedId.value
  const filePath = path.value
  const version = ready.value ? revision.value : detail.value.revision
  if (
    !(await confirmAction({
      title: st('Delete file?'),
      description: filePath,
      confirmText: st('Delete'),
      cancelText: st('Cancel'),
    }))
  )
    return
  const succeeded = await runAction('delete-file', async () => {
    const api = await scopedApi(requestedContext)
    await api.removeSkillResource(id, filePath, version)
    if (requestedContext === context.value) await loadDetail(id)
  })
  if (succeeded && requestedContext !== context.value) await refresh()
}

const createFormId = useId()
const createOpen = ref(false)
const createName = ref('')
const createDescription = ref('')
const createInstructions = ref('')
const createScope = ref<SkillScope>('workspace')
const createError = ref('')
const createValid = computed(
  () =>
    validSkillName(createName.value) &&
    Boolean(createDescription.value.trim()) &&
    Array.from(createDescription.value).length <= 1024,
)
async function openCreate() {
  if (!(await canSwitch())) return
  createError.value = ''
  createOpen.value = true
}
async function submitCreate() {
  if (!createValid.value || busy.value) return
  action.value = 'create'
  const requestedContext = context.value
  createError.value = ''
  try {
    const document = newSkillDocument(createName.value, createDescription.value, createInstructions.value)
    const api = await scopedApi(requestedContext)
    const result = await api.createSkill(createScope.value, document)
    createOpen.value = false
    createName.value = ''
    createDescription.value = ''
    createInstructions.value = ''
    if (requestedContext === context.value) {
      selectedId.value = result.id
      showDetail.value = true
      detail.value = result
      await loadDetail(result.id)
    }
    toasts.push('success', st('Skill created'))
  } catch (reason) {
    createError.value = message(reason)
  } finally {
    action.value = ''
  }
  if (!createOpen.value) await refresh()
}
async function closeCreate() {
  if (busy.value) return
  if (
    (createName.value || createDescription.value || createInstructions.value) &&
    !(await confirmAction({
      title: st('Discard unsaved changes?'),
      description: st('You have unsaved changes.'),
      confirmText: st('Discard changes'),
      cancelText: st('Keep editing'),
    }))
  )
    return
  createOpen.value = false
  createName.value = ''
  createDescription.value = ''
  createInstructions.value = ''
}

const fileFormId = useId()
const fileOpen = ref(false)
const newFilePath = ref('')
const newFileContent = ref('')
const fileError = ref('')
const newFileValid = computed(
  () =>
    safeSkillResourcePath(newFilePath.value) &&
    newFilePath.value !== 'SKILL.md' &&
    !detail.value?.files.some((file) => file.path === newFilePath.value),
)
async function openNewFile() {
  if (!canManage.value || !(await canSwitch())) return
  newFilePath.value = ''
  newFileContent.value = ''
  fileError.value = ''
  fileOpen.value = true
}
async function submitFile() {
  if (!newFileValid.value || busy.value) return
  action.value = 'create-file'
  const requestedContext = context.value
  fileError.value = ''
  try {
    const id = selectedId.value
    const filePath = newFilePath.value
    const text = newFileContent.value
    const api = await scopedApi(requestedContext)
    await api.writeSkillResource(id, filePath, text)
    fileOpen.value = false
    if (requestedContext === context.value) await loadDetail(selectedId.value, newFilePath.value)
  } catch (reason) {
    fileError.value = message(reason)
  } finally {
    action.value = ''
  }
}
async function closeFile() {
  if (busy.value) return
  if (
    (newFilePath.value || newFileContent.value) &&
    !(await confirmAction({
      title: st('Discard unsaved changes?'),
      description: st('You have unsaved changes.'),
      confirmText: st('Discard changes'),
      cancelText: st('Keep editing'),
    }))
  )
    return
  fileOpen.value = false
  newFilePath.value = ''
  newFileContent.value = ''
}

const copyFormId = useId()
const copyOpen = ref(false)
const copyDocument = ref('')
const copyOriginal = ref('')
const copyScope = ref<SkillScope>('workspace')
const copyError = ref('')
async function copySkill() {
  const current = detail.value
  if (!current || busy.value || !(await canSwitch())) return
  if (!current.path.startsWith('plugin://')) {
    emit('install-from-source', {
      source: { kind: 'folder', path: current.path.replace(/[\\/]SKILL\.md$/, '') },
      scope: current.scope,
    })
    return
  }
  copyDocument.value = current.document
  copyOriginal.value = current.document
  copyScope.value = current.scope
  copyError.value = ''
  copyOpen.value = true
}
async function submitCopy() {
  if (busy.value || !copyDocument.value.trim()) return
  const requestedContext = context.value
  const document = copyDocument.value
  const destination = copyScope.value
  action.value = 'copy'
  copyError.value = ''
  try {
    const api = await scopedApi(requestedContext)
    const result = await api.createSkill(destination, document)
    copyOpen.value = false
    if (requestedContext === context.value) {
      selectedId.value = result.id
      showDetail.value = true
      await loadDetail(result.id)
    }
    toasts.push('success', st('Skill created'))
  } catch (reason) {
    copyError.value = message(reason)
  } finally {
    action.value = ''
  }
  if (!copyOpen.value) await refresh()
}
async function closeCopy() {
  if (busy.value) return
  if (
    copyDocument.value !== copyOriginal.value &&
    !(await confirmAction({
      title: st('Discard unsaved changes?'),
      description: st('You have unsaved changes.'),
      confirmText: st('Discard changes'),
      cancelText: st('Keep editing'),
    }))
  )
    return
  copyOpen.value = false
}

async function canLeave() {
  if (busy.value) return false
  rememberDraft()
  if (!cachedDrafts.value.size && !createOpen.value && !fileOpen.value && !copyOpen.value) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    description: st('You have unsaved changes.'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}
onBeforeRouteLeave(canLeave)
onBeforeRouteUpdate(async (to, from) => {
  if (to.query.view === from.query.view && to.query.skill === from.query.skill) return true
  if (!(await canLeave())) return false
  cachedDrafts.value = new Map()
  discardDraft()
  return true
})
function beforeUnload(event: BeforeUnloadEvent) {
  if (!dirty.value && !cachedDrafts.value.size && !createOpen.value && !fileOpen.value && !copyOpen.value) return
  event.preventDefault()
  event.returnValue = ''
}
function keydown(event: KeyboardEvent) {
  if (
    (event.ctrlKey || event.metaKey) &&
    event.key.toLowerCase() === 's' &&
    dirty.value &&
    !createOpen.value &&
    !fileOpen.value &&
    !copyOpen.value
  ) {
    event.preventDefault()
    void save()
  }
}
watch(context, () => {
  rememberDraft()
  void refresh()
})
watch(
  () => route.query.skill,
  (id) => {
    if (typeof id === 'string' && id !== selectedId.value && catalog.value?.skills.some((skill) => skill.id === id))
      void selectSkill(id)
  },
)
onMounted(() => {
  void refresh()
  window.addEventListener('beforeunload', beforeUnload)
  window.addEventListener('keydown', keydown)
})
onBeforeUnmount(() => {
  disposed = true
  listController?.abort()
  detailController?.abort()
  window.removeEventListener('beforeunload', beforeUnload)
  window.removeEventListener('keydown', keydown)
})
defineExpose({ refresh })
</script>

<template>
  <div class="flex min-w-0 flex-col gap-4 lg:h-full lg:min-h-0">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex min-w-0 flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span class="rounded-full border border-border/60 bg-muted/30 px-2.5 py-1">{{
          $st('{count} skills', { count: catalog?.skills.length || 0 })
        }}</span>
        <span v-if="pendingDrafts" class="text-amber-700 dark:text-amber-300">{{
          $st('Unsaved files: {count}', { count: pendingDrafts })
        }}</span>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <IconButton :title="$st('Refresh')" :aria-label="$st('Refresh')" :disabled="loading || busy" @click="refresh">
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
        <Button size="sm" variant="outline" :disabled="busy" @click="emit('install')">
          <RiDownloadCloud2Line class="mr-1.5 h-4 w-4" />{{ $st('Install skills') }}
        </Button>
        <Button size="sm" :disabled="busy" @click="openCreate">
          <RiAddLine class="mr-1.5 h-4 w-4" />{{ $st('Create skill') }}
        </Button>
      </div>
    </div>
    <p
      v-if="error"
      role="alert"
      class="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
    >
      {{ error }}
    </p>
    <details
      v-if="catalog?.diagnostics.length"
      class="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs"
    >
      <summary class="cursor-pointer font-medium">
        {{ $st('Discovery issues: {count}', { count: catalog.diagnostics.length }) }}
      </summary>
      <ul class="mt-2 space-y-2">
        <li v-for="(issue, index) in catalog.diagnostics" :key="index" class="break-words">
          <span class="font-mono">{{ issue.path }}</span>
          <p class="mt-0.5 text-muted-foreground">{{ issue.message }}</p>
        </li>
      </ul>
    </details>
    <div class="grid min-w-0 gap-4 lg:min-h-0 lg:flex-1 lg:grid-cols-[19rem_minmax(0,1fr)]">
      <aside
        class="min-w-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-background lg:flex lg:min-h-0"
        :class="showDetail ? 'hidden' : 'flex'"
      >
        <div class="space-y-3 border-b border-border/60 p-3">
          <SearchInput v-model="query" :placeholder="$st('Search skills')" :aria-label="$st('Search skills')" />
          <div class="grid grid-cols-2 gap-2">
            <OptionPicker
              v-model="scopeFilter"
              :options="scopeOptions"
              :include-empty="false"
              :title="$st('Scope')"
              size="sm"
            />
            <OptionPicker
              v-model="statusFilter"
              :options="statusOptions"
              :include-empty="false"
              :title="$st('Status')"
              size="sm"
            />
          </div>
        </div>
        <div class="max-h-[28rem] min-h-0 flex-1 overflow-y-auto p-1.5 lg:max-h-none" :aria-busy="loading">
          <div v-if="loading && !catalog" role="status" class="px-4 py-10 text-center text-sm text-muted-foreground">
            {{ $st('Loading…') }}
          </div>
          <div v-else-if="!items.length" class="px-4 py-10 text-center">
            <RiBookOpenLine class="mx-auto mb-3 h-8 w-8 text-muted-foreground/50" />
            <p class="text-sm font-medium">
              {{ catalog?.skills.length ? $st('No matching skills') : $st('No skills installed') }}
            </p>
            <p class="mt-2 text-xs leading-relaxed text-muted-foreground">
              {{ $st('Create a skill or install one from GitHub, an archive, or a folder.') }}
            </p>
          </div>
          <button
            v-for="skill in items"
            :key="skill.id"
            type="button"
            class="mb-1 w-full rounded-lg p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :class="selectedId === skill.id ? 'bg-primary/10 ring-1 ring-inset ring-primary/20' : 'hover:bg-muted/50'"
            :aria-current="selectedId === skill.id ? 'true' : undefined"
            :disabled="busy"
            @click="selectSkill(skill.id)"
          >
            <div class="flex items-start justify-between gap-2">
              <span class="min-w-0 break-words text-sm font-medium">{{ skill.name }}</span>
              <span
                class="mt-1.5 h-2 w-2 shrink-0 rounded-full"
                :class="
                  !skill.valid
                    ? 'bg-amber-500'
                    : skill.enabled && !skill.shadowed
                      ? 'bg-emerald-500'
                      : 'bg-muted-foreground/40'
                "
              />
            </div>
            <p class="mt-1 line-clamp-2 text-xs leading-relaxed text-muted-foreground">
              {{ skill.description || $st('No description') }}
            </p>
            <div class="mt-2 flex flex-wrap gap-1.5 text-[10px] text-muted-foreground">
              <span class="rounded border border-border/60 px-1.5 py-0.5">{{ scopeLabel(skill.scope) }}</span>
              <span
                v-if="!skill.valid"
                class="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-800 dark:text-amber-200"
                >{{ $st('Needs attention') }}</span
              >
              <span v-else-if="skill.shadowed" class="rounded bg-muted px-1.5 py-0.5">{{ $st('Overridden') }}</span>
              <span v-else-if="!skill.enabled" class="rounded bg-muted px-1.5 py-0.5">{{ $st('Disabled') }}</span>
              <span v-if="!skill.editable" class="rounded bg-muted px-1.5 py-0.5">{{ $st('Read only') }}</span>
            </div>
          </button>
        </div>
        <details v-if="catalog?.roots.length" class="shrink-0 border-t border-border/60 px-3 py-2 text-xs">
          <summary class="cursor-pointer text-muted-foreground">{{ $st('Skill locations') }}</summary>
          <ul class="max-h-40 space-y-2 overflow-auto py-2">
            <li v-for="root in catalog.roots" :key="root.path" class="break-all">
              <span class="font-medium">{{ scopeLabel(root.scope) }}</span>
              <p class="mt-0.5 font-mono text-[10px] text-muted-foreground">{{ root.path }}</p>
            </li>
          </ul>
        </details>
      </aside>

      <section
        class="min-w-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-background lg:flex lg:min-h-0"
        :class="showDetail ? 'flex' : 'hidden'"
      >
        <div class="flex items-center justify-between gap-2 border-b border-border/60 p-3 lg:hidden">
          <Button variant="ghost" size="sm" @click="showDetail = false"
            ><RiArrowLeftLine class="mr-1 h-4 w-4" />{{ $st('Skills') }}</Button
          >
        </div>
        <div
          v-if="detailLoading"
          role="status"
          class="grid min-h-64 flex-1 place-content-center text-sm text-muted-foreground"
        >
          {{ $st('Loading…') }}
        </div>
        <div v-else-if="!detail" class="grid min-h-64 flex-1 place-content-center px-6 text-center">
          <RiBookOpenLine class="mx-auto mb-3 h-9 w-9 text-muted-foreground/40" />
          <p class="text-sm text-muted-foreground">
            {{ editorError || $st('Select a skill to inspect its instructions and files.') }}
          </p>
        </div>
        <template v-else>
          <header class="shrink-0 space-y-3 border-b border-border/60 p-4">
            <div class="flex flex-wrap items-start justify-between gap-3">
              <div class="min-w-0">
                <h2 class="break-words text-base font-semibold">{{ detail.name }}</h2>
                <p class="mt-1 max-w-2xl text-xs leading-relaxed text-muted-foreground">{{ detail.description }}</p>
              </div>
              <div class="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  :disabled="!canManage || dirty || (!detail.valid && !detail.enabled)"
                  @click="toggleEnabled"
                >
                  {{ detail.enabled ? $st('Disable') : $st('Enable') }}
                </Button>
                <Button v-if="!detail.editable" size="sm" variant="outline" :disabled="busy" @click="copySkill">{{
                  $st('Create editable copy')
                }}</Button>
                <Button
                  v-if="sourceInstall"
                  size="sm"
                  variant="outline"
                  :disabled="busy || dirty"
                  @click="emit('install-from-source', { source: sourceInstall, scope: detail.scope })"
                  >{{ $st('Update from source') }}</Button
                >
                <IconButton
                  :title="$st('Delete skill')"
                  :aria-label="$st('Delete skill')"
                  :disabled="!canManage || dirty"
                  @click="deleteSelected"
                >
                  <RiDeleteBinLine class="h-4 w-4 text-destructive" />
                </IconButton>
              </div>
            </div>
            <details class="text-xs">
              <summary class="cursor-pointer text-muted-foreground">
                {{ $st('Details and source') }} · {{ scopeLabel(detail.scope) }}
              </summary>
              <dl class="mt-3 grid max-h-48 gap-3 overflow-auto sm:grid-cols-2">
                <div class="min-w-0 sm:col-span-2">
                  <dt class="text-muted-foreground">{{ $st('Path') }}</dt>
                  <dd class="mt-1 break-all font-mono">{{ detail.path }}</dd>
                </div>
                <div v-if="detail.source" class="min-w-0 sm:col-span-2">
                  <dt class="text-muted-foreground">{{ $st('Source') }}</dt>
                  <dd class="mt-1 break-all">{{ detail.source.location }}</dd>
                  <dd
                    v-if="detail.source.reference || detail.source.subpath"
                    class="mt-1 break-all text-muted-foreground"
                  >
                    {{ [detail.source.reference, detail.source.subpath].filter(Boolean).join(' · ') }}
                  </dd>
                  <dd v-if="detail.source.revision" class="mt-1 break-all font-mono text-[10px] text-muted-foreground">
                    {{ detail.source.revision }}
                  </dd>
                </div>
                <div v-if="detail.metadata?.license">
                  <dt class="text-muted-foreground">{{ $st('License') }}</dt>
                  <dd class="mt-1">{{ detail.metadata.license }}</dd>
                </div>
                <div v-if="detail.metadata?.compatibility">
                  <dt class="text-muted-foreground">{{ $st('Compatibility') }}</dt>
                  <dd class="mt-1">{{ detail.metadata.compatibility }}</dd>
                </div>
                <div v-if="detail.metadata?.allowed_tools" class="sm:col-span-2">
                  <dt class="text-muted-foreground">{{ $st('Allowed tools') }}</dt>
                  <dd class="mt-1 break-words font-mono">{{ detail.metadata.allowed_tools }}</dd>
                </div>
                <div v-for="(value, key) in detail.metadata?.metadata || {}" :key="key">
                  <dt class="text-muted-foreground">{{ key }}</dt>
                  <dd class="mt-1 break-words">{{ value }}</dd>
                </div>
              </dl>
            </details>
            <p v-if="detail.shadowed" class="rounded-md bg-muted/50 p-2 text-xs text-muted-foreground">
              {{ $st('A higher-priority skill with the same name is active.') }}
            </p>
            <ul
              v-if="detail.diagnostics.length"
              class="max-h-28 space-y-1 overflow-auto rounded-md border border-amber-500/25 bg-amber-500/5 p-2 text-xs text-amber-900 dark:text-amber-200"
            >
              <li v-for="(issue, index) in detail.diagnostics" :key="index">{{ issue }}</li>
            </ul>
          </header>
          <div
            class="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 bg-muted/10 px-3 py-2"
          >
            <div class="flex min-w-0 flex-1 items-center gap-1.5">
              <RiFileTextLine class="h-4 w-4 shrink-0 text-muted-foreground" />
              <OptionPicker
                :model-value="path"
                :options="fileOptions"
                :include-empty="false"
                :title="$st('Skill files')"
                :disabled="busy"
                monospace
                size="sm"
                trigger-class="max-w-full"
                @update:model-value="selectResource"
              />
              <IconButton
                :title="$st('Add file')"
                :aria-label="$st('Add file')"
                :disabled="!canManage"
                @click="openNewFile"
                ><RiAddLine class="h-4 w-4"
              /></IconButton>
              <IconButton
                v-if="path !== 'SKILL.md'"
                :title="$st('Delete file')"
                :aria-label="$st('Delete file')"
                :disabled="!canDeleteFile"
                @click="deleteResource"
                ><RiDeleteBinLine class="h-4 w-4"
              /></IconButton>
            </div>
            <div class="flex items-center gap-1">
              <Button v-if="path === 'SKILL.md'" size="sm" variant="ghost" :disabled="busy" @click="validateDocument">{{
                $st('Validate')
              }}</Button>
              <Button
                size="sm"
                :variant="preview ? 'ghost' : 'secondary'"
                :aria-pressed="!preview"
                @click="preview = false"
                >{{ $st('Edit') }}</Button
              >
              <Button
                size="sm"
                :variant="preview ? 'secondary' : 'ghost'"
                :aria-pressed="preview"
                @click="preview = true"
                >{{ $st('Preview') }}</Button
              >
            </div>
          </div>
          <div v-if="validation" class="shrink-0 space-y-1 border-b border-border/60 px-4 py-2 text-xs" role="status">
            <p :class="validation.valid ? 'text-emerald-700 dark:text-emerald-300' : 'text-destructive'">
              {{ validation.valid ? $st('Skill validation passed') : $st('Skill validation failed') }}
            </p>
            <p v-for="issue in validation.errors" :key="issue" class="text-destructive">{{ issue }}</p>
            <p v-for="issue in validation.warnings" :key="issue" class="text-amber-800 dark:text-amber-200">
              {{ issue }}
            </p>
          </div>
          <div
            v-if="resourceUnavailable"
            class="grid min-h-64 flex-1 place-content-center p-4 text-sm text-muted-foreground"
          >
            {{ $st('This file cannot be opened as text. Choose another skill file.') }}
          </div>
          <div v-else-if="preview" class="min-h-80 flex-1 overflow-auto p-4 lg:min-h-0">
            <MarkdownRenderer
              v-if="isMarkdown"
              :content="path === 'SKILL.md' ? skillMarkdownBody(content) : content"
              :stream="false"
              :allow-apps="false"
              source-path=""
            />
            <pre v-else class="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed">{{ content }}</pre>
          </div>
          <textarea
            v-else
            v-model="content"
            :aria-label="$st('Skill file contents')"
            :readonly="!detail.editable"
            :disabled="busy"
            spellcheck="false"
            class="min-h-80 w-full flex-1 resize-y bg-transparent p-4 font-mono text-xs leading-6 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:opacity-60 lg:min-h-0 lg:resize-none"
          />
          <div class="shrink-0 space-y-2 border-t border-border/60 p-3">
            <p v-if="editorError" role="alert" class="whitespace-pre-wrap break-words text-xs text-destructive">
              {{ editorError }}
            </p>
            <SettingsSaveBar
              v-if="detail.editable && ready"
              :dirty="dirty"
              :saving="action === 'save'"
              :disabled="busy && action !== 'save'"
              @save="save"
              @discard="discardAndReload"
            />
            <p v-else-if="!detail.editable" class="text-xs text-muted-foreground">
              {{ $st('This skill is read only. Install a copy into Global or Workspace to edit it.') }}
            </p>
          </div>
        </template>
      </section>
    </div>

    <Dialog
      :open="createOpen"
      :title="$st('Create skill')"
      :description="$st('Create a standard SKILL.md package with reusable instructions.')"
      max-width="max-w-2xl"
      @close="closeCreate"
    >
      <form :id="createFormId" class="space-y-4" @submit.prevent="submitCreate">
        <div class="grid gap-4 sm:grid-cols-[1fr_12rem]">
          <label class="space-y-1.5 text-xs font-medium"
            >{{ $st('Name') }}
            <input
              v-model="createName"
              required
              :disabled="busy"
              class="block w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            />
            <span class="block text-[11px] font-normal text-muted-foreground">{{
              $st('Use lowercase letters, numbers, and single hyphens (1–64 characters).')
            }}</span>
          </label>
          <div class="space-y-1.5 text-xs font-medium">
            <span>{{ $st('Install scope') }}</span>
            <OptionPicker
              :model-value="createScope"
              :options="scopeOptions.filter((option) => option.value !== 'all')"
              :include-empty="false"
              :disabled="busy"
              :title="$st('Install scope')"
              @update:model-value="createScope = $event as SkillScope"
            />
          </div>
        </div>
        <label class="block space-y-1.5 text-xs font-medium"
          >{{ $st('Description') }}
          <textarea
            v-model="createDescription"
            required
            :disabled="busy"
            rows="2"
            class="block w-full resize-y rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          <span class="block text-[11px] font-normal text-muted-foreground">{{
            $st('Explain what the skill does and when the agent should use it.')
          }}</span>
        </label>
        <label class="block space-y-1.5 text-xs font-medium"
          >{{ $st('Instructions') }}
          <textarea
            v-model="createInstructions"
            :disabled="busy"
            rows="10"
            spellcheck="false"
            class="block w-full resize-y rounded-md border border-input bg-background px-3 py-2 font-mono text-xs leading-6 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
        </label>
        <p v-if="createError" role="alert" class="whitespace-pre-wrap text-xs text-destructive">{{ createError }}</p>
      </form>
      <template #footer
        ><Button variant="ghost" :disabled="busy" @click="closeCreate">{{ $st('Cancel') }}</Button
        ><Button :form="createFormId" type="submit" class="ml-auto" :disabled="!createValid || busy">{{
          busy ? $st('Saving changes…') : $st('Create skill')
        }}</Button></template
      >
    </Dialog>
    <Dialog
      :open="copyOpen"
      :title="$st('Create editable copy')"
      :description="
        $st(
          'Choose a valid name in the frontmatter. This copies SKILL.md instructions; install a folder or archive to copy supporting files.',
        )
      "
      max-width="max-w-2xl"
      @close="closeCopy"
    >
      <form :id="copyFormId" class="space-y-4" @submit.prevent="submitCopy">
        <OptionPicker
          :model-value="copyScope"
          :options="scopeOptions.filter((option) => option.value !== 'all')"
          :include-empty="false"
          :disabled="busy"
          :title="$st('Install scope')"
          @update:model-value="copyScope = $event as SkillScope"
        />
        <textarea
          v-model="copyDocument"
          :aria-label="$st('Skill file contents')"
          :disabled="busy"
          rows="16"
          spellcheck="false"
          class="block w-full resize-y rounded-md border border-input bg-background px-3 py-2 font-mono text-xs leading-6 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        <p v-if="copyError" role="alert" class="whitespace-pre-wrap text-xs text-destructive">{{ copyError }}</p>
      </form>
      <template #footer
        ><Button variant="ghost" :disabled="busy" @click="closeCopy">{{ $st('Cancel') }}</Button
        ><Button :form="copyFormId" type="submit" class="ml-auto" :disabled="!copyDocument.trim() || busy">{{
          $st('Create editable copy')
        }}</Button></template
      >
    </Dialog>
    <Dialog :open="fileOpen" :title="$st('Add file')" max-width="max-w-2xl" @close="closeFile">
      <form :id="fileFormId" class="space-y-4" @submit.prevent="submitFile">
        <label class="block space-y-1.5 text-xs font-medium"
          >{{ $st('Relative file path') }}
          <input
            v-model="newFilePath"
            required
            :disabled="busy"
            class="block w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
        </label>
        <label class="block space-y-1.5 text-xs font-medium"
          >{{ $st('File contents') }}
          <textarea
            v-model="newFileContent"
            :disabled="busy"
            rows="12"
            spellcheck="false"
            class="block w-full resize-y rounded-md border border-input bg-background px-3 py-2 font-mono text-xs leading-6 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
        </label>
        <p v-if="fileError" role="alert" class="whitespace-pre-wrap text-xs text-destructive">{{ fileError }}</p>
      </form>
      <template #footer
        ><Button variant="ghost" :disabled="busy" @click="closeFile">{{ $st('Cancel') }}</Button
        ><Button :form="fileFormId" type="submit" class="ml-auto" :disabled="!newFileValid || busy">{{
          $st('Add file')
        }}</Button></template
      >
    </Dialog>
  </div>
</template>
