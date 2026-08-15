<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiRefreshLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import { apiJson } from '../../lib/api'
import { useToastsStore } from '../../stores/toasts'

type PermissionRule = Record<string, unknown>

const toasts = useToastsStore()

const loading = ref(false)
const error = ref('')
const rules = ref<PermissionRule[]>([])

const createBusy = ref(false)
const createError = ref('')
const newAction = ref('')

const canCreate = computed(() => !createBusy.value && newAction.value.trim().length > 0)

const sortedRules = computed(() => [...rules.value].sort((a, b) => String(a.rule_id || '').localeCompare(String(b.rule_id || ''))))

function stringFieldNames(rule: PermissionRule): string[] {
  return Object.keys(rule)
    .filter((key) => key !== 'rule_id' && typeof rule[key] === 'string' && String(rule[key]).trim().length > 0)
}

function displayValue(value: unknown): string {
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const data = await apiJson<PermissionRule[]>('/api/v1/permission-rules')
    rules.value = Array.isArray(data) ? data : []
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    rules.value = []
  } finally {
    loading.value = false
  }
}

async function createRule() {
  if (!canCreate.value) return
  createBusy.value = true
  createError.value = ''
  try {
    await apiJson('/api/v1/permission-rules', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ action: newAction.value.trim() }),
    })
    newAction.value = ''
    toasts.push('success', 'Permission rule created')
    await refresh()
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    createError.value = msg
    toasts.push('error', msg)
  } finally {
    createBusy.value = false
  }
}

async function removeRule(id: string) {
  try {
    await apiJson(`/api/v1/permission-rules/${encodeURIComponent(id)}`, { method: 'DELETE' })
    toasts.push('success', 'Permission rule removed')
    await refresh()
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    toasts.push('error', msg)
  }
}

onMounted(() => {
  void refresh()
})
</script>

<template>
  <div class="space-y-6">
    <div>
      <div class="text-lg font-medium">Permissions</div>
      <div class="mt-1 text-sm text-muted-foreground">Rules the server applies when approving tool and command actions.</div>
    </div>

    <div class="grid gap-3 rounded-lg border border-border bg-muted/10 p-4">
      <div class="text-sm font-medium">Create rule</div>
      <div class="flex items-center gap-2">
        <Input
          v-model="newAction"
          placeholder="action pattern, e.g. Bash(npm run *)"
          :disabled="createBusy"
          class="h-10"
          @keydown.enter="createRule"
        />
        <Button :disabled="!canCreate" @click="createRule">
          {{ createBusy ? 'Creating...' : 'Create' }}
        </Button>
      </div>
      <div v-if="createError" class="text-xs text-destructive break-words">{{ createError }}</div>
    </div>

    <div class="grid gap-3">
      <div v-if="loading" class="text-sm text-muted-foreground">Loading permission rules...</div>
      <div v-else-if="error" class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
        {{ error }}
      </div>
      <div v-else-if="sortedRules.length === 0" class="text-sm text-muted-foreground">No permission rules configured.</div>

      <div v-else class="space-y-2">
        <div
          v-for="rule in sortedRules"
          :key="String(rule.rule_id || '')"
          class="flex items-center justify-between gap-3 rounded-md border border-border/60 bg-background/50 px-3 py-2.5"
        >
          <div class="min-w-0">
            <div class="font-mono text-sm font-semibold break-words">{{ rule.rule_id }}</div>
            <div v-if="stringFieldNames(rule).length" class="mt-0.5 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
              <span v-for="key in stringFieldNames(rule)" :key="key" class="break-all">
                {{ key }}: {{ displayValue(rule[key]) }}
              </span>
            </div>
          </div>

          <ConfirmPopover
            :title="'Remove permission rule?'"
            :description="String(rule.rule_id || '')"
            :confirm-text="'Remove'"
            :cancel-text="'Cancel'"
            variant="destructive"
            @confirm="removeRule(String(rule.rule_id || ''))"
          >
            <Button variant="outline" size="sm" class="shrink-0 text-destructive border-destructive/30 hover:bg-destructive/10">
              Remove
            </Button>
          </ConfirmPopover>
        </div>
      </div>
    </div>

    <div class="flex items-center gap-2">
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? 'Refreshing...' : 'Refresh'"
        :aria-label="loading ? 'Refreshing...' : 'Refresh'"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
      <Button variant="outline" size="sm" :disabled="loading" @click="refresh">
        {{ loading ? 'Refreshing...' : 'Refresh' }}
      </Button>
    </div>
  </div>
</template>
