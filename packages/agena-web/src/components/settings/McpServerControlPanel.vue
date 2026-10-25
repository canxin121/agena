<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import { RiRefreshLine } from '@remixicon/vue'

import SettingsSaveBar from '@/components/settings/SettingsSaveBar.vue'
import SettingsCard from './SettingsCard.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import { apiJson } from '@/lib/api'
import { confirmAction } from '@/lib/appConfirm'
import { settingsText as st } from '@/i18n/settingsText'

type McpOAuthStatus = {
  configured: boolean
  passwordConfigured: boolean
  fallbackToUiPassword: boolean
  ready: boolean
  authorizationServerKind: string
  registrationMethods: string[]
  tokenEndpointAuthMethods: string[]
  pkceMethods: string[]
  oidcSupported: boolean
  warnings: string[]
  scope: string
  issuer: string
  authorizationEndpoint: string
  tokenEndpoint: string
  revocationEndpoint: string
  protectedResourceMetadata: string
  authorizationServerMetadata: string
}

type McpServerControl = {
  enabled: boolean
  publicUrl: string | null
  oauthIssuerUrl: string | null
  resourceUrl: string
  ready: boolean
  warnings: string[]
  generatedOauthPassword?: string | null
  oauth: McpOAuthStatus
}

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const control = ref<McpServerControl | null>(null)
const enabled = ref(false)
const publicUrl = ref('')
const oauthIssuerUrl = ref('')
const oauthPassword = ref('')
const copiedEndpoint = ref('')

const oauth = computed(() => control.value?.oauth || null)
const controlDirty = computed(() => {
  const value = control.value
  if (!value) return false
  return (
    enabled.value !== value.enabled ||
    publicUrl.value.trim() !== String(value.publicUrl || '') ||
    oauthIssuerUrl.value.trim() !== String(value.oauthIssuerUrl || '')
  )
})
const mcpSurfaceSummary = computed(() => {
  const value = control.value
  if (!value) return ''
  if (!value.enabled) {
    return st('The /mcp endpoint is disabled. The management API remains available so it can be enabled again.')
  }
  return st(
    'The /mcp endpoint is available. Discovery is public and every tool call requires an OAuth access token unless the tool itself is marked safe for anonymous use.',
  )
})

function errorMessage(value: unknown): string {
  return value instanceof Error ? value.message : String(value)
}

function applyControl(value: McpServerControl, preserveDraft = false) {
  const keepDraft = preserveDraft && controlDirty.value
  control.value = value
  if (keepDraft) return
  enabled.value = value.enabled
  publicUrl.value = value.publicUrl || ''
  oauthIssuerUrl.value = value.oauthIssuerUrl || ''
}

async function copyEndpoint(label: string, value: string | undefined) {
  if (!value || !navigator.clipboard) return
  try {
    await navigator.clipboard.writeText(value)
    copiedEndpoint.value = label
    window.setTimeout(() => {
      if (copiedEndpoint.value === label) copiedEndpoint.value = ''
    }, 1500)
  } catch {
    // Clipboard access is optional (for example in an insecure local preview).
  }
}

async function refresh() {
  if (loading.value || saving.value) return
  loading.value = true
  error.value = ''
  try {
    applyControl(await apiJson<McpServerControl>('/api/v1/server/mcp'), true)
  } catch (err) {
    error.value = errorMessage(err)
  } finally {
    loading.value = false
  }
}

async function updateControl(body: {
  enabled?: boolean
  publicUrl?: string | null
  oauthIssuerUrl?: string | null
}) {
  if (!control.value || saving.value) return
  saving.value = true
  error.value = ''
  try {
    const next = await apiJson<McpServerControl>('/api/v1/server/mcp', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        enabled: body.enabled ?? control.value.enabled,
        ...(Object.prototype.hasOwnProperty.call(body, 'publicUrl') ? { publicUrl: body.publicUrl } : {}),
        ...(Object.prototype.hasOwnProperty.call(body, 'oauthIssuerUrl')
          ? { oauthIssuerUrl: body.oauthIssuerUrl }
          : {}),
      }),
    })
    applyControl(next)
  } catch (err) {
    error.value = errorMessage(err)
  } finally {
    saving.value = false
  }
}

function toggleEnabled() {
  enabled.value = !enabled.value
}

async function saveControlDraft() {
  await updateControl({
    enabled: enabled.value,
    publicUrl: publicUrl.value.trim() || null,
    oauthIssuerUrl: oauthIssuerUrl.value.trim() || null,
  })
}

function discardControlDraft() {
  if (control.value) applyControl(control.value)
}
async function setPassword() {
  const password = oauthPassword.value
  if (!password.trim() || saving.value) return
  saving.value = true
  error.value = ''
  try {
    const next = await apiJson<McpServerControl>('/api/v1/server/mcp/oauth/password', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ password }),
    })
    oauthPassword.value = ''
    applyControl(next, true)
  } catch (err) {
    error.value = errorMessage(err)
  } finally {
    saving.value = false
  }
}

