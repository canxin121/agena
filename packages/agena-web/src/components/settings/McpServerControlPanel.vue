<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiRefreshLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'

type McpToolExposure = 'read_only' | 'all_non_interactive'
type McpAuthMode = 'none' | 'oauth' | 'mixed'
type McpAnonymousAccess = 'none' | 'read_only'
type McpClientRegistration = 'cimd_only' | 'cimd_and_dcr'

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
  registrationEndpoint?: string
  revocationEndpoint: string
  protectedResourceMetadata: string
  authorizationServerMetadata: string
}

async function saveOAuthIssuerUrl() {
  const value = oauthIssuerUrl.value.trim()
  await updateControl({ oauthIssuerUrl: value || null })
}

async function saveToolExposure() {
  await updateControl({ toolExposure: toolExposure.value })
}

type McpServerControl = {
  enabled: boolean
  authEnabled: boolean
  authMode: McpAuthMode
  anonymousAccess: McpAnonymousAccess
  publicUrl: string | null
  oauthIssuerUrl: string | null
  toolExposure: McpToolExposure
  clientRegistration: McpClientRegistration
  resourceUrl: string
  ready: boolean
  warnings: string[]
  oauth?: McpOAuthStatus
}

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const control = ref<McpServerControl | null>(null)
const publicUrl = ref('')
const oauthIssuerUrl = ref('')
const authMode = ref<McpAuthMode>('none')
const anonymousAccess = ref<McpAnonymousAccess>('none')
const toolExposure = ref<McpToolExposure>('read_only')
const clientRegistration = ref<McpClientRegistration>('cimd_only')
const oauthPassword = ref('')
const copiedEndpoint = ref('')

const oauth = computed(() => control.value?.oauth || null)
const authModeOptions = [
  {
    value: 'none',
    label: 'No authentication',
    description: 'Anonymous MCP. OAuth discovery endpoints are hidden and every exposed tool declares noauth.',
  },
  {
    value: 'oauth',
    label: 'Full OAuth',
    description: 'Require a valid OAuth bearer token for initialize, discovery, tool listing, and every tool call.',
  },
  {
    value: 'mixed',
    label: 'Mixed auth (recommended for ChatGPT)',
    description:
      'Keep initialize and tool discovery public; tool calls remain OAuth-protected unless explicitly opted into anonymous access.',
  },
]
const anonymousAccessOptions = [
  {
    value: 'none',
    label: 'No anonymous tool calls (recommended)',
    description: 'ChatGPT can initialize and discover tools before signing in, but every tool call requires OAuth.',
  },
  {
    value: 'read_only',
    label: 'Anonymous read-only tools',
    description:
      'High risk: permission-contract read-only tools can run without OAuth and may expose private workspace, filesystem, configuration, or diagnostic data.',
  },
]
const clientRegistrationOptions = [
  {
    value: 'cimd_only',
    label: 'CIMD only (recommended)',
    description:
      'Accept OpenAI ChatGPT Client ID Metadata Documents and keep public Dynamic Client Registration disabled.',
  },
  {
    value: 'cimd_and_dcr',
    label: 'CIMD + Dynamic Client Registration',
    description: 'Compatibility mode for older OAuth clients. This exposes an unauthenticated registration endpoint.',
  },
]

const toolExposureOptions = [
  {
    value: 'read_only',
    label: 'Read-only tools (recommended)',
    description:
      'Expose only non-interactive tools whose permission contract has no writes, shell, mutation, network, or task authority.',
  },
  {
    value: 'all_non_interactive',
    label: 'All non-interactive tools',
    description: 'Also expose tools that can write files, run shell commands, mutate state, or access the network.',
  },
]

function errorMessage(value: unknown): string {
  return value instanceof Error ? value.message : String(value)
}

function applyControl(value: McpServerControl) {
  control.value = value
  publicUrl.value = value.publicUrl || ''
  oauthIssuerUrl.value = value.oauthIssuerUrl || ''
  authMode.value = value.authMode || (value.authEnabled ? 'oauth' : 'none')
  anonymousAccess.value = value.anonymousAccess || 'none'
  toolExposure.value = value.toolExposure || 'read_only'
  clientRegistration.value = value.clientRegistration || 'cimd_only'
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
  loading.value = true
  error.value = ''
  try {
    applyControl(await apiJson<McpServerControl>('/api/v1/server/mcp'))
  } catch (err) {
    error.value = errorMessage(err)
  } finally {
    loading.value = false
  }
}

