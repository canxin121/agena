<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useId, watch } from 'vue'
import { RiRefreshLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { mcpConnectionsApi as api, type McpCapabilityKind } from '@/lib/mcpConnectionsApi'
import { mcpRecord } from '@/lib/mcpConnections'

const props = defineProps<{ connectionId: string; enabled: boolean; kind: McpCapabilityKind }>()
const fieldId = useId()
const items = ref<Record<string, unknown>[]>([])
const selected = ref<Record<string, unknown> | null>(null)
const cursor = ref<string | null>(null)
const query = ref('')
const loading = ref(false)
const reading = ref(false)
const error = ref('')
const resultError = ref('')
const result = ref<Record<string, unknown> | null>(null)
const uri = ref('')
const argumentsDraft = ref<Record<string, string>>({})
const resourceKind = ref<McpCapabilityKind>('resources')
const effectiveKind = computed(() => props.kind === 'resources' ? resourceKind.value : props.kind)
const resourceKinds = computed(() => [
  { value: 'resources', label: st('Resources') },
  { value: 'resource_templates', label: st('Resource templates') },
])
const filtered = computed(() => {
  const needle = query.value.trim().toLocaleLowerCase()
  return items.value.filter((item) => !needle || ['name', 'title', 'description', 'uri', 'uriTemplate'].some((key) => String(item[key] || '').toLocaleLowerCase().includes(needle)))
})
const promptArguments = computed(() => { const args = selected.value?.arguments; return Array.isArray(args) ? args.map(mcpRecord) : [] })
const resultText = computed(() => JSON.stringify(result.value, (key, value) => key === 'blob' && typeof value === 'string' ? st('Binary content') : value, 2))
let generation = 0
let previewGeneration = 0
let controller: AbortController | undefined
let disposed = false
function title(item: Record<string, unknown>) { return String(item.title || item.name || item.uri || item.uriTemplate || '') }
function choose(item: Record<string, unknown>) {
  previewGeneration++
  selected.value = item
  result.value = null
  resultError.value = ''
  reading.value = false
  uri.value = String(item.uri || item.uriTemplate || '')
  argumentsDraft.value = {}
}
async function load(more = false) {
  if (!props.enabled || (more && !cursor.value)) return
  const requestedId = props.connectionId
  const requestedKind = effectiveKind.value
  const request = ++generation
  controller?.abort()
  controller = new AbortController()
  loading.value = true
  error.value = ''
  if (!more) { items.value = []; cursor.value = null; selected.value = null; result.value = null; previewGeneration++ }
  try {
    const page = await api.capabilities(requestedId, requestedKind, more ? cursor.value : null, controller.signal)
    if (disposed || request !== generation) return
    items.value = more ? [...items.value, ...page.items] : page.items
    cursor.value = page.next_cursor
    if (!selected.value && items.value[0]) choose(items.value[0])
  } catch (reason) {
    if (!disposed && request === generation && !controller.signal.aborted) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally { if (request === generation) loading.value = false }
}
async function preview() {
  if (!props.enabled || !selected.value || reading.value) return
  const request = ++previewGeneration
  const id = props.connectionId
  const kind = effectiveKind.value
  const item = selected.value
  const args = Object.fromEntries(promptArguments.value.map((argument) => [String(argument.name), argumentsDraft.value[String(argument.name)] || '']))
  reading.value = true
  resultError.value = ''
  result.value = null
  try {
    const next = kind === 'prompts' ? await api.getPrompt(id, String(item.name || ''), args) : await api.readResource(id, uri.value.trim())
    if (!disposed && request === previewGeneration) result.value = next
  } catch (reason) {
    if (!disposed && request === previewGeneration) resultError.value = reason instanceof Error ? reason.message : String(reason)
  } finally { if (request === previewGeneration) reading.value = false }
}
watch(() => [props.connectionId, effectiveKind.value, props.enabled], () => {
  generation++; previewGeneration++; controller?.abort(); items.value = []; selected.value = null; cursor.value = null; result.value = null; error.value = ''; resultError.value = ''; reading.value = false
  if (props.enabled) void load()
}, { immediate: true })
onBeforeUnmount(() => { disposed = true; generation++; previewGeneration++; controller?.abort() })
</script>

<template>
  <div class="min-w-0 space-y-4">
    <p v-if="!enabled" class="rounded-xl border border-border/70 bg-muted/10 p-4 text-sm text-muted-foreground">{{ st('Connect this service to inspect its capabilities.') }}</p>
    <template v-else>
      <div class="flex flex-wrap items-center gap-2">
        <div class="min-w-40 flex-1"><SearchInput v-model="query" :placeholder="st('Search capabilities')" /></div>
        <OptionPicker v-if="kind === 'resources'" :model-value="resourceKind" @update:model-value="resourceKind = $event === 'resource_templates' ? 'resource_templates' : 'resources'" :options="resourceKinds" :include-empty="false" :title="st('Resource kind')" />
        <IconButton :disabled="loading" :tooltip="st('Refresh capabilities')" :aria-label="st('Refresh capabilities')" @click="load()"><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" /></IconButton>
      </div>
      <p v-if="error" role="alert" class="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">{{ error }}</p>
      <p v-if="loading && !items.length" role="status" class="p-6 text-center text-sm text-muted-foreground">{{ st('Loading capabilities…') }}</p>
      <p v-else-if="!filtered.length" class="rounded-xl border border-dashed border-border/70 p-6 text-center text-sm text-muted-foreground">{{ query ? st('No matching capabilities.') : st('This service did not advertise any entries in this category.') }}</p>
      <div v-else class="grid min-w-0 gap-4 xl:grid-cols-[minmax(12rem,0.8fr)_minmax(0,1.2fr)]">
        <div class="min-w-0 space-y-2">
          <div class="max-h-80 space-y-1 overflow-y-auto rounded-xl border border-border/70 p-1.5">
            <button v-for="(item, index) in filtered" :key="String(item.name || item.uri || item.uriTemplate || index)" type="button" :aria-pressed="selected === item" :aria-label="title(item)" :aria-describedby="item.description ? fieldId + '-description-' + index : undefined" class="w-full min-w-0 rounded-lg border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" :class="selected === item ? 'border-primary/25 bg-primary/10' : 'border-transparent hover:bg-muted/40'" @click="choose(item)">
              <span class="block break-words text-sm font-medium">{{ title(item) }}</span>
              <span v-if="item.description" :id="fieldId + '-description-' + index" class="mt-1 line-clamp-3 block break-words text-xs leading-relaxed text-muted-foreground">{{ item.description }}</span>
            </button>
          </div>
          <Button v-if="cursor" variant="outline" size="sm" :disabled="loading" @click="load(true)">{{ st('Load more capabilities') }}</Button>
        </div>
        <section v-if="selected" class="min-w-0 space-y-4 rounded-xl border border-border/70 p-4">
          <div><h3 class="break-words text-sm font-semibold">{{ title(selected) }}</h3><p v-if="selected.description" class="mt-1 break-words text-xs leading-relaxed text-muted-foreground">{{ selected.description }}</p></div>
          <template v-if="effectiveKind === 'prompts'">
            <div v-for="(argument, index) in promptArguments" :key="String(argument.name)" class="space-y-1.5">
              <label class="block text-xs font-medium" :for="fieldId + '-argument-' + index">{{ argument.name }}<span v-if="argument.required" aria-hidden="true" class="ml-1 text-destructive">*</span></label>
              <input :id="fieldId + '-argument-' + index" v-model="argumentsDraft[String(argument.name)]" :aria-required="Boolean(argument.required)" :aria-describedby="argument.description ? fieldId + '-argument-help-' + index : undefined" :disabled="reading" class="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm" />
              <p v-if="argument.description" :id="fieldId + '-argument-help-' + index" class="text-xs text-muted-foreground">{{ argument.description }}</p>
            </div>
            <Button size="sm" :disabled="reading" @click="preview">{{ reading ? st('Loading preview…') : st('Preview prompt') }}</Button>
          </template>
          <template v-else-if="effectiveKind === 'resources' || effectiveKind === 'resource_templates'">
            <label class="block space-y-1.5" :for="fieldId + '-uri'"><span class="text-xs font-medium">{{ st('Resource URI') }}</span><input :id="fieldId + '-uri'" v-model="uri" :disabled="reading" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs" spellcheck="false" /></label>
            <p v-if="effectiveKind === 'resource_templates'" class="text-xs text-muted-foreground">{{ st('Replace template parameters with the resource you want to read.') }}</p>
            <Button size="sm" :disabled="reading || !uri.trim() || /\{[^}]*\}/.test(uri)" @click="preview">{{ reading ? st('Loading preview…') : st('Read resource') }}</Button>
          </template>
          <details :open="effectiveKind === 'tools'" class="min-w-0 rounded-lg border border-border/60"><summary class="cursor-pointer px-3 py-2 text-xs font-medium">{{ st('Capability definition') }}</summary><pre class="max-h-96 overflow-auto whitespace-pre-wrap break-words border-t border-border/60 p-3 font-mono text-[11px] leading-relaxed">{{ JSON.stringify(selected, null, 2) }}</pre></details>
          <p v-if="resultError" role="alert" class="break-words text-xs text-destructive">{{ resultError }}</p>
          <div v-if="result" class="min-w-0 space-y-2"><h4 class="text-xs font-semibold">{{ st('Server response') }}</h4><pre class="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border/70 bg-muted/10 p-3 font-mono text-[11px] leading-relaxed">{{ resultText }}</pre></div>
        </section>
      </div>
    </template>
  </div>
</template>
