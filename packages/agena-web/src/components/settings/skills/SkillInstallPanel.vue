<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import { RiArchiveLine, RiFolderLine, RiGithubLine, RiUploadCloud2Line } from '@remixicon/vue'
import SettingsCard from '../SettingsCard.vue'
import Button from '@/components/ui/Button.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { buildSelectedSkillInstall, skillUploadProblem, skillUploadSource } from '@/lib/skills'
import {
  createSkillsClient,
  type SkillInstallPreview,
  type SkillInstallRequest,
  type SkillInstallResult,
  type SkillInstallSource,
  type SkillScope,
  type SkillsClient,
} from '@/lib/skillsApi'
import { useDirectoryStore } from '@/stores/directory'
import { resolveWorkspace } from '@/stores/chat/api'
import { useToastsStore } from '@/stores/toasts'

const props = defineProps<{ initialSource?: { source: SkillInstallSource; scope: SkillScope } | null }>()
const emit = defineEmits<{ installed: [id: string] }>()
const directory = useDirectoryStore()
const toasts = useToastsStore()
type SourceKind = 'github' | 'archive' | 'folder' | 'upload-folder'
const kind = ref<SourceKind>('github')
const scope = ref<SkillScope>('workspace')
const githubUrl = ref('')
const reference = ref('')
const subpath = ref('')
const archiveMode = ref('upload')
const archiveLocation = ref('')
const folderPath = ref('')
const archiveFiles = ref<File[]>([])
const folderFiles = ref<File[]>([])
const phase = ref<'preview' | 'install' | ''>('')
const error = ref('')
const preview = ref<SkillInstallPreview | null>(null)
const snapshot = ref<Pick<SkillInstallRequest, 'scope' | 'source'> | null>(null)
const previewClient = ref<SkillsClient | null>(null)
const selected = ref<string[]>([])
const replace = ref(false)
const result = ref<SkillInstallResult | null>(null)
let generation = 0
let controller: AbortController | undefined
let disposed = false
const busy = computed(() => Boolean(phase.value))
const sourceKinds = computed(() => [
  { value: 'github' as const, label: st('GitHub'), description: st('Repository or skill URL'), icon: RiGithubLine },
  { value: 'archive' as const, label: st('Archive'), description: st('ZIP, TAR, or TAR.GZ'), icon: RiArchiveLine },
  {
    value: 'folder' as const,
    label: st('Server folder'),
    description: st('Folder on the Agena server'),
    icon: RiFolderLine,
  },
  {
    value: 'upload-folder' as const,
    label: st('Upload folder'),
    description: st('Folder from this device'),
    icon: RiUploadCloud2Line,
  },
])
const scopeOptions = computed(() => [
  { value: 'workspace', label: st('Workspace'), description: st('Available in this project') },
  { value: 'global', label: st('Global'), description: st('Available in every project') },
])
const archiveOptions = computed(() => [
  { value: 'upload', label: st('Upload file') },
  { value: 'url', label: st('Download URL') },
  { value: 'path', label: st('Server path') },
])
const canPreview = computed(
  () =>
    !busy.value &&
    (kind.value === 'github'
      ? Boolean(githubUrl.value.trim())
      : kind.value === 'folder'
        ? Boolean(folderPath.value.trim())
        : kind.value === 'upload-folder'
          ? folderFiles.value.length > 0
          : archiveMode.value === 'upload'
            ? archiveFiles.value.length === 1
            : Boolean(archiveLocation.value.trim())),
)
const installRequest = computed(() =>
  snapshot.value && preview.value && previewClient.value
    ? buildSelectedSkillInstall(snapshot.value, preview.value, selected.value, replace.value)
    : null,
)
const validCandidates = computed(() => (preview.value?.candidates || []).filter((candidate) => candidate.valid))
const conflictCount = computed(() => validCandidates.value.filter((candidate) => candidate.conflict).length)
const uploadSummary = computed(() => {
  const files = kind.value === 'upload-folder' ? folderFiles.value : archiveFiles.value
  return { count: files.length, mib: (files.reduce((size, file) => size + file.size, 0) / 1024 / 1024).toFixed(1) }
})