async function updateControl(body: {
  enabled?: boolean
  authMode?: McpAuthMode
  anonymousAccess?: McpAnonymousAccess
  publicUrl?: string | null
  oauthIssuerUrl?: string | null
  toolExposure?: McpToolExposure
  clientRegistration?: McpClientRegistration
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
        authMode: body.authMode ?? control.value.authMode,
        anonymousAccess: body.anonymousAccess ?? control.value.anonymousAccess,
        ...(Object.prototype.hasOwnProperty.call(body, 'publicUrl') ? { publicUrl: body.publicUrl } : {}),
        ...(Object.prototype.hasOwnProperty.call(body, 'oauthIssuerUrl')
          ? { oauthIssuerUrl: body.oauthIssuerUrl }
          : {}),
        ...(body.toolExposure ? { toolExposure: body.toolExposure } : {}),
        ...(body.clientRegistration ? { clientRegistration: body.clientRegistration } : {}),
      }),
    })
    applyControl(next)
  } catch (err) {
    error.value = errorMessage(err)
  } finally {
    saving.value = false
  }
}

async function toggleEnabled() {
  if (!control.value) return
  await updateControl({ enabled: !control.value.enabled })
}

async function saveAuthMode() {
  await updateControl({ authMode: authMode.value })
}

async function saveAnonymousAccess() {
  await updateControl({ anonymousAccess: anonymousAccess.value })
}

async function saveClientRegistration() {
  await updateControl({ clientRegistration: clientRegistration.value })
}

async function savePublicUrl() {
  const value = publicUrl.value.trim()
  await updateControl({ publicUrl: value || null })
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
    applyControl(next)
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
    applyControl(next)
  } catch (err) {
    error.value = errorMessage(err)
  } finally {
    saving.value = false
  }
}

onMounted(() => {
  void refresh()
})
</script>

