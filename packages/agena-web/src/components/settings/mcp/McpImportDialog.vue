<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useId, watch } from 'vue'
import { RiUpload2Line } from '@remixicon/vue'
import Dialog from '@/components/ui/Dialog.vue'
import Button from '@/components/ui/Button.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { mcpConnectionsApi as api, type McpImportPreview, type McpImportResult, type McpScope } from '@/lib/mcpConnectionsApi'
import { parseMcpImport } from '@/lib/mcpConnections'
import { confirmAction } from '@/lib/appConfirm'

const props = defineProps<{ open: boolean; workspaceRoot: string }>()
const emit = defineEmits<{ 'update:open': [value: boolean]; installed: [result: McpImportResult]; dirty: [value: boolean] }>()
const fieldId = useId()
const scope = ref<McpScope>('workspace')
const documentText = ref('')
const preview = ref<McpImportPreview | null>(null)
const selected = ref<string[]>([])
const replace = ref(false)
const busy = ref('')
const error = ref('')
const previewKey = ref('')
const currentKey = computed(() => JSON.stringify([scope.value, documentText.value]))
const currentPreview = computed(() => preview.value && previewKey.value === currentKey.value ? preview.value : null)
const scopeOptions = computed(() => [{ value: 'workspace', label: st('Workspace') }, { value: 'global', label: st('Global') }])
const selectable = computed(() => (currentPreview.value?.candidates || []).filter((candidate) => candidate.valid && (!candidate.conflict || replace.value)))
const selectedCount = computed(() => selectable.value.filter((candidate) => selected.value.includes(candidate.name)).length)
let generation = 0
let controller: AbortController | undefined
let disposed = false
const message = (reason: unknown) => st(reason instanceof Error ? reason.message : String(reason))
function resetPreview() {
  generation++; controller?.abort(); preview.value = null; previewKey.value = ''; selected.value = []; error.value = ''
}
function reset() { resetPreview(); documentText.value = ''; scope.value = 'workspace'; replace.value = false; busy.value = '' }
async function close() {
  if (busy.value) return
  if (documentText.value.trim() && !(await confirmAction({ title: st('Discard import?'), description: st('The imported configuration has not been saved.'), confirmText: st('Discard changes'), cancelText: st('Keep editing') }))) return
  emit('update:open', false)
}
async function upload(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  const request = ++generation
  try {
    if (file.size > 1024 * 1024) throw new Error(st('Choose a configuration file smaller than 1 MiB.'))
    const text = await file.text()
    if (!disposed && request === generation && props.open) documentText.value = text
  } catch (reason) { if (request === generation) error.value = message(reason) }
  finally { input.value = '' }
}
async function inspect() {
  if (busy.value) return
  resetPreview()
  const request = ++generation
  const key = currentKey.value
  controller = new AbortController()
  busy.value = 'preview'
  try {
    const parsed = parseMcpImport(documentText.value)
    const result = await api.previewImport(scope.value, parsed, controller.signal)
    if (disposed || request !== generation || key !== currentKey.value) return
    preview.value = result
    previewKey.value = key
    selected.value = result.candidates.filter((candidate) => candidate.valid && (!candidate.conflict || replace.value)).map((candidate) => candidate.name)
  } catch (reason) { if (!disposed && request === generation && !controller.signal.aborted) error.value = message(reason) }
  finally { if (request === generation) busy.value = '' }
}
function toggle(name: string, checked: boolean) {
  selected.value = checked ? [...new Set([...selected.value, name])] : selected.value.filter((candidate) => candidate !== name)
}
async function install() {
  if (busy.value || !currentPreview.value || !selectedCount.value) return
  const requested = currentPreview.value
  const key = currentKey.value
  const request = ++generation
  const names = selectable.value.filter((candidate) => selected.value.includes(candidate.name)).map((candidate) => candidate.name)
  busy.value = 'install'
  error.value = ''
  try {
    const result = await api.import(scope.value, parseMcpImport(documentText.value), names, replace.value, requested.revision)
    if (disposed || request !== generation || key !== currentKey.value) return
    emit('installed', result)
    emit('update:open', false)
  } catch (reason) { if (!disposed && request === generation) error.value = message(reason) }
  finally { if (request === generation) busy.value = '' }
}
watch([documentText, scope], () => { emit('dirty', Boolean(documentText.value.trim())); if (!busy.value) resetPreview() })
watch(replace, () => { if (!replace.value) selected.value = selected.value.filter((name) => selectable.value.some((candidate) => candidate.name === name)) })
watch(() => props.open, (open) => { if (!open) reset() })
onBeforeUnmount(() => { disposed = true; generation++; controller?.abort(); documentText.value = '' })
</script>