async function clearPassword() {
  if (saving.value) return
  saving.value = true
  error.value = ''
  try {
    const next = await apiJson<McpServerControl>('/api/v1/server/mcp/oauth/password', { method: 'DELETE' })
    oauthPassword.value = ''
    applyControl(next, true)
  } catch (err) {
    error.value = errorMessage(err)
  } finally {
    saving.value = false
  }
}

async function canLeave() {
  if (saving.value) return false
  if (!controlDirty.value && !oauthPassword.value) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}
onBeforeRouteUpdate((to, from) => {
  if (to.path === from.path && to.query.view === from.query.view) return true
  return canLeave()
})
onBeforeRouteLeave(canLeave)
defineExpose({ refresh })
onMounted(() => {
  void refresh()
})
</script>

<template>
  <div class="min-w-0 space-y-4">
    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <div v-if="loading && !control" class="text-sm text-muted-foreground">
      {{ $st('Loading MCP server status...') }}
    </div>
    <div
      v-else-if="!control"
      class="flex flex-wrap items-center gap-3 rounded-xl border border-border/60 p-4 text-sm text-muted-foreground"
    >
      {{ $st('MCP server status is unavailable.')
      }}<Button variant="outline" size="sm" @click="refresh()">{{ $st('Refresh') }}</Button>
    </div>

    <fieldset v-else :disabled="loading || saving" class="min-w-0 space-y-4">
      <SettingsCard
        :title="$st('MCP surface')"
        :description="
          $st(
            'Manage the live MCP surface served by the connected Agena server process. Web and TUI are control clients; they do not run a second MCP server.',
          )
        "
      >
        <template #actions
          ><IconButton
            variant="outline"
            size="sm"
            :tooltip="$st('Refresh MCP server status')"
            :aria-label="$st('Refresh MCP server status')"
            :disabled="loading || saving"
            @click="refresh()"
            ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" /></IconButton
        ></template>
        <div class="mb-3 flex flex-wrap items-center justify-between gap-3">
          <div>
            <div class="mt-1 text-xs text-muted-foreground">
              {{ mcpSurfaceSummary }}
            </div>
          </div>
          <Button :variant="enabled ? 'outline' : 'default'" :disabled="saving" @click="toggleEnabled">
            {{ enabled ? $st('Disable MCP') : $st('Enable MCP') }}
          </Button>
        </div>

        <div
          class="rounded-md border px-3 py-3 text-xs"
          :class="control.ready ? 'border-emerald-500/30 bg-emerald-500/5' : 'border-amber-500/30 bg-amber-500/5'"
        >
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="font-medium">
              {{ control.ready ? $st('MCP connection ready') : $st('MCP connection needs attention') }}
            </div>
            <code class="break-all">{{ control.resourceUrl }}</code>
          </div>
          <ul v-if="control.warnings.length" class="mt-2 list-disc space-y-1 pl-4 text-muted-foreground">
            <li v-for="warning in control.warnings" :key="warning">{{ warning }}</li>
          </ul>
          <div v-else class="mt-1 text-muted-foreground">
            {{ $st('The live Agena server is ready to serve the selected MCP mode.') }}
          </div>
        </div>
      </SettingsCard>
      <SettingsCard :title="$st('Connection addresses')">
        <div class="grid min-w-0 gap-4 xl:grid-cols-2">
          <div class="min-w-0 space-y-2">
            <label for="mcp-public-url" class="text-sm font-medium">{{ $st('Public MCP resource URL') }}</label>
            <div>
              <Input
                id="mcp-public-url"
                v-model="publicUrl"
                class="font-mono text-xs"
                placeholder="https://your-domain.example/mcp or https://tunnel-service.../v1/mcp/tunnel_id"
                :disabled="saving"
              />
            </div>
            <details class="text-xs leading-relaxed text-muted-foreground">
              <summary class="cursor-pointer">{{ $st('Details') }}</summary>
              <div class="mt-2">
                {{ $st('Enter the canonical HTTPS MCP resource. Secure MCP Tunnel URLs may include the full') }}
                <code>/v1/mcp/tunnel_id</code>
                {{
                  $st(
                    'path and must be kept intact. Leaving this empty keeps the listener-local URL; request Host and forwarded headers are never trusted to define OAuth identity. Current effective resource:',
                  )
                }}
                <code class="break-all">{{ control.resourceUrl }}</code>
              </div>
            </details>
          </div>

          <div class="min-w-0 space-y-2">
            <label for="mcp-issuer-url" class="text-sm font-medium">{{ $st('OAuth issuer URL') }}</label>
            <div>
              <Input
                id="mcp-issuer-url"
                v-model="oauthIssuerUrl"
                class="font-mono text-xs"
                placeholder="https://auth.your-domain.example"
                :disabled="saving"
              />
            </div>
            <details class="text-xs leading-relaxed text-muted-foreground">
              <summary class="cursor-pointer">{{ $st('Details') }}</summary>
              <div class="mt-2">
                {{
                  $st('This is the stable authorization-server identity placed in OAuth discovery, signed access-token')
                }}
                <code>iss</code>
                {{
                  $st(
                    'claims, and the RFC 9207 authorization callback. Use a canonical public HTTPS URL. A path is supported only when the reverse proxy preserves that prefix and routes the matching path-aware RFC 8414 well-known URL. Leave it empty when OAuth and MCP use the same domain; Agena derives the resource origin. Request headers are never trusted.',
                  )
                }}
              </div>
            </details>
          </div>
        </div>
      </SettingsCard>
      <SettingsCard
        :title="$st('OAuth authorization')"
        :description="
          $st(
            'ChatGPT discovers these endpoints from the MCP server. Do not paste ChatGPT client secrets here. Client metadata documents (CIMD) and dynamic client registration are both supported.',
          )
        "
      >
        <div
          v-if="control?.generatedOauthPassword"
          class="mb-3 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-amber-800 dark:text-amber-200"
        >
          {{
            $st(
              'Agena generated an OAuth authorization password because none was configured. Copy it now; it is reported only once.',
            )
          }}
          <code class="mt-1 block select-all break-all">{{ control.generatedOauthPassword }}</code>
        </div>
        <div class="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">{{ $st('Password status') }}</div>
            <div class="mt-1 font-medium">
              {{
                oauth?.passwordConfigured
                  ? $st('MCP-specific password configured')
                  : oauth?.fallbackToUiPassword
                    ? $st('Using server UI password')
                    : $st('Not configured')
              }}
            </div>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">{{ $st('Scope') }}</div>
            <code class="mt-1 block">{{ oauth?.scope || 'agena:tools' }}</code>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">{{ $st('OAuth readiness') }}</div>
            <div class="mt-1 font-medium">
              {{ oauth?.ready ? $st('ready') : $st('not ready') }} · {{ oauth?.authorizationServerKind || '—' }}
            </div>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">{{ $st('Client registration') }}</div>
            <div class="mt-1 font-medium">{{ oauth?.registrationMethods.join(' / ') || '—' }}</div>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">{{ $st('Token client auth / PKCE') }}</div>
            <div class="mt-1 font-medium">
              {{ oauth?.tokenEndpointAuthMethods.join(' / ') || '—' }} ·
              {{ oauth?.pkceMethods.join(' / ') || '—' }}
            </div>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">OIDC</div>
            <div class="mt-1 font-medium">{{ oauth?.oidcSupported ? $st('supported') : $st('not advertised') }}</div>
          </div>
        </div>
        <ul v-if="oauth?.warnings.length" class="list-disc space-y-1 pl-4 text-xs text-amber-700 dark:text-amber-300">
          <li v-for="warning in oauth.warnings" :key="warning">{{ warning }}</li>
        </ul>
        <div class="mt-4 flex flex-col gap-2 sm:flex-row sm:flex-wrap">
          <Input
            v-model="oauthPassword"
            :aria-label="$st('New MCP OAuth password')"
            type="password"
            autocomplete="new-password"
            class="min-w-0 sm:min-w-40 sm:flex-1 font-mono text-xs"
            :placeholder="$st('New MCP OAuth password')"
            :disabled="saving"
          />
          <Button class="shrink-0" :disabled="saving || !oauthPassword.trim()" @click="setPassword">{{
            $st('Set password')
          }}</Button>
          <Button class="shrink-0" variant="outline" :disabled="saving" @click="clearPassword">{{
            $st('Clear')
          }}</Button>
        </div>
      </SettingsCard>

      <details v-if="oauth" class="min-w-0 rounded-xl border border-border/60 bg-card/30 p-4">
        <summary class="cursor-pointer text-sm font-medium">{{ $st('OAuth discovery endpoints') }}</summary>
        <dl class="mt-3 grid gap-2 text-xs">
          <div
            v-for="entry in [
              ['Protected resource metadata', oauth?.protectedResourceMetadata],
              ['Authorization server metadata', oauth?.authorizationServerMetadata],
              ['Authorization endpoint', oauth?.authorizationEndpoint],
              ['Token endpoint', oauth?.tokenEndpoint],
              ['Revocation endpoint', oauth?.revocationEndpoint],
            ]"
            :key="entry[0]"
            class="grid gap-1 sm:grid-cols-[13rem_minmax(0,1fr)]"
          >
            <dt class="text-muted-foreground">{{ entry[0] }}</dt>
            <dd class="flex min-w-0 items-start gap-2">
              <code class="min-w-0 flex-1 break-all">{{ entry[1] || '—' }}</code>
              <Button
                v-if="entry[1]"
                class="shrink-0"
                size="sm"
                variant="ghost"
                @click="copyEndpoint(entry[0], entry[1])"
              >
                {{ copiedEndpoint === entry[0] ? $st('Copied') : $st('Copy') }}
              </Button>
            </dd>
          </div>
        </dl>
      </details>
    </fieldset>
    <SettingsSaveBar
      v-if="controlDirty || saving"
      :dirty="controlDirty"
      :saving="saving"
      :disabled="loading"
      :error="error"
      :save-label="$st('Save MCP configuration')"
      sticky
      @save="saveControlDraft"
      @discard="discardControlDraft"
    />
  </div>
</template>