<template>
  <section class="grid gap-4 rounded-lg border border-border/60 bg-background/30 p-4 lg:p-5">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <div class="text-base font-medium">Agena MCP Server</div>
        <div class="mt-1 max-w-3xl text-sm text-muted-foreground">
          Manage the live MCP surface served by the connected Agena server process. Web and TUI are control clients;
          they do not run a second MCP server.
        </div>
      </div>
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? 'Refreshing MCP server status' : 'Refresh MCP server status'"
        :aria-label="loading ? 'Refreshing MCP server status' : 'Refresh MCP server status'"
        :disabled="loading || saving"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <div v-if="loading && !control" class="text-sm text-muted-foreground">Loading MCP server status...</div>
    <div v-else-if="!control" class="text-sm text-muted-foreground">MCP server status is unavailable.</div>

    <template v-else>
      <div
        class="flex flex-wrap items-center justify-between gap-3 rounded-md border border-border/60 bg-background/50 px-3 py-3"
      >
        <div>
          <div class="text-sm font-medium">MCP surface</div>
          <div class="mt-1 text-xs text-muted-foreground">
            {{ control.enabled ? 'The /mcp endpoint is available.' : 'The /mcp endpoint is disabled.' }}
            {{
              control.enabled
                ? control.authMode === 'oauth'
                  ? ' Full OAuth protects the complete MCP transport.'
                  : control.authMode === 'mixed'
                    ? control.anonymousAccess === 'read_only'
                      ? ' Mixed auth keeps discovery and opted-in read-only tools public.'
                      : ' Mixed auth keeps discovery public and protects every tool call.'
                    : ' MCP calls are anonymous.'
                : ''
            }}
            The management API remains available so it can be enabled again.
          </div>
        </div>
        <Button :variant="control.enabled ? 'outline' : 'default'" :disabled="saving" @click="toggleEnabled">
          {{ control.enabled ? 'Disable MCP' : 'Enable MCP' }}
        </Button>
      </div>

      <div
        class="rounded-md border px-3 py-3 text-xs"
        :class="control.ready ? 'border-emerald-500/30 bg-emerald-500/5' : 'border-amber-500/30 bg-amber-500/5'"
      >
        <div class="flex flex-wrap items-center justify-between gap-2">
          <div class="font-medium">{{ control.ready ? 'MCP connection ready' : 'MCP connection needs attention' }}</div>
          <code>{{ control.resourceUrl }}</code>
        </div>
        <ul v-if="control.warnings.length" class="mt-2 list-disc space-y-1 pl-4 text-muted-foreground">
          <li v-for="warning in control.warnings" :key="warning">{{ warning }}</li>
        </ul>
        <div v-else class="mt-1 text-muted-foreground">
          The live Agena server is ready to serve the selected MCP mode.
        </div>
      </div>

      <div class="grid gap-2">
        <div class="text-sm font-medium">Public MCP resource URL</div>
        <div class="flex flex-col gap-2 sm:flex-row">
          <Input
            v-model="publicUrl"
            class="font-mono text-xs"
            placeholder="https://your-domain.example/mcp or https://tunnel-service.../v1/mcp/tunnel_id"
            :disabled="saving"
          />
          <Button class="shrink-0" variant="outline" :disabled="saving" @click="savePublicUrl">Save URL</Button>
        </div>
        <div class="text-xs text-muted-foreground">
          Enter the canonical HTTPS MCP resource. Secure MCP Tunnel URLs may include the full
          <code>/v1/mcp/tunnel_id</code> path and must be kept intact. Leaving this empty keeps the listener-local URL;
          request Host and forwarded headers are never trusted to define OAuth identity. Current effective resource:
          <code class="break-all">{{ control.resourceUrl }}</code>
        </div>
      </div>

      <div class="grid gap-2">
        <div class="text-sm font-medium">OAuth issuer URL</div>
        <div class="flex flex-col gap-2 sm:flex-row">
          <Input
            v-model="oauthIssuerUrl"
            class="font-mono text-xs"
            placeholder="https://auth.your-domain.example"
            :disabled="saving"
          />
          <Button class="shrink-0" variant="outline" :disabled="saving" @click="saveOAuthIssuerUrl">Save issuer</Button>
        </div>
        <div class="text-xs text-muted-foreground">
          This is the stable authorization-server identity placed in OAuth discovery, signed access-token
          <code>iss</code> claims, and the RFC 9207 authorization callback. Use a canonical public HTTPS URL. A path is
          supported only when the reverse proxy preserves that prefix and routes the matching path-aware RFC 8414
          well-known URL. Leave it empty when OAuth and MCP use the same domain; Agena derives the resource origin.
          Request headers are never trusted.
        </div>
      </div>

      <div class="grid gap-2">
        <div class="text-sm font-medium">Tool exposure</div>
        <div class="flex flex-col gap-2 sm:flex-row sm:items-end">
          <OptionPicker
            v-model="toolExposure"
            class="min-w-0 flex-1"
            :options="toolExposureOptions"
            :include-empty="false"
            title="MCP tool exposure"
            :disabled="saving"
          />
          <Button class="shrink-0" variant="outline" :disabled="saving" @click="saveToolExposure">Save exposure</Button>
        </div>
        <div
          v-if="toolExposure === 'all_non_interactive'"
          class="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-amber-800 dark:text-amber-200"
        >
          High-risk mode: ChatGPT may invoke non-interactive tools that write files, execute shell commands, mutate
          state, or access external networks. Interactive and autonomous task tools remain hidden.
        </div>
        <div v-else class="text-xs text-muted-foreground">
          Recommended for public connectors: only tools proven read-only by Agena's permission contract are exposed.
        </div>
      </div>

      <div class="grid gap-2">
        <div class="text-sm font-medium">MCP authentication mode</div>
        <div class="flex flex-col gap-2 sm:flex-row sm:items-end">
          <OptionPicker
            v-model="authMode"
            class="min-w-0 flex-1"
            :options="authModeOptions"
            :include-empty="false"
            title="MCP authentication mode"
            :disabled="saving"
          />
          <Button class="shrink-0" variant="outline" :disabled="saving" @click="saveAuthMode">Save auth mode</Button>
        </div>
        <div
          v-if="authMode === 'none'"
          class="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-amber-800 dark:text-amber-200"
        >
          High-risk mode: every exposed tool is anonymous. Anyone who can reach the public MCP URL can invoke the
          exposed catalog without an Agena login.
        </div>
        <div v-else-if="authMode === 'mixed'" class="text-xs text-muted-foreground">
          ChatGPT can initialize and discover tools without signing in. Tool calls remain OAuth-protected unless the
          anonymous-access policy below explicitly opts in a class of tools.
        </div>
      </div>

      <div v-if="authMode === 'mixed'" class="grid gap-2">
        <div class="text-sm font-medium">Anonymous tool access in mixed mode</div>
        <div class="flex flex-col gap-2 sm:flex-row sm:items-end">
          <OptionPicker
            v-model="anonymousAccess"
            class="min-w-0 flex-1"
            :options="anonymousAccessOptions"
            :include-empty="false"
            title="Mixed-auth anonymous tool access"
            :disabled="saving"
          />
          <Button class="shrink-0" variant="outline" :disabled="saving" @click="saveAnonymousAccess"
            >Save anonymous access</Button
          >
        </div>
        <div
          v-if="anonymousAccess === 'read_only'"
          class="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-amber-800 dark:text-amber-200"
        >
          High-risk opt-in: read-only does not mean public. Anonymous tools can still reveal private workspace files,
          configuration, diagnostics, or other sensitive data. Keep this set to “none” for ordinary Agena workspaces.
        </div>
        <div v-else class="text-xs text-muted-foreground">
          Safe default: initialize and tool discovery are public, but every tool call requires OAuth.
        </div>
      </div>

      <div class="grid gap-2">
        <div class="text-sm font-medium">OAuth client registration</div>
        <div class="flex flex-col gap-2 sm:flex-row sm:items-end">
          <OptionPicker
            v-model="clientRegistration"
            class="min-w-0 flex-1"
            :options="clientRegistrationOptions"
            :include-empty="false"
            title="OAuth client registration"
            :disabled="saving"
          />
          <Button class="shrink-0" variant="outline" :disabled="saving" @click="saveClientRegistration"
            >Save registration</Button
          >
        </div>
        <div
          v-if="clientRegistration === 'cimd_and_dcr'"
          class="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-amber-800 dark:text-amber-200"
        >
          Compatibility mode exposes an unauthenticated Dynamic Client Registration endpoint. ChatGPT supports CIMD, so
          leave DCR disabled unless another client requires it.
        </div>
      </div>

      <div v-if="control.authMode !== 'none'" class="grid gap-3 border-t border-border/60 pt-4">
        <div>
          <div class="text-sm font-medium">OAuth authorization</div>
          <div class="mt-1 text-xs text-muted-foreground">
            ChatGPT discovers these endpoints from the MCP server. Do not paste ChatGPT client secrets here. CIMD is
            supported by default; Dynamic Client Registration is available only when explicitly enabled above.
          </div>
        </div>
        <div class="grid gap-2 sm:grid-cols-2">
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">Password status</div>
            <div class="mt-1 font-medium">
              {{
                oauth?.passwordConfigured
                  ? 'MCP-specific password configured'
                  : oauth?.fallbackToUiPassword
                    ? 'Using server UI password'
                    : 'Not configured'
              }}
            </div>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">Scope</div>
            <code class="mt-1 block">{{ oauth?.scope || 'agena:tools' }}</code>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">OAuth readiness</div>
            <div class="mt-1 font-medium">
              {{ oauth?.ready ? 'ready' : 'not ready' }} · {{ oauth?.authorizationServerKind || '—' }}
            </div>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">Client registration</div>
            <div class="mt-1 font-medium">{{ oauth?.registrationMethods.join(' / ') || '—' }}</div>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">Token client auth / PKCE</div>
            <div class="mt-1 font-medium">
              {{ oauth?.tokenEndpointAuthMethods.join(' / ') || '—' }} ·
              {{ oauth?.pkceMethods.join(' / ') || '—' }}
            </div>
          </div>
          <div class="rounded-md border border-border/60 p-3 text-xs">
            <div class="text-muted-foreground">OIDC</div>
            <div class="mt-1 font-medium">{{ oauth?.oidcSupported ? 'supported' : 'not advertised' }}</div>
          </div>
        </div>
        <ul v-if="oauth?.warnings.length" class="list-disc space-y-1 pl-4 text-xs text-amber-700 dark:text-amber-300">
          <li v-for="warning in oauth.warnings" :key="warning">{{ warning }}</li>
        </ul>
        <div class="flex flex-col gap-2 sm:flex-row">
          <Input
            v-model="oauthPassword"
            type="password"
            autocomplete="new-password"
            class="font-mono text-xs"
            placeholder="New MCP OAuth password"
            :disabled="saving"
          />
          <Button class="shrink-0" :disabled="saving || !oauthPassword.trim()" @click="setPassword"
            >Set password</Button
          >
          <Button class="shrink-0" variant="outline" :disabled="saving" @click="clearPassword">Clear</Button>
        </div>
      </div>

      <details v-if="control.authMode !== 'none' && oauth" class="border-t border-border/60 pt-4">
        <summary class="cursor-pointer text-sm font-medium">OAuth discovery endpoints</summary>
        <dl class="mt-3 grid gap-2 text-xs">
          <div
            v-for="entry in [
              ['Protected resource metadata', oauth?.protectedResourceMetadata],
              ['Authorization server metadata', oauth?.authorizationServerMetadata],
              ['Authorization endpoint', oauth?.authorizationEndpoint],
              ['Token endpoint', oauth?.tokenEndpoint],
              ['Registration endpoint', oauth?.registrationEndpoint],
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
                {{ copiedEndpoint === entry[0] ? 'Copied' : 'Copy' }}
              </Button>
            </dd>
          </div>
        </dl>
      </details>
    </template>
  </section>
</template>
