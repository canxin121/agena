<script setup lang="ts">
import { computed, ref, useId, watch } from 'vue'
import { RiAddLine, RiDeleteBinLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SettingsToggle from '../SettingsToggle.vue'
import McpMapEditor from './McpMapEditor.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { changeMcpValue, cloneMcpConfig, mcpRecord, mcpValueAt, newMcpConfig, preservedMcpSecrets, type McpDraft } from '@/lib/mcpConnections'

const props = defineProps<{ modelValue: McpDraft; disabled?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: McpDraft]; validity: [valid: boolean] }>()
const fieldId = useId()
const jsonMode = ref(false)
const jsonText = ref('')
const jsonError = ref('')
let lastPublishedJson = ''
const config = computed(() => props.modelValue.config)
const transport = computed(() => String(config.value.transport || ''))
const process = computed(() => mcpRecord(config.value.process))
const args = computed(() => Array.isArray(process.value.args) ? process.value.args : [])
const auth = computed(() => mcpRecord(config.value.auth))
const registrationOpen = ref(Boolean(auth.value.client_id || auth.value.client_secret || auth.value.client_metadata_url))
watch(() => Boolean(auth.value.client_id || auth.value.client_secret || auth.value.client_metadata_url), (configured) => {
  if (configured) registrationOpen.value = true
})
const authKind = computed(() => { const kind = String(auth.value.kind || 'none'); return kind === 'oauth' ? 'o_auth' : kind })
const authOptions = computed(() => [
  { value: 'none', label: st('No authentication') },
  { value: 'bearer', label: st('Bearer token') },
  { value: 'bearer_from_env', label: st('Token from environment') },
  { value: 'bearer_from_store', label: st('Token from secret store') },
  { value: 'o_auth', label: st('OAuth sign-in') },
  { value: 'custom', label: st('Custom authentication headers') },
])
const value = (pointer: string) => mcpValueAt(config.value, pointer)
const text = (pointer: string) => String(value(pointer) ?? '')
const secretPreserved = (pointer: string) => props.modelValue.preserveSecrets.includes(pointer)
function set(pointer: string, next: unknown, remove = false) {
  emit('update:modelValue', changeMcpValue(props.modelValue, pointer, next, remove))
}
function setOptional(pointer: string, next: string) { set(pointer, next, next === '') }
function changeTransport(kind: 'stdio' | 'http') {
  if (transport.value === kind) return
  const defaults = newMcpConfig(kind)
  for (const key of ['enabled', 'timeouts', 'tools']) if (Object.hasOwn(config.value, key)) defaults[key] = cloneMcpConfig(config.value[key])
  emit('update:modelValue', { config: defaults, preserveSecrets: [] })
}
function changeAuth(kind: string) {
  const next = kind === 'none' ? null : kind === 'bearer' ? { kind, token: '' } : kind === 'bearer_from_env' ? { kind, env: '' } : kind === 'o_auth' ? { kind, scopes: [] } : kind === 'custom' ? { kind, headers: {} } : { kind }
  set('/auth', next)
}
function updateArg(index: number, next: string) {
  const updated = [...args.value]
  updated[index] = next
  set('/process/args', updated)
}
function listText(pointer: string) { const next = value(pointer); return Array.isArray(next) ? next.join('\n') : '' }
function setList(pointer: string, input: string) { set(pointer, input.split(/\r?\n/).map((line) => line.trim()).filter(Boolean)) }
function toggleJson() {
  if (jsonMode.value && jsonError.value) return
  jsonMode.value = !jsonMode.value
  jsonText.value = JSON.stringify(config.value, null, 2)
  jsonError.value = ''
  emit('validity', true)
}
function updateJson(input: string) {
  jsonText.value = input
  try {
    const parsed: unknown = JSON.parse(input)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error()
    const draft = { config: parsed as Record<string, unknown>, preserveSecrets: props.modelValue.preserveSecrets }
    draft.preserveSecrets = preservedMcpSecrets(draft)
    lastPublishedJson = JSON.stringify(draft.config)
    emit('update:modelValue', draft)
    jsonError.value = ''
  } catch { jsonError.value = st('Enter a valid JSON object.') }
  emit('validity', !jsonError.value)
}
watch(() => props.modelValue, (next) => {
  const serialized = JSON.stringify(next.config)
  if (jsonMode.value && !jsonError.value && serialized !== lastPublishedJson) jsonText.value = JSON.stringify(next.config, null, 2)
  lastPublishedJson = serialized
})
</script>

