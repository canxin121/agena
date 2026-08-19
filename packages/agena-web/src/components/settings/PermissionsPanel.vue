<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiDeleteBinLine, RiRefreshLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'
import { useToastsStore } from '@/stores/toasts'
import { settingsText as st } from '@/i18n/settingsText'

type PermissionMode = 'allow' | 'auto' | 'ask' | 'deny'
type PermissionScope = 'workspace' | 'global'

type PermissionRule = {
  id: number
  action_key: string
  subject_kind: string
  tool_name?: string | null
  qualifier?: string | null
  path_access_kind?: string | null
  target_path?: string | null
  network_target?: string | null
  mode: PermissionMode
  scope: string
  source: string
  reason?: string | null
  revoked_at?: string | null
  created_at: string
  updated_at: string
}

type PermissionRulePage = {
  items?: PermissionRule[]
  page?: {
    limit: number
    returned: number
    has_more: boolean
    next_cursor?: string | null
  }
}

const toasts = useToastsStore()

const loading = ref(false)
const error = ref('')
const rules = ref<PermissionRule[]>([])
const hasMore = ref(false)

const createBusy = ref(false)
const createError = ref('')
const newToolName = ref('')
const newQualifier = ref('')
const newMode = ref<PermissionMode>('ask')
const newScope = ref<PermissionScope>('workspace')

const modeOptions = [
  { value: 'allow', label: st('Allow'), description: st('Approve matching tool calls.') },
  { value: 'auto', label: st('Auto'), description: st('Let Agena evaluate matching calls automatically.') },
  { value: 'ask', label: st('Ask'), description: st('Request confirmation before running.') },
  { value: 'deny', label: st('Deny'), description: st('Block matching tool calls.') },
]

const scopeOptions = [
  { value: 'workspace', label: st('Workspace'), description: st('Apply only to this workspace.') },
  { value: 'global', label: st('Global'), description: st('Apply across all workspaces.') },
]

const canCreate = computed(() => !createBusy.value && newToolName.value.trim().length > 0)

const sortedRules = computed(() =>
  [...rules.value].sort((a, b) => {
    const revokedOrder = Number(Boolean(a.revoked_at)) - Number(Boolean(b.revoked_at))
    return revokedOrder || b.id - a.id
  }),
)

function ruleTitle(rule: PermissionRule): string {
  if (rule.subject_kind === 'tool' && rule.tool_name) {
    return rule.qualifier
      ? st('{tool_name} · {qualifier}', { tool_name: rule.tool_name, qualifier: rule.qualifier })
      : rule.tool_name
  }
  if (rule.subject_kind === 'path_access') {
    return st('{path} · {target_path}', { path: rule.path_access_kind || 'path', target_path: rule.target_path || '' })
  }
  if (rule.subject_kind === 'network_access') return rule.network_target || 'Network access'
  return rule.action_key
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const data = await apiJson<PermissionRulePage>('/api/v1/permission-rules?limit=200')
    rules.value = Array.isArray(data?.items) ? data.items : []
    hasMore.value = data?.page?.has_more === true
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    rules.value = []
    hasMore.value = false
  } finally {
    loading.value = false
  }
}

async function createRule() {
  if (!canCreate.value) return
  createBusy.value = true
  createError.value = ''
  try {
    const qualifier = newQualifier.value.trim()
    await apiJson('/api/v1/permission-rules', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        subject_kind: 'tool',
        tool_name: newToolName.value.trim(),
        ...(qualifier ? { qualifier } : {}),
        mode: newMode.value,
        scope: newScope.value,
      }),
    })
    newToolName.value = ''
    newQualifier.value = ''
    toasts.push('success', st('Permission rule created'))
    await refresh()
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    createError.value = message
    toasts.push('error', message)
  } finally {
    createBusy.value = false
  }
}

async function removeRule(id: number) {
  try {
    await apiJson(`/api/v1/permission-rules/${id}`, { method: 'DELETE' })
    toasts.push('success', st('Permission rule removed'))
    await refresh()
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  }
}