function message(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason)
}
function resetPreview() {
  generation += 1
  controller?.abort()
  preview.value = null
  snapshot.value = null
  previewClient.value = null
  selected.value = []
  result.value = null
  error.value = ''
}
watch(
  [
    kind,
    scope,
    githubUrl,
    reference,
    subpath,
    archiveMode,
    archiveLocation,
    folderPath,
    archiveFiles,
    folderFiles,
    () => directory.currentDirectory,
  ],
  resetPreview,
)
watch(replace, (value) => {
  if (!value && preview.value) {
    const allowed = new Set(
      preview.value.candidates
        .filter((candidate) => candidate.valid && !candidate.conflict)
        .map((candidate) => candidate.path),
    )
    selected.value = selected.value.filter((path) => allowed.has(path))
  }
})
watch(
  () => props.initialSource,
  (seed) => {
    if (!seed) return
    scope.value = seed.scope
    if (seed.source.kind === 'github') {
      kind.value = 'github'
      githubUrl.value = seed.source.url
      reference.value = seed.source.reference || ''
      subpath.value = seed.source.subpath || ''
    } else if (seed.source.kind === 'folder') {
      kind.value = 'folder'
      folderPath.value = seed.source.path
    } else if (seed.source.kind === 'archive') {
      kind.value = 'archive'
      archiveMode.value = seed.source.url ? 'url' : 'path'
      archiveLocation.value = seed.source.url || seed.source.path || ''
    }
  },
  { immediate: true },
)

function filesChanged(event: Event, folder: boolean) {
  const input = event.target as HTMLInputElement
  const files = Array.from(input.files || [])
  if (folder) folderFiles.value = files
  else archiveFiles.value = files.slice(0, 1)
}
function uploadError(files: File[], folder: boolean) {
  const problem = skillUploadProblem(files, folder)
  if (problem === 'empty') return st('Choose a file or folder first.')
  if (problem === 'too_large')
    return st('Browser uploads are limited to 32 MiB. Use a server path or GitHub for larger packages.')
  if (problem === 'too_many_files') return st('Browser folder uploads are limited to 2,000 files.')
  if (problem === 'unsafe_path' || problem === 'duplicate_path')
    return st('The selected folder contains invalid or duplicate relative paths.')
  return ''
}
async function sourceRequest(): Promise<SkillInstallSource> {
  if (kind.value === 'github')
    return {
      kind: 'github',
      url: githubUrl.value.trim(),
      ...(reference.value.trim() ? { reference: reference.value.trim() } : {}),
      ...(subpath.value.trim() ? { subpath: subpath.value.trim() } : {}),
    }
  if (kind.value === 'folder') return { kind: 'folder', path: folderPath.value.trim() }
  if (kind.value === 'archive' && archiveMode.value !== 'upload') {
    return archiveMode.value === 'url'
      ? { kind: 'archive', url: archiveLocation.value.trim() }
      : { kind: 'archive', path: archiveLocation.value.trim() }
  }
  const folder = kind.value === 'upload-folder'
  const files = folder ? folderFiles.value : archiveFiles.value
  const problem = uploadError(files, folder)
  if (problem) throw new Error(problem)
  return skillUploadSource(files, folder)
}
async function inspect() {
  if (!canPreview.value) return
  resetPreview()
  const currentGeneration = generation
  const requestedScope = scope.value
  const requestedDirectory = directory.currentDirectory
  phase.value = 'preview'
  controller = new AbortController()
  try {
    const source = await sourceRequest()
    if (disposed || currentGeneration !== generation) return
    const request = { scope: requestedScope, source }
    const workspace = requestedDirectory ? await resolveWorkspace(requestedDirectory) : null
    if (disposed || currentGeneration !== generation) return
    const client = createSkillsClient(workspace?.id)
    const next = await client.previewSkillInstall(request, controller.signal)
    if (disposed || currentGeneration !== generation) return
    snapshot.value = request
    previewClient.value = client
    preview.value = next
    selected.value = next.candidates
      .filter((candidate) => candidate.valid && (!candidate.conflict || replace.value))
      .map((candidate) => candidate.path)
  } catch (reason) {
    if (!disposed && currentGeneration === generation && !controller.signal.aborted) error.value = message(reason)
  } finally {
    if (!disposed) phase.value = ''
  }
}
function toggleCandidate(path: string, checked: boolean) {
  const next = new Set(selected.value)
  if (checked) next.add(path)
  else next.delete(path)
  selected.value = [...next]
}
function selectAll() {
  selected.value = validCandidates.value
    .filter((candidate) => !candidate.conflict || replace.value)
    .map((candidate) => candidate.path)
}
async function install() {
  const request = installRequest.value
  const client = previewClient.value
  if (!request || !client || busy.value) return
  const currentGeneration = generation
  phase.value = 'install'
  error.value = ''
  try {
    const installed = await client.installSkills(request)
    toasts.push('success', st('{count} skills installed', { count: installed.installed.length }))
    if (disposed || currentGeneration !== generation) return
    result.value = installed
    preview.value = null
    snapshot.value = null
    selected.value = []
  } catch (reason) {
    if (!disposed && currentGeneration === generation) error.value = message(reason)
  } finally {
    if (!disposed) phase.value = ''
  }
}
onBeforeRouteLeave(() => phase.value !== 'install')
onBeforeRouteUpdate(() => phase.value !== 'install')
function beforeUnload(event: BeforeUnloadEvent) {
  if (phase.value !== 'install') return
  event.preventDefault()
  event.returnValue = ''
}
onMounted(() => window.addEventListener('beforeunload', beforeUnload))
onBeforeUnmount(() => {
  disposed = true
  generation += 1
  controller?.abort()
  window.removeEventListener('beforeunload', beforeUnload)
})
defineExpose({
  refresh: async () => {
    if (preview.value && !busy.value) await inspect()
  },
})
</script>