<template>
  <div class="min-w-0 space-y-5">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <h3 class="text-sm font-semibold">{{ st('Connection configuration') }}</h3>
      <Button type="button" variant="ghost" size="sm" :disabled="disabled || Boolean(jsonError)" @click="toggleJson">{{ jsonMode ? st('Structured form') : st('Edit JSON') }}</Button>
    </div>
    <div v-if="jsonMode" class="space-y-2">
      <label :for="fieldId + '-json'" class="text-xs font-medium">{{ st('Connection JSON') }}</label>
      <textarea :id="fieldId + '-json'" :value="jsonText" :disabled="disabled" spellcheck="false" class="min-h-80 w-full resize-y rounded-lg border border-input bg-background p-3 font-mono text-xs leading-relaxed" @input="updateJson(($event.target as HTMLTextAreaElement).value)" />
      <p v-if="jsonError" role="alert" class="text-xs text-destructive">{{ jsonError }}</p>
      <p class="text-xs leading-relaxed text-muted-foreground">{{ st('Keep existing redacted placeholders to preserve stored secrets. Replace or remove a field to change it.') }}</p>
    </div>
    <template v-else>
      <fieldset :disabled="disabled" class="space-y-5">
        <div class="grid grid-cols-2 gap-2" role="group" :aria-label="st('Transport')">
          <button v-for="kind in ['stdio', 'http'] as const" :key="kind" type="button" :disabled="disabled" :aria-pressed="transport === kind" :aria-label="kind === 'stdio' ? st('Local command') : st('Remote HTTP')" :aria-describedby="fieldId + '-transport-' + kind" class="min-w-0 rounded-xl border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50" :class="transport === kind ? 'border-primary/50 bg-primary/5 ring-1 ring-primary/10' : 'border-border/70 hover:bg-muted/30'" @click="changeTransport(kind)">
            <span class="block text-sm font-medium">{{ kind === 'stdio' ? st('Local command') : st('Remote HTTP') }}</span>
            <span :id="fieldId + '-transport-' + kind" class="mt-1 block text-xs leading-relaxed text-muted-foreground">{{ kind === 'stdio' ? st('Start an MCP process on the Agena server.') : st('Connect to a Streamable HTTP endpoint.') }}</span>
          </button>
        </div>
        <div v-if="transport === 'stdio'" class="space-y-5">
          <label class="block space-y-1.5" :for="fieldId + '-command'">
            <span class="text-xs font-medium">{{ st('Command') }}</span>
            <input :id="fieldId + '-command'" :value="text('/process/command')" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-sm" autocomplete="off" spellcheck="false" @input="set('/process/command', ($event.target as HTMLInputElement).value)" />
          </label>
          <fieldset class="space-y-2">
            <legend class="sr-only">{{ st('Arguments') }}</legend><div class="flex items-center justify-between gap-2"><span class="text-xs font-medium">{{ st('Arguments') }}</span><Button type="button" variant="ghost" size="sm" :disabled="disabled" @click="set('/process/args', [...args, ''])"><RiAddLine class="mr-1 h-3.5 w-3.5" />{{ st('Add argument') }}</Button></div>
            <p v-if="!args.length" class="text-xs text-muted-foreground">{{ st('No arguments. Add one field for each command-line argument.') }}</p>
            <div v-for="(arg, index) in args" :key="index" class="flex min-w-0 items-center gap-2">
              <label class="sr-only" :for="fieldId + '-arg-' + index">{{ st('Argument') }} {{ index + 1 }}</label>
              <span class="w-5 shrink-0 text-right text-[11px] tabular-nums text-muted-foreground">{{ index + 1 }}</span>
              <input :id="fieldId + '-arg-' + index" :value="String(arg)" class="min-w-0 flex-1 rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs" autocomplete="off" spellcheck="false" @input="updateArg(index, ($event.target as HTMLInputElement).value)" />
              <IconButton type="button" size="sm" :disabled="disabled" :aria-label="st('Remove argument') + ' ' + (index + 1)" :tooltip="st('Remove argument')" @click="set('/process/args', args.filter((_, position) => position !== index))"><RiDeleteBinLine class="h-4 w-4" /></IconButton>
            </div>
          </fieldset>
          <div class="space-y-1.5">
            <label class="block text-xs font-medium" :for="fieldId + '-cwd'">{{ st('Working directory') }}</label>
            <input :id="fieldId + '-cwd'" :value="text('/process/cwd')" :aria-describedby="fieldId + '-cwd-help'" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs" autocomplete="off" spellcheck="false" @input="set('/process/cwd', ($event.target as HTMLInputElement).value || undefined, !($event.target as HTMLInputElement).value)" />
            <p :id="fieldId + '-cwd-help'" class="text-xs text-muted-foreground">{{ st('Optional path on the Agena server.') }}</p>
          </div>
          <McpMapEditor :model-value="modelValue" pointer="/process/env" :title="st('Environment variables')" kind="env" :disabled="disabled" @update:model-value="emit('update:modelValue', $event)" />
        </div>
        <div v-else-if="transport === 'http'" class="space-y-5">
          <div class="space-y-2">
            <label class="block space-y-1.5" :for="fieldId + '-url'"><span class="text-xs font-medium">{{ st('MCP endpoint URL') }}</span><input :id="fieldId + '-url'" :value="secretPreserved('/endpoint/url') ? '' : text('/endpoint/url')" :placeholder="secretPreserved('/endpoint/url') ? st('Stored endpoint URL') : ''" :disabled="disabled || secretPreserved('/endpoint/url')" type="url" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-sm disabled:opacity-70" autocomplete="off" spellcheck="false" @input="set('/endpoint/url', ($event.target as HTMLInputElement).value)" /></label>
            <Button v-if="secretPreserved('/endpoint/url')" type="button" variant="outline" size="sm" :disabled="disabled" @click="set('/endpoint/url', '')">{{ st('Replace endpoint URL') }}</Button>
          </div>
          <div class="space-y-1.5"><span class="text-xs font-medium">{{ st('Authentication') }}</span><OptionPicker :model-value="authKind" :options="authOptions" :title="st('Authentication')" :include-empty="false" :disabled="disabled" @update:model-value="changeAuth" /></div>
          <div v-if="authKind === 'bearer'" class="space-y-2">
            <label class="block space-y-1.5" :for="fieldId + '-token'"><span class="text-xs font-medium">{{ st('Bearer token') }}</span><input :id="fieldId + '-token'" type="password" :value="secretPreserved('/auth/token') ? '' : text('/auth/token')" :placeholder="secretPreserved('/auth/token') ? st('Stored value') : ''" :disabled="disabled || secretPreserved('/auth/token')" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-sm disabled:opacity-70" autocomplete="new-password" @input="set('/auth/token', ($event.target as HTMLInputElement).value)" /></label>
            <Button v-if="secretPreserved('/auth/token')" type="button" variant="outline" size="sm" :disabled="disabled" @click="set('/auth/token', '')">{{ st('Replace token') }}</Button>
          </div>
          <label v-else-if="authKind === 'bearer_from_env'" class="block space-y-1.5" :for="fieldId + '-token-env'"><span class="text-xs font-medium">{{ st('Token environment variable') }}</span><input :id="fieldId + '-token-env'" :value="text('/auth/env')" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-sm" autocomplete="off" spellcheck="false" @input="set('/auth/env', ($event.target as HTMLInputElement).value)" /></label>
          <div v-else-if="authKind === 'o_auth'" class="space-y-4">
            <div class="space-y-1.5">
              <label class="block text-xs font-medium" :for="fieldId + '-scopes'">{{ st('OAuth scopes') }}</label>
              <textarea :id="fieldId + '-scopes'" :value="listText('/auth/scopes')" :aria-describedby="fieldId + '-scopes-help'" rows="3" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs" spellcheck="false" @input="setList('/auth/scopes', ($event.target as HTMLTextAreaElement).value)" />
              <p :id="fieldId + '-scopes-help'" class="text-xs text-muted-foreground">{{ st('One scope per line. Save the connection before signing in.') }}</p>
            </div>
            <details class="rounded-xl border border-border/70 bg-muted/10" :open="registrationOpen" @toggle="registrationOpen = ($event.target as HTMLDetailsElement).open">
              <summary class="cursor-pointer px-3 py-3 text-xs font-medium">{{ st('OAuth client registration') }}</summary>
              <div class="space-y-4 border-t border-border/60 p-3">
                <p class="text-xs leading-relaxed text-muted-foreground">{{ st('Leave these fields empty for automatic registration. Use a client ID or a client metadata URL for an existing registration.') }}</p>
                <label class="block space-y-1.5" :for="fieldId + '-client-id'"><span class="text-xs font-medium">{{ st('Client ID') }}</span><input :id="fieldId + '-client-id'" :value="text('/auth/client_id')" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs" autocomplete="off" spellcheck="false" @input="setOptional('/auth/client_id', ($event.target as HTMLInputElement).value)" /></label>
                <div class="space-y-2">
                  <label class="block space-y-1.5" :for="fieldId + '-client-secret'"><span class="text-xs font-medium">{{ st('Client secret (optional)') }}</span><input :id="fieldId + '-client-secret'" type="password" :value="secretPreserved('/auth/client_secret') ? '' : text('/auth/client_secret')" :placeholder="secretPreserved('/auth/client_secret') ? st('Stored value') : ''" :disabled="disabled || secretPreserved('/auth/client_secret')" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs disabled:opacity-70" autocomplete="new-password" @input="setOptional('/auth/client_secret', ($event.target as HTMLInputElement).value)" /></label>
                  <div v-if="auth.client_secret" class="flex flex-wrap gap-2"><Button v-if="secretPreserved('/auth/client_secret')" type="button" variant="outline" size="sm" :disabled="disabled" @click="setOptional('/auth/client_secret', '')">{{ st('Replace client secret') }}</Button><Button type="button" variant="ghost" size="sm" :disabled="disabled" @click="set('/auth/client_secret', undefined, true)">{{ st('Remove client secret') }}</Button></div>
                </div>
                <div class="space-y-2">
                  <label class="block space-y-1.5" :for="fieldId + '-client-metadata'"><span class="text-xs font-medium">{{ st('Client metadata URL') }}</span><input :id="fieldId + '-client-metadata'" type="url" :value="secretPreserved('/auth/client_metadata_url') ? '' : text('/auth/client_metadata_url')" :placeholder="secretPreserved('/auth/client_metadata_url') ? st('Stored value') : ''" :disabled="disabled || secretPreserved('/auth/client_metadata_url')" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs disabled:opacity-70" autocomplete="off" spellcheck="false" @input="setOptional('/auth/client_metadata_url', ($event.target as HTMLInputElement).value)" /></label>
                  <Button v-if="secretPreserved('/auth/client_metadata_url')" type="button" variant="outline" size="sm" :disabled="disabled" @click="setOptional('/auth/client_metadata_url', '')">{{ st('Replace client metadata URL') }}</Button>
                </div>
              </div>
            </details>
          </div>
          <p v-else-if="authKind === 'bearer_from_store'" class="text-xs leading-relaxed text-muted-foreground">{{ st('Save the connection, then manage its token in the credentials section.') }}</p>
          <McpMapEditor v-if="authKind === 'custom'" :model-value="modelValue" pointer="/auth/headers" :title="st('Authentication headers')" kind="headers" :disabled="disabled" @update:model-value="emit('update:modelValue', $event)" />
          <McpMapEditor :model-value="modelValue" pointer="/endpoint/headers" :title="st('HTTP headers')" kind="headers" :disabled="disabled" @update:model-value="emit('update:modelValue', $event)" />
        </div>
        <div class="flex items-center justify-between gap-3 rounded-lg border border-border/60 p-3"><span class="text-xs font-medium">{{ st('Connection enabled') }}</span><SettingsToggle :model-value="config.enabled !== false" :disabled="disabled" :label="st('Connection enabled')" @update:model-value="set('/enabled', $event)" /></div>
        <details class="rounded-xl border border-border/70 bg-muted/10">
          <summary class="cursor-pointer px-3 py-3 text-xs font-medium">{{ st('Timeouts and tool filters') }}</summary>
          <div class="space-y-4 border-t border-border/60 p-3">
            <div class="grid gap-3 sm:grid-cols-2">
              <label class="space-y-1.5" :for="fieldId + '-connect'"><span class="text-xs font-medium">{{ st('Connect timeout (ms)') }}</span><input :id="fieldId + '-connect'" type="number" min="1" max="300000" :value="value('/timeouts/connect_ms') ?? 20000" class="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm" @input="set('/timeouts/connect_ms', Number(($event.target as HTMLInputElement).value))" /></label>
              <label class="space-y-1.5" :for="fieldId + '-request'"><span class="text-xs font-medium">{{ st('Request timeout (ms)') }}</span><input :id="fieldId + '-request'" type="number" min="1" max="3600000" :value="value('/timeouts/request_ms') ?? 60000" class="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm" @input="set('/timeouts/request_ms', Number(($event.target as HTMLInputElement).value))" /></label>
            </div>
            <div class="grid gap-3 sm:grid-cols-2">
              <label class="space-y-1.5" :for="fieldId + '-include'"><span class="text-xs font-medium">{{ st('Include tool patterns') }}</span><textarea :id="fieldId + '-include'" :value="listText('/tools/include')" rows="3" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs" spellcheck="false" @input="setList('/tools/include', ($event.target as HTMLTextAreaElement).value)" /></label>
              <label class="space-y-1.5" :for="fieldId + '-exclude'"><span class="text-xs font-medium">{{ st('Exclude tool patterns') }}</span><textarea :id="fieldId + '-exclude'" :value="listText('/tools/exclude')" rows="3" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs" spellcheck="false" @input="setList('/tools/exclude', ($event.target as HTMLTextAreaElement).value)" /></label>
            </div>
            <p class="text-xs text-muted-foreground">{{ st('One pattern per line. An empty include list allows all discovered tools.') }}</p>
          </div>
        </details>
      </fieldset>
    </template>
  </div>
</template>