onMounted(() => {
  void refresh()
})
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-start justify-between gap-3">
      <div>
        <div class="text-lg font-medium">{{ $st('Permissions') }}</div>
        <div class="mt-1 text-sm text-muted-foreground">
          {{ $st('Persistent rules Agena applies to tool approval decisions.') }}
        </div>
      </div>
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? $st('Refreshing permission rules') : $st('Refresh permission rules')"
        :aria-label="loading ? $st('Refreshing permission rules') : $st('Refresh permission rules')"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <div class="grid gap-3 border-b border-border/60 pb-4">
      <div class="text-sm font-medium">{{ $st('Create tool rule') }}</div>
      <div class="grid gap-3 sm:grid-cols-2">
        <label class="grid gap-1.5">
          <span class="text-xs text-muted-foreground">{{ $st('Tool name') }}</span>
          <Input
            v-model="newToolName"
            placeholder="shell"
            :disabled="createBusy"
            class="h-10 font-mono"
            @keydown.enter="createRule"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-xs text-muted-foreground">{{ $st('Qualifier') }}</span>
          <Input
            v-model="newQualifier"
            :placeholder="$st('Optional command or operation')"
            :disabled="createBusy"
            class="h-10 font-mono"
            @keydown.enter="createRule"
          />
        </label>
      </div>
      <div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] sm:items-end">
        <label class="grid gap-1.5">
          <span class="text-xs text-muted-foreground">{{ $st('Mode') }}</span>
          <OptionPicker
            v-model="newMode"
            :options="modeOptions"
            :title="$st('Permission mode')"
            :include-empty="false"
            :disabled="createBusy"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-xs text-muted-foreground">{{ $st('Scope') }}</span>
          <OptionPicker
            v-model="newScope"
            :options="scopeOptions"
            :title="$st('Permission scope')"
            :include-empty="false"
            :disabled="createBusy"
          />
        </label>
        <Button class="h-10" :disabled="!canCreate" @click="createRule">
          {{ createBusy ? 'Creating...' : $st('Create rule') }}
        </Button>
      </div>
      <div v-if="createError" class="break-words text-xs text-destructive">{{ createError }}</div>
    </div>

    <div class="grid gap-3">
      <div v-if="loading" class="text-sm text-muted-foreground">{{ $st('Loading permission rules...') }}</div>
      <div
        v-else-if="error"
        class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        {{ error }}
      </div>
      <div v-else-if="sortedRules.length === 0" class="text-sm text-muted-foreground">
        {{ $st('No permission rules configured.') }}
      </div>

      <div v-else class="space-y-2">
        <div
          v-for="rule in sortedRules"
          :key="rule.id"
          class="flex items-center justify-between gap-3 rounded-md border border-border/60 bg-background/50 px-3 py-2.5"
          :class="rule.revoked_at ? 'opacity-60' : ''"
        >
          <div class="min-w-0">
            <div class="break-words font-mono text-sm font-semibold">{{ ruleTitle(rule) }}</div>
            <div class="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
              <span>#{{ rule.id }}</span>
              <span>{{ $st('mode:') }} {{ rule.mode }}</span>
              <span>{{ $st('scope:') }} {{ rule.scope }}</span>
              <span>{{ $st('source:') }} {{ rule.source }}</span>
              <span v-if="rule.revoked_at" class="text-destructive">{{ $st('revoked') }}</span>
            </div>
          </div>

          <ConfirmPopover
            :title="$st('Remove permission rule?')"
            :description="ruleTitle(rule)"
            confirm-text="Remove"
            cancel-text="Cancel"
            variant="destructive"
            @confirm="removeRule(rule.id)"
          >
            <IconButton
              variant="outline"
              size="sm"
              class="shrink-0 text-destructive"
              :tooltip="$st('Remove rule')"
              :aria-label="$st('Remove rule')"
            >
              <RiDeleteBinLine class="h-4 w-4" />
            </IconButton>
          </ConfirmPopover>
        </div>
      </div>
      <div v-if="hasMore" class="text-xs text-muted-foreground">
        {{ $st('More than 200 rules exist. Refine or manage older rules through the CLI.') }}
      </div>
    </div>
  </div>
</template>