<template>
  <Dialog :open="open" :title="st('Import MCP connections')" :description="st('Paste or upload an MCP configuration, review the services, then choose what to import.')" max-width="max-w-3xl" @update:open="(value) => { if (!value) void close() }">
    <div class="space-y-5">
      <div class="flex flex-wrap items-end justify-between gap-3">
        <div class="space-y-1.5"><span class="text-xs font-medium">{{ st('Import scope') }}</span><OptionPicker :model-value="scope" :options="scopeOptions" :include-empty="false" :disabled="Boolean(busy)" :title="st('Import scope')" @update:model-value="scope = $event === 'global' ? 'global' : 'workspace'" /></div>
        <label class="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-input bg-background px-3 py-2 text-xs font-medium" :class="busy ? 'pointer-events-none opacity-50' : 'hover:bg-muted/40'"><RiUpload2Line class="h-4 w-4" />{{ st('Choose JSON file') }}<input type="file" accept=".json,application/json" class="sr-only" :disabled="Boolean(busy)" @change="upload" /></label>
      </div>
      <p v-if="scope === 'workspace'" class="break-all text-xs text-muted-foreground">{{ st('Server workspace') }}: {{ workspaceRoot }}</p>
      <div class="space-y-1.5">
        <label class="block text-xs font-medium" :for="fieldId + '-document'">{{ st('MCP configuration JSON') }}</label>
        <textarea :id="fieldId + '-document'" v-model="documentText" :aria-describedby="fieldId + '-document-help'" :disabled="Boolean(busy)" rows="10" spellcheck="false" class="w-full rounded-xl border border-input bg-background p-3 font-mono text-xs leading-relaxed" />
        <p :id="fieldId + '-document-help'" class="text-xs leading-relaxed text-muted-foreground">{{ st('Supports mcpServers configurations and native Agena servers maps. Importing does not execute tools.') }}</p>
      </div>
      <p v-if="error" role="alert" class="break-words rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">{{ error }}</p>
      <section v-if="currentPreview" class="space-y-3 rounded-xl border border-border/70 p-3 sm:p-4">
        <div class="flex flex-wrap items-center justify-between gap-2"><h3 class="text-sm font-semibold">{{ st('Choose connections to import') }}</h3><span class="text-xs text-muted-foreground">{{ selectedCount }} / {{ currentPreview.candidates.length }}</span></div>
        <ul v-if="currentPreview.errors.length" class="list-inside list-disc text-xs text-destructive"><li v-for="issue in currentPreview.errors" :key="issue">{{ issue }}</li></ul>
        <label class="flex items-start gap-2 text-xs"><input v-model="replace" type="checkbox" :disabled="Boolean(busy)" class="mt-0.5 accent-primary" /><span>{{ st('Replace existing connections with the same name in this scope') }}</span></label>
        <div class="flex flex-wrap gap-2"><Button size="sm" variant="ghost" :disabled="Boolean(busy)" @click="selected = selectable.map((candidate) => candidate.name)">{{ st('Select all valid') }}</Button><Button size="sm" variant="ghost" :disabled="Boolean(busy)" @click="selected = []">{{ st('Clear selection') }}</Button></div>
        <p v-if="!currentPreview.candidates.length" class="py-3 text-sm text-muted-foreground">{{ st('No MCP connections were found in this configuration.') }}</p>
        <div v-for="candidate in currentPreview.candidates" :key="candidate.name" class="min-w-0 rounded-lg border border-border/60 p-3">
          <label class="flex items-start gap-2.5"><input type="checkbox" :checked="selected.includes(candidate.name)" :disabled="Boolean(busy) || !candidate.valid || (candidate.conflict && !replace)" class="mt-0.5 accent-primary" @change="toggle(candidate.name, ($event.target as HTMLInputElement).checked)" /><span class="min-w-0"><span class="block break-words text-sm font-medium">{{ candidate.name }}</span><span class="mt-1 flex flex-wrap gap-1.5 text-[11px] text-muted-foreground"><span>{{ candidate.config.transport }}</span><span v-if="candidate.conflict" class="text-amber-700 dark:text-amber-300">{{ st('Already configured') }}</span><span v-if="!candidate.valid" class="text-destructive">{{ st('Invalid configuration') }}</span></span></span></label>
          <ul v-if="candidate.errors.length" class="mt-2 list-inside list-disc text-xs text-destructive"><li v-for="issue in candidate.errors" :key="issue">{{ issue }}</li></ul>
          <details class="mt-2"><summary class="cursor-pointer text-xs text-muted-foreground">{{ st('Review configuration') }}</summary><pre class="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/30 p-2 font-mono text-[11px]">{{ JSON.stringify(candidate.config, null, 2) }}</pre></details>
        </div>
      </section>
    </div>
    <template #footer>
      <Button variant="ghost" :disabled="Boolean(busy)" @click="close">{{ st('Cancel') }}</Button>
      <div class="flex-1" />
      <Button v-if="!currentPreview" :disabled="Boolean(busy) || !documentText.trim()" @click="inspect">{{ busy === 'preview' ? st('Checking configuration…') : st('Preview import') }}</Button>
      <Button v-else :disabled="Boolean(busy) || !selectedCount" @click="install">{{ busy === 'install' ? st('Importing connections…') : st('Import selected connections') }}</Button>
    </template>
  </Dialog>
</template>