<template>
  <div class="mx-auto grid max-w-5xl gap-5" :aria-busy="busy">
    <SettingsCard
      :title="$st('Choose a source')"
      :description="$st('Preview validates packages without installing. Select the skills you want, then install.')"
    >
      <div class="grid grid-cols-2 gap-2 lg:grid-cols-4" role="group" :aria-label="$st('Installation source')">
        <button
          v-for="source in sourceKinds"
          :key="source.value"
          type="button"
          :disabled="busy"
          :aria-pressed="kind === source.value"
          class="flex min-w-0 flex-col gap-1 rounded-xl border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
          :class="
            kind === source.value
              ? 'border-primary/40 bg-primary/5 ring-1 ring-inset ring-primary/10'
              : 'border-border/60 hover:bg-muted/40'
          "
          @click="kind = source.value"
        >
          <component
            :is="source.icon"
            class="mb-1 h-5 w-5"
            :class="kind === source.value ? 'text-primary' : 'text-muted-foreground'"
          />
          <span class="text-sm font-medium">{{ source.label }}</span>
          <span class="text-[11px] leading-relaxed text-muted-foreground">{{ source.description }}</span>
        </button>
      </div>
      <form class="mt-5 space-y-4" @submit.prevent="inspect">
        <template v-if="kind === 'github'">
          <label class="block space-y-1.5 text-xs font-medium"
            >{{ $st('GitHub repository or skill URL') }}
            <input
              v-model="githubUrl"
              required
              :disabled="busy"
              placeholder="https://github.com/owner/repository/tree/main/skills/example"
              class="block w-full rounded-md border border-input bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
            />
          </label>
          <div class="grid gap-4 sm:grid-cols-2">
            <label class="block space-y-1.5 text-xs font-medium"
              >{{ $st('Branch, tag, or commit (optional)') }}
              <input
                v-model="reference"
                :disabled="busy"
                class="block w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
              />
            </label>
            <label class="block space-y-1.5 text-xs font-medium"
              >{{ $st('Repository subfolder (optional)') }}
              <input
                v-model="subpath"
                :disabled="busy"
                class="block w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
              />
            </label>
          </div>
        </template>
        <template v-else-if="kind === 'archive'">
          <OptionPicker
            v-model="archiveMode"
            :options="archiveOptions"
            :include-empty="false"
            :disabled="busy"
            :title="$st('Archive source')"
          />
          <label
            v-if="archiveMode === 'upload'"
            class="block rounded-lg border border-dashed border-border bg-muted/10 p-4 text-xs font-medium"
          >
            {{ $st('Choose an archive') }}
            <input
              type="file"
              accept=".zip,.tar,.tar.gz,.tgz"
              :disabled="busy"
              class="mt-3 block w-full min-w-0 text-xs text-muted-foreground file:mr-3 file:rounded-md file:border-0 file:bg-secondary file:px-3 file:py-2 file:text-xs file:font-medium file:text-secondary-foreground"
              @change="filesChanged($event, false)"
            />
          </label>
          <label v-else class="block space-y-1.5 text-xs font-medium"
            >{{ archiveMode === 'url' ? $st('Archive URL') : $st('Archive path on the server') }}
            <input
              v-model="archiveLocation"
              required
              :disabled="busy"
              :placeholder="archiveMode === 'url' ? 'https://example.com/skills.zip' : '/path/to/skills.zip'"
              class="block w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
            />
          </label>
        </template>
        <label v-else-if="kind === 'folder'" class="block space-y-1.5 text-xs font-medium"
          >{{ $st('Folder path on the server') }}
          <input
            v-model="folderPath"
            required
            :disabled="busy"
            placeholder="/path/to/skills"
            class="block w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          <span class="block text-[11px] font-normal text-muted-foreground">{{
            $st('Choose a single skill folder or a collection containing multiple SKILL.md files.')
          }}</span>
        </label>
        <label v-else class="block rounded-lg border border-dashed border-border bg-muted/10 p-4 text-xs font-medium">
          {{ $st('Choose a folder from this device') }}
          <input
            type="file"
            webkitdirectory
            directory
            multiple
            :disabled="busy"
            class="mt-3 block w-full min-w-0 text-xs text-muted-foreground file:mr-3 file:rounded-md file:border-0 file:bg-secondary file:px-3 file:py-2 file:text-xs file:font-medium file:text-secondary-foreground"
            @change="filesChanged($event, true)"
          />
        </label>
        <div
          v-if="kind === 'upload-folder' || (kind === 'archive' && archiveMode === 'upload')"
          class="space-y-1 text-xs text-muted-foreground"
        >
          <p v-if="uploadSummary.count">
            {{ $st('{count} files · {size} MiB', { count: uploadSummary.count, size: uploadSummary.mib }) }}
          </p>
          <p>
            {{
              $st(
                'Browser uploads support up to 32 MiB and 2,000 files. Larger packages can use a server path or GitHub.',
              )
            }}
          </p>
        </div>
        <div class="flex flex-wrap items-end justify-between gap-4 border-t border-border/60 pt-4">
          <div class="min-w-48 space-y-1.5 text-xs font-medium">
            <span>{{ $st('Install scope') }}</span>
            <OptionPicker
              :model-value="scope"
              :options="scopeOptions"
              :include-empty="false"
              :disabled="busy"
              :title="$st('Install scope')"
              @update:model-value="scope = $event as SkillScope"
            />
          </div>
          <Button type="submit" :disabled="!canPreview">{{
            phase === 'preview' ? $st('Inspecting source…') : $st('Preview installation')
          }}</Button>
        </div>
      </form>
    </SettingsCard>

    <p
      v-if="error"
      role="alert"
      class="whitespace-pre-wrap break-words rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
    >
      {{ error }}
    </p>

    <SettingsCard v-if="preview" :title="$st('Choose skills to install')">
      <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
        <p class="text-xs text-muted-foreground">
          {{
            $st('{selected} of {total} skills selected', {
              selected: selected.length,
              total: preview.candidates.length,
            })
          }}
        </p>
        <div class="flex gap-2">
          <Button size="sm" variant="ghost" :disabled="busy" @click="selectAll">{{ $st('Select all valid') }}</Button>
          <Button size="sm" variant="ghost" :disabled="busy" @click="selected = []">{{
            $st('Clear selection')
          }}</Button>
        </div>
      </div>
      <div
        v-if="!preview.candidates.length"
        class="rounded-lg bg-muted/30 px-4 py-8 text-center text-sm text-muted-foreground"
      >
        {{ $st('No SKILL.md packages were found in this source.') }}
      </div>
      <div v-else class="max-h-[32rem] divide-y divide-border/60 overflow-auto rounded-lg border border-border/60">
        <label
          v-for="candidate in preview.candidates"
          :key="candidate.path"
          class="flex items-start gap-3 p-3"
          :class="
            !candidate.valid || (candidate.conflict && !replace) ? 'bg-muted/20' : 'cursor-pointer hover:bg-muted/20'
          "
        >
          <input
            type="checkbox"
            :checked="selected.includes(candidate.path)"
            :disabled="busy || !candidate.valid || (candidate.conflict && !replace)"
            class="mt-1 h-4 w-4 shrink-0 rounded accent-primary"
            @change="toggleCandidate(candidate.path, ($event.target as HTMLInputElement).checked)"
          />
          <div class="min-w-0 flex-1">
            <div class="flex flex-wrap items-center gap-2">
              <span class="text-sm font-medium">{{ candidate.name }}</span>
              <span
                v-if="candidate.conflict"
                class="rounded bg-amber-500/10 px-2 py-0.5 text-[10px] text-amber-800 dark:text-amber-200"
                >{{ $st('Already installed') }}</span
              >
              <span
                v-if="!candidate.valid"
                class="rounded bg-destructive/10 px-2 py-0.5 text-[10px] text-destructive"
                >{{ $st('Needs attention') }}</span
              >
            </div>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ candidate.description }}</p>
            <p class="mt-1 break-all font-mono text-[10px] text-muted-foreground">{{ candidate.path }}</p>
            <p
              v-for="(issue, index) in candidate.diagnostics"
              :key="index"
              class="mt-1 text-xs text-amber-800 dark:text-amber-200"
            >
              {{ issue }}
            </p>
          </div>
        </label>
      </div>
      <label
        v-if="conflictCount"
        class="mt-4 flex items-start gap-3 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3"
      >
        <input
          v-model="replace"
          type="checkbox"
          :disabled="busy"
          class="mt-0.5 h-4 w-4 shrink-0 rounded accent-primary"
        />
        <span class="text-xs"
          ><span class="font-medium">{{ $st('Replace selected existing skills') }}</span>
          <span class="mt-1 block text-muted-foreground">{{
            $st('Replacement overwrites the existing skill folder, including local edits. Unselected skills are kept.')
          }}</span>
        </span>
      </label>
      <div class="mt-4 flex flex-wrap items-center justify-between gap-3">
        <p class="text-xs text-muted-foreground">
          {{ $st('Destination: {scope}', { scope: scope === 'global' ? $st('Global') : $st('Workspace') }) }}
        </p>
        <Button :disabled="busy || !installRequest" @click="install">{{
          phase === 'install' ? $st('Installing skills…') : $st('Install selected skills')
        }}</Button>
      </div>
    </SettingsCard>

    <SettingsCard v-if="result" :title="$st('Installation complete')">
      <p class="mb-3 text-sm text-muted-foreground">
        {{ $st('{count} skills installed', { count: result.installed.length }) }}
      </p>
      <div class="grid gap-2 sm:grid-cols-2">
        <button
          v-for="skill in result.installed"
          :key="skill.id"
          type="button"
          class="rounded-lg border border-border/60 p-3 text-left transition-colors hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          @click="emit('installed', skill.id)"
        >
          <span class="block text-sm font-medium">{{ skill.name }}</span
          ><span class="mt-1 block text-xs text-muted-foreground">{{ $st('Open in skill library') }}</span>
        </button>
      </div>
    </SettingsCard>
  </div>
</template>
