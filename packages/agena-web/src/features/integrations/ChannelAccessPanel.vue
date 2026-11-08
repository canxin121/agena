<script setup lang="ts">
import { ref, watch, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '@/components/ui/Button.vue'
import Dialog from '@/components/ui/Dialog.vue'
import { integrationApi } from '@/lib/integrationApi'
import { channelPolicyTemplate, parseMcpAllowlist } from './model'
import type {
  IntegrationDetail,
  IntegrationBinding,
  ChannelAccessPolicy,
  ChannelRole,
  ChannelGrant,
  ChannelPolicyUpdate,
} from '@/generated/agenaState'

const props = defineProps<{ source: number; detail: IntegrationDetail; readonly?: boolean }>()
const emit = defineEmits<{ changed: [IntegrationBinding]; editing: [boolean] }>()
const { t } = useI18n()
const copy = <T,>(v: T): T => JSON.parse(JSON.stringify(v)) as T
const roles: ChannelRole[] = ['member', 'operator', 'administrator', 'owner', 'blocked']
const grantRoles: ChannelRole[] = ['member', 'operator', 'administrator', 'owner']
const emptyGrant = (): ChannelGrant => ({
  tools: [],
  mcp_tools: {},
  background_work: false,
  private_memory: false,
  source_workspace: false,
  network_hosts: [],
})
const draft = ref<ChannelAccessPolicy | null>(null),
  pending = ref<ChannelPolicyUpdate | null>(null)
const baseRevision = ref(0),
  busy = ref(false),
  error = ref(''),
  stale = ref(false)
const mcpDrafts = ref<Partial<Record<ChannelRole, string>>>({})
let disposed = false
function open() {
  const value = copy(props.detail.binding.spec.channel_policy || channelPolicyTemplate())
  value.grants ||= {}
  for (const role of grantRoles) value.grants[role] ||= emptyGrant()
  mcpDrafts.value = Object.fromEntries(
    grantRoles.map((role) => [role, JSON.stringify(value.grants?.[role]?.mcp_tools || {}, null, 2)]),
  )
  draft.value = value
  pending.value = null
  baseRevision.value = props.detail.binding.revision
  error.value = ''
  stale.value = false
  emit('editing', true)
}
function close() {
  if (!busy.value) {
    draft.value = null
    pending.value = null
    emit('editing', false)
  }
}
function add() {
  if (draft.value && draft.value.members.length < 128)
    draft.value.members.push({ actor: '', role: 'member', expires_at_ms: null })
}
function lines(role: ChannelRole, field: 'tools' | 'network_hosts', event: Event) {
  const grant = draft.value?.grants?.[role]
  if (grant)
    grant[field] = [
      ...new Set(
        (event.target as HTMLTextAreaElement).value
          .split(/\r?\n/)
          .map((v) => v.trim())
          .filter(Boolean),
      ),
    ]
}
function expiry(index: number, event: Event) {
  if (!draft.value) return
  const text = (event.target as HTMLInputElement).value
  const value = text ? new Date(text).getTime() : null
  if (value === null || Number.isFinite(value)) draft.value.members[index]!.expires_at_ms = value
}
function datetime(ms: number | null | undefined) {
  if (!ms) return ''
  const value = new Date(ms)
  return new Date(ms - value.getTimezoneOffset() * 60000).toISOString().slice(0, 16)
}
async function save() {
  if (!draft.value || busy.value || stale.value) return
  if (!pending.value) {
    try {
      for (const role of grantRoles) {
        draft.value.grants![role]!.mcp_tools = parseMcpAllowlist(mcpDrafts.value[role] || '{}')
      }
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e)
      return
    }
  }
  pending.value ||= {
    expected_revision: baseRevision.value,
    request_key: crypto.randomUUID(),
    policy: copy(draft.value),
  }
  const source = props.source,
    id = props.detail.binding.id
  busy.value = true
  error.value = ''
  try {
    const saved = await integrationApi.access(source, id, pending.value)
    if (disposed || props.detail.binding.id !== id || props.source !== source) return
    draft.value = null
    pending.value = null
    emit('editing', false)
    emit('changed', saved)
  } catch (e) {
    if (!disposed) {
      error.value = e instanceof Error ? e.message : String(e)
      const status = e && typeof e === 'object' && 'status' in e ? e.status : undefined
      stale.value = status === 409
      // Correctable invalid input is known not to have committed. Uncertain
      // network outcomes still retain the frozen request identity and body.
      if (status === 400 || status === 422) pending.value = null
    }
  } finally {
    busy.value = false
  }
}
watch(
  () => props.detail.binding.id,
  () => {
    draft.value = null
    pending.value = null
    error.value = ''
    emit('editing', false)
  },
)
onBeforeUnmount(() => {
  disposed = true
  emit('editing', false)
})
</script>
<template>
  <section
    v-if="detail.binding.spec.mode === 'channel'"
    class="rounded-lg border border-border p-4 space-y-3"
    data-testid="channel-access-panel"
    :data-access-revision="detail.binding.revision"
  >
    <div class="flex flex-wrap items-center justify-between gap-2">
      <h3 class="font-semibold">{{ t('channelAccess.title') }}</h3>
      <Button
        size="sm"
        variant="outline"
        :disabled="readonly || busy"
        @click="open"
        data-testid="channel-access-edit"
        >{{ t('channelAccess.edit') }}</Button
      >
    </div>
    <p class="text-xs text-muted-foreground">{{ t('channelAccess.boundary') }}</p>
    <p v-if="!detail.binding.spec.channel_policy" class="text-sm text-destructive">{{ t('channelAccess.legacy') }}</p>
    <template v-else
      ><p class="break-all text-xs">
        {{ detail.binding.spec.channel_policy.identity.tenant }} /
        {{ detail.binding.spec.channel_policy.identity.account }} ·
        {{ t(`channelAccess.kind_${detail.binding.spec.channel_policy.identity.kind}`) }} ·
        {{ t(`channelAccess.mode_${detail.binding.spec.channel_policy.session_mode || 'per_user'}`) }}
      </p>
      <div class="overflow-x-auto">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-border text-left">
              <th class="py-2 pr-3">{{ t('channelAccess.user') }}</th>
              <th class="py-2 pr-3">{{ t('channelAccess.role') }}</th>
              <th class="py-2">{{ t('integrations.expires') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="member in detail.binding.spec.channel_policy.members"
              :key="member.actor"
              class="border-b border-border/50"
            >
              <td class="py-2 pr-3 break-all">{{ member.actor }}</td>
              <td class="py-2 pr-3">{{ t(`channelAccess.role_${member.role}`) }}</td>
              <td class="py-2 text-xs">
                {{ member.expires_at_ms ? new Date(member.expires_at_ms).toLocaleString() : '—' }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>
    <details v-if="detail.access_audit.length">
      <summary class="cursor-pointer text-sm">
        {{ t('channelAccess.audit') }} · {{ detail.access_audit.length }}
      </summary>
      <p v-for="row in detail.access_audit" :key="row.request_key" class="mt-2 text-xs break-all">
        {{ new Date(row.at_ms).toLocaleString() }} · {{ row.actor }} · {{ t('assistant.revision') }} {{ row.revision }}
      </p>
    </details>
    <p class="text-xs text-muted-foreground">{{ t('channelAccess.commands') }}</p>
    <Dialog
      :open="!!draft"
      :title="t('channelAccess.title')"
      :description="t('channelAccess.confirmHint')"
      @close="close"
      ><form v-if="draft" class="p-4 space-y-4" @submit.prevent="save">
        <fieldset :disabled="busy || !!pending" class="space-y-4 disabled:opacity-70">
          <div class="grid gap-3 sm:grid-cols-2">
            <label class="channel-field"
              >{{ t('channelAccess.tenant')
              }}<input v-model="draft.identity.tenant" :disabled="!!detail.binding.spec.channel_policy" /></label
            ><label class="channel-field"
              >{{ t('channelAccess.account')
              }}<input v-model="draft.identity.account" :disabled="!!detail.binding.spec.channel_policy" /></label
            ><label class="channel-field"
              >{{ t('channelAccess.kind')
              }}<select v-model="draft.identity.kind" :disabled="!!detail.binding.spec.channel_policy">
                <option
                  v-for="kind in ['direct', 'group', 'public_channel', 'private_channel'] as const"
                  :key="kind"
                  :value="kind"
                >
                  {{ t(`channelAccess.kind_${kind}`) }}
                </option>
              </select></label
            ><label class="channel-field"
              >{{ t('channelAccess.context')
              }}<select v-model="draft.session_mode">
                <option value="per_user">{{ t('channelAccess.mode_per_user') }}</option>
                <option value="shared_chat" :disabled="draft.identity.kind === 'direct'">
                  {{ t('channelAccess.mode_shared_chat') }}
                </option>
              </select></label
            ><label class="channel-field"
              >{{ t('channelAccess.thread')
              }}<select v-model="draft.thread_mode">
                <option value="exact">{{ t('channelAccess.exact') }}</option>
                <option value="per_thread" :disabled="draft.identity.kind === 'direct'">
                  {{ t('channelAccess.perThread') }}
                </option>
              </select></label
            ><label class="channel-field"
              >{{ t('channelAccess.trigger')
              }}<select v-model="draft.trigger">
                <option value="direct_or_mention">{{ t('channelAccess.mention') }}</option>
                <option value="every_message">{{ t('channelAccess.every') }}</option>
              </select></label
            ><label class="channel-field"
              >{{ t('channelAccess.daily')
              }}<input type="number" v-model.number="draft.per_user_daily_limit" min="1" max="1000" /></label
            ><label class="flex items-center gap-2 text-sm"
              ><input
                type="checkbox"
                v-model="draft.allow_unlisted_chat"
                :disabled="draft.identity.kind === 'direct'"
              />{{ t('channelAccess.unlisted') }}</label
            >
          </div>
          <div class="space-y-3">
            <h4 class="font-medium">{{ t('channelAccess.members') }}</h4>
            <div
              v-for="(member, index) in draft.members"
              :key="index"
              class="grid gap-2 rounded border border-border p-3 sm:grid-cols-2"
            >
              <label class="channel-field"
                >{{ t('channelAccess.user')
                }}<input v-model="member.actor" :data-testid="`channel-member-${index}`" /></label
              ><label class="channel-field"
                >{{ t('channelAccess.role')
                }}<select v-model="member.role" :data-testid="`channel-role-${index}`">
                  <option v-for="role in roles" :key="role" :value="role">{{ t(`channelAccess.role_${role}`) }}</option>
                </select></label
              ><label class="channel-field"
                >{{ t('integrations.expires')
                }}<input
                  type="datetime-local"
                  :value="datetime(member.expires_at_ms)"
                  @input="expiry(index, $event)" /></label
              ><Button type="button" size="sm" variant="ghost" @click="draft.members.splice(index, 1)">{{
                t('channelAccess.remove')
              }}</Button>
            </div>
            <Button
              type="button"
              size="sm"
              variant="outline"
              :disabled="draft.members.length >= 128"
              @click="add"
              data-testid="channel-member-add"
              >{{ t('channelAccess.add') }}</Button
            >
          </div>
          <details>
            <summary class="cursor-pointer font-medium">{{ t('channelAccess.capabilities') }}</summary>
            <p class="mt-2 text-xs text-muted-foreground">{{ t('channelAccess.capabilityHint') }}</p>
            <div v-for="role in grantRoles" :key="role" class="mt-3 rounded border border-border p-3 space-y-3">
              <h4 class="text-sm font-semibold">{{ t(`channelAccess.role_${role}`) }}</h4>
              <template v-if="draft.grants?.[role]"
                ><div class="flex flex-wrap gap-3 text-xs">
                  <label
                    ><input type="checkbox" v-model="draft.grants[role]!.background_work" />
                    {{ t('channelAccess.work') }}</label
                  ><label
                    ><input type="checkbox" v-model="draft.grants[role]!.private_memory" />
                    {{ t('channelAccess.memory') }}</label
                  ><label
                    ><input type="checkbox" v-model="draft.grants[role]!.source_workspace" />
                    {{ t('channelAccess.project') }}</label
                  >
                </div>
                <label class="channel-field"
                  >{{ t('channelAccess.tools')
                  }}<textarea
                    rows="3"
                    :value="draft.grants[role]!.tools?.join('\n')"
                    @input="lines(role, 'tools', $event)"
                  /></label
                ><label class="channel-field"
                  >{{ t('channelAccess.mcp')
                  }}<textarea
                    rows="3"
                    v-model="mcpDrafts[role]"
                    spellcheck="false"
                    placeholder='{"docs":["search"]}'
                    :data-testid="`channel-mcp-${role}`"
                  /></label
                ><label class="channel-field"
                  >{{ t('channelAccess.network')
                  }}<textarea
                    rows="2"
                    :value="draft.grants[role]!.network_hosts?.join('\n')"
                    @input="lines(role, 'network_hosts', $event)"
                  /></label
              ></template>
            </div>
          </details>
          <label class="flex items-center gap-2 text-sm"
            ><input type="checkbox" v-model="draft.allow_automated_reports" />{{ t('channelAccess.reports') }}</label
          >
        </fieldset>
        <p v-if="error" role="alert" class="text-sm text-destructive break-words">{{ error }}</p>
        <p v-if="stale" class="text-sm">{{ t('channelAccess.stale') }}</p>
        <div class="flex justify-end gap-2">
          <Button type="button" variant="ghost" :disabled="busy" @click="close" data-testid="channel-access-close">{{
            t('assistant.close')
          }}</Button
          ><Button type="submit" :disabled="busy || stale" data-testid="channel-access-save">{{
            t('assistant.confirm')
          }}</Button>
        </div>
      </form></Dialog
    >
  </section>
</template>
<style scoped>
.channel-field {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
  font-size: 0.8rem;
  min-width: 0;
}
.channel-field input,
.channel-field select,
.channel-field textarea {
  border: 1px solid var(--border);
  border-radius: 0.35rem;
  background: var(--background);
  padding: 0.5rem;
  min-width: 0;
  width: 100%;
}
.channel-field input:focus,
.channel-field select:focus,
.channel-field textarea:focus {
  outline: 2px solid var(--ring);
  outline-offset: 1px;
}
</style>
