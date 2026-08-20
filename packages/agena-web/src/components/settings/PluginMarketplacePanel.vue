<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiDownloadCloud2Line, RiRefreshLine, RiSearchLine, RiUploadCloud2Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { apiJson } from '@/lib/api'
import { useToastsStore } from '@/stores/toasts'

type MarketplacePlugin = {
  plugin_id: string
  name: string
  description: string
  homepage?: string | null
  repository?: string | null
  license?: string | null
  category?: string | null
  tags?: string[]
  version_count: number
  latest_version?: string | null
  latest_kind?: string | null
  latest_platform?: string | null
  latest_source_repository?: string | null
  latest_source_tag?: string | null
  latest_source_commit?: string | null
  review_tier: 'official' | 'verified' | 'community' | string
  featured: boolean
}

type MarketplaceSearchResponse = {
  registry_id: string
  registry_url: string
  marketplace: {
    name: string
    description: string
    homepage?: string | null
    repository?: string | null
    owner_name?: string | null
    owner_url?: string | null
  }
  entries?: MarketplacePlugin[]
}

type InstalledPlugin = {
  plugin_id: string
  version: string
  kind: string
  platform: string
  binary_path: string
  config_path: string
  installed_at: string
  registry_id: string
  registry_url: string
  require_signature: boolean
  require_github_distribution: boolean
}

type OutdatedPlugin = {
  plugin_id: string
  installed_version: string
  latest_version: string
}

type ItemEnvelope<T> = { items?: T[] }

type BackgroundTask = {
  id: string
  kind: string
  title: string
  status: 'running' | 'succeeded' | 'failed' | 'cancelled' | string
  message?: string | null
  failure?: { fallback?: string; title?: string; detail?: string } | null
}

type BackgroundTaskStart = {
  started: boolean
  task: BackgroundTask
}

const toasts = useToastsStore()
const loading = ref(false)
const actionKey = ref('')
const error = ref('')
const query = ref('')
const source = ref('')
const directRepository = ref('')
const requireSignature = ref(false)
const allowUnverified = ref(false)
const searchResponse = ref<MarketplaceSearchResponse | null>(null)
const installed = ref<InstalledPlugin[]>([])
const outdated = ref<OutdatedPlugin[]>([])

const entries = computed(() => searchResponse.value?.entries || [])
const installedById = computed(() => new Map(installed.value.map((entry) => [entry.plugin_id, entry])))
const outdatedById = computed(() => new Map(outdated.value.map((entry) => [entry.plugin_id, entry])))

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => globalThis.setTimeout(resolve, milliseconds))
}

function marketplaceRegistryBody(): { registry_id?: string; registry_url?: string } {
  const value = source.value.trim()
  return value ? { registry_id: 'custom', registry_url: value } : {}
}

function taskFailure(task: BackgroundTask): string {
  return String(task.failure?.fallback || task.failure?.detail || task.failure?.title || task.message || '').trim()
}

async function waitForTask(start: BackgroundTaskStart): Promise<BackgroundTask> {
  if (!start.started || start.task.status !== 'running') return start.task
  for (let attempt = 0; attempt < 120; attempt += 1) {
    await delay(250)
    const response = await apiJson<ItemEnvelope<BackgroundTask>>('/api/v1/runtime/tasks')
    const task = (response.items || []).find((entry) => entry.id === start.task.id)
    if (task && task.status !== 'running') return task
  }
  throw new Error(st('The marketplace operation is still running. Check Runtime background tasks for progress.'))
}

async function runTask(key: string, request: Promise<BackgroundTaskStart>) {
  if (actionKey.value) return
  actionKey.value = key
  error.value = ''
  try {
    const task = await waitForTask(await request)
    if (task.status === 'succeeded') {
      toasts.push('success', task.message || st('Marketplace operation completed'))
    } else {
      throw new Error(taskFailure(task) || st('Marketplace operation failed'))
    }
    await refreshInstalled()
    await search(false)
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    error.value = message
    toasts.push('error', message)
  } finally {
    actionKey.value = ''
  }
}

async function refreshInstalled() {
  const [installedResponse, outdatedResponse] = await Promise.all([
    apiJson<ItemEnvelope<InstalledPlugin>>('/api/v1/plugins/marketplace/installed'),
    apiJson<ItemEnvelope<OutdatedPlugin>>('/api/v1/plugins/marketplace/outdated'),
  ])
  installed.value = [...(installedResponse.items || [])].sort((left, right) =>
    left.plugin_id.localeCompare(right.plugin_id),
  )
  outdated.value = [...(outdatedResponse.items || [])].sort((left, right) =>
    left.plugin_id.localeCompare(right.plugin_id),
  )
}

async function search(refresh = false) {
  loading.value = true
  error.value = ''
  try {
    searchResponse.value = await apiJson<MarketplaceSearchResponse>('/api/v1/plugins/marketplace/search', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        ...marketplaceRegistryBody(),
        query: query.value.trim() || null,
        refresh,
      }),
    })
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}

async function syncRegistry() {
  await runTask(
    'sync',
    apiJson<BackgroundTaskStart>('/api/v1/plugins/marketplace/sync', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(marketplaceRegistryBody()),
    }),
  )
}

async function installPlugin(spec: string, dryRun = false) {
  const trimmed = spec.trim()
  if (!trimmed) return
  await runTask(
    `${dryRun ? 'dry-run' : 'install'}:${trimmed}`,
    apiJson<BackgroundTaskStart>('/api/v1/plugins/marketplace/install', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        spec: trimmed,
        ...(!trimmed.includes('/') ? marketplaceRegistryBody() : {}),
        dry_run: dryRun,
        refresh: true,
        require_signature: requireSignature.value,
        allow_unverified: allowUnverified.value,
      }),
    }),
  )
}

async function uninstallPlugin(pluginId: string) {
  await runTask(
    `uninstall:${pluginId}`,
    apiJson<BackgroundTaskStart>('/api/v1/plugins/marketplace/uninstall', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ plugin_id: pluginId, cascade: false }),
    }),
  )
}

async function upgradePlugin(pluginId?: string) {
  await runTask(
    pluginId ? `upgrade:${pluginId}` : 'upgrade:all',
    apiJson<BackgroundTaskStart>('/api/v1/plugins/marketplace/upgrade', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        plugin_id: pluginId || null,
        all: !pluginId,
        ...marketplaceRegistryBody(),
      }),
    }),
  )
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    await Promise.all([refreshInstalled(), search(false)])
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}

onMounted(() => void refresh())
</script>

<template>
  <div class="grid min-w-0 gap-5">
    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="min-w-0">
        <h2 class="text-base font-semibold">{{ $st('Plugin Marketplace') }}</h2>
        <p class="mt-1 max-w-3xl text-sm leading-6 text-muted-foreground">
          {{
            $st(
              'Discover GitHub-hosted plugins, verify immutable release assets, and manage installed versions from one server-owned workflow.',
            )
          }}
        </p>
      </div>
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? $st('Refreshing marketplace') : $st('Refresh marketplace')"
        :aria-label="loading ? $st('Refreshing marketplace') : $st('Refresh marketplace')"
        :disabled="loading || Boolean(actionKey)"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </header>

    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <section class="grid gap-3 rounded-lg border border-border/60 bg-muted/10 p-4">
      <div class="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(14rem,0.7fr)_auto]">
        <SearchInput
          v-model="query"
          :placeholder="$st('Search plugin id, name, category, repository, or tags')"
          :show-search-button="false"
          :input-aria-label="$st('Search plugin marketplace')"
          @keydown.enter.prevent="search(false)"
        />
        <input
          v-model="source"
          class="h-9 min-w-0 rounded-md border border-input bg-background px-3 font-mono text-xs"
          :placeholder="$st('Official marketplace or owner/repository@ref')"
          :aria-label="$st('Marketplace source')"
          @keydown.enter.prevent="search(true)"
        />
        <div class="flex gap-2">
          <Button size="sm" :disabled="loading || Boolean(actionKey)" @click="search(false)">
            <RiSearchLine class="mr-1.5 h-4 w-4" />{{ $st('Search') }}
          </Button>
          <Button size="sm" variant="outline" :disabled="loading || Boolean(actionKey)" @click="syncRegistry">
            <RiUploadCloud2Line class="mr-1.5 h-4 w-4" />{{ $st('Sync') }}
          </Button>
        </div>
      </div>
      <div class="flex flex-wrap gap-x-5 gap-y-2 text-xs text-muted-foreground">
        <label class="inline-flex items-center gap-2">
          <input v-model="requireSignature" type="checkbox" />
          {{ $st('Require a trusted Ed25519 signature') }}
        </label>
        <label class="inline-flex items-center gap-2">
          <input v-model="allowUnverified" type="checkbox" />
          {{ $st('Allow artifacts without SHA-256 only when explicitly requested') }}
        </label>
        <span v-if="searchResponse?.registry_url" class="break-all font-mono text-[10px]">
          {{ searchResponse.registry_url }}
        </span>
      </div>
      <div
        v-if="searchResponse?.marketplace?.name"
        class="rounded-md border border-border/50 bg-background/70 px-3 py-2 text-xs"
      >
        <div class="font-medium">{{ searchResponse.marketplace.name }}</div>
        <div v-if="searchResponse.marketplace.description" class="mt-1 text-muted-foreground">
          {{ searchResponse.marketplace.description }}
        </div>
        <div class="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
          <span v-if="searchResponse.marketplace.owner_name">
            {{ $st('Maintainer') }}: {{ searchResponse.marketplace.owner_name }}
          </span>
          <a
            v-if="searchResponse.marketplace.repository"
            :href="searchResponse.marketplace.repository"
            target="_blank"
            rel="noopener noreferrer"
            class="font-mono text-primary underline-offset-4 hover:underline"
          >
            {{ searchResponse.marketplace.repository }}
          </a>
        </div>
      </div>
    </section>

    <section class="grid gap-3 rounded-lg border border-border/60 p-4">
      <div>
        <h3 class="text-sm font-semibold">{{ $st('Install directly from GitHub Releases') }}</h3>
        <p class="mt-1 text-xs leading-5 text-muted-foreground">
          {{
            $st('Enter owner/repository for the latest release or owner/repository@tag for an exact immutable version.')
          }}
        </p>
      </div>
      <div class="flex flex-col gap-2 sm:flex-row">
        <input
          v-model="directRepository"
          class="h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-3 font-mono text-sm"
          :placeholder="$st('owner/repository@v0.1.0')"
          :aria-label="$st('GitHub plugin repository')"
          @keydown.enter.prevent="installPlugin(directRepository)"
        />
        <Button
          size="sm"
          :disabled="!directRepository.trim() || Boolean(actionKey)"
          @click="installPlugin(directRepository)"
        >
          <RiDownloadCloud2Line class="mr-1.5 h-4 w-4" />{{ $st('Install from GitHub') }}
        </Button>
      </div>
    </section>

    <section class="grid gap-3">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 class="text-sm font-semibold">{{ $st('Marketplace catalog') }}</h3>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {{ $st('{count} plugin releases match the current query.', { count: entries.length }) }}
          </p>
        </div>
      </div>
      <div v-if="loading && entries.length === 0" class="text-sm text-muted-foreground">
        {{ $st('Loading marketplace plugins…') }}
      </div>
      <div
        v-else-if="entries.length === 0"
        class="rounded-md border border-border/60 p-6 text-center text-sm text-muted-foreground"
      >
        {{ $st('No marketplace plugins match this query.') }}
      </div>
      <div v-else class="grid gap-3 xl:grid-cols-2">
        <article
          v-for="plugin in entries"
          :key="plugin.plugin_id"
          class="grid gap-3 rounded-lg border border-border/60 p-4"
        >
          <div class="flex flex-wrap items-start justify-between gap-3">
            <div class="min-w-0">
              <h4 class="break-all font-mono text-sm font-semibold">{{ plugin.plugin_id }}</h4>
              <p v-if="plugin.name" class="mt-1 text-sm">{{ plugin.name }}</p>
            </div>
            <span v-if="plugin.latest_version" class="rounded bg-muted px-2 py-1 font-mono text-[10px]">
              v{{ plugin.latest_version }}
            </span>
            <span v-if="plugin.review_tier" class="rounded bg-muted px-2 py-1 text-[10px]">
              {{
                plugin.review_tier === 'official'
                  ? $st('Official')
                  : plugin.review_tier === 'verified'
                    ? $st('Verified')
                    : $st('Community')
              }}
            </span>
            <span v-if="plugin.featured" class="rounded bg-primary/10 px-2 py-1 text-[10px] text-primary">
              {{ $st('Featured') }}
            </span>
          </div>
          <p class="text-xs leading-5 text-muted-foreground">
            {{ plugin.description || $st('No marketplace description was provided.') }}
          </p>
          <div class="flex flex-wrap gap-1.5">
            <span v-if="plugin.category" class="rounded bg-muted px-2 py-1 text-[10px]">{{ plugin.category }}</span>
            <span v-for="tag in plugin.tags || []" :key="tag" class="rounded bg-muted px-2 py-1 font-mono text-[10px]">
              {{ tag }}
            </span>
          </div>
          <div class="grid gap-1 text-[11px] text-muted-foreground sm:grid-cols-2">
            <span>{{ $st('Kind') }}: {{ plugin.latest_kind || $st('Not reported') }}</span>
            <span>{{ $st('Target') }}: {{ plugin.latest_platform || $st('Not reported') }}</span>
            <span>{{ $st('License') }}: {{ plugin.license || $st('Not reported') }}</span>
            <a
              v-if="plugin.repository"
              class="truncate text-primary underline-offset-4 hover:underline"
              :href="plugin.repository"
              target="_blank"
              rel="noopener noreferrer"
            >
              {{ plugin.repository }}
            </a>
          </div>
          <div
            v-if="plugin.latest_source_repository || plugin.latest_source_commit"
            class="rounded-md border border-border/50 bg-muted/20 px-3 py-2 text-[10px] text-muted-foreground"
          >
            <div class="font-medium text-foreground">{{ $st('Release provenance') }}</div>
            <div v-if="plugin.latest_source_repository" class="mt-1 break-all font-mono">
              {{ plugin.latest_source_repository
              }}<template v-if="plugin.latest_source_tag">@{{ plugin.latest_source_tag }}</template>
            </div>
            <div v-if="plugin.latest_source_commit" class="mt-1 font-mono">
              {{ $st('Source commit') }}: {{ plugin.latest_source_commit.slice(0, 12) }}
            </div>
          </div>
          <div class="flex flex-wrap justify-end gap-2 border-t border-border/60 pt-3">
            <Button
              size="sm"
              variant="outline"
              :disabled="Boolean(actionKey)"
              @click="installPlugin(plugin.plugin_id, true)"
            >
              {{ $st('Dry run') }}
            </Button>
            <Button
              v-if="!installedById.has(plugin.plugin_id)"
              size="sm"
              :disabled="Boolean(actionKey)"
              @click="installPlugin(plugin.plugin_id)"
            >
              <RiDownloadCloud2Line class="mr-1.5 h-4 w-4" />{{ $st('Install') }}
            </Button>
            <Button
              v-else-if="outdatedById.has(plugin.plugin_id)"
              size="sm"
              :disabled="Boolean(actionKey)"
              @click="upgradePlugin(plugin.plugin_id)"
            >
              {{ $st('Upgrade') }}
            </Button>
            <span v-else class="self-center text-xs text-success">{{ $st('Installed') }}</span>
          </div>
        </article>
      </div>
    </section>

    <section class="grid gap-3 border-t border-border/60 pt-5">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 class="text-sm font-semibold">{{ $st('Installed marketplace plugins') }}</h3>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {{ $st('{count} plugin installations are managed by the marketplace.', { count: installed.length }) }}
          </p>
        </div>
        <Button
          v-if="outdated.length"
          size="sm"
          variant="outline"
          :disabled="Boolean(actionKey)"
          @click="upgradePlugin()"
        >
          {{ $st('Upgrade all outdated plugins') }}
        </Button>
      </div>
      <div v-if="installed.length === 0" class="text-sm text-muted-foreground">
        {{ $st('No marketplace plugins are installed.') }}
      </div>
      <div v-else class="divide-y divide-border/60 rounded-lg border border-border/60">
        <div
          v-for="plugin in installed"
          :key="plugin.plugin_id"
          class="grid gap-2 px-4 py-3 sm:grid-cols-[minmax(0,1fr)_auto]"
        >
          <div class="min-w-0">
            <div class="flex flex-wrap items-center gap-2">
              <span class="font-mono text-xs font-semibold">{{ plugin.plugin_id }}</span>
              <span class="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px]">v{{ plugin.version }}</span>
              <span v-if="outdatedById.has(plugin.plugin_id)" class="text-[10px] text-warning">
                {{
                  $st('Update available: {version}', {
                    version: outdatedById.get(plugin.plugin_id)?.latest_version || '',
                  })
                }}
              </span>
            </div>
            <div class="mt-1 break-all text-[10px] text-muted-foreground">
              {{ plugin.kind }} · {{ plugin.platform }} · {{ plugin.binary_path }}
            </div>
            <div class="mt-2 flex flex-wrap gap-1.5 text-[10px] text-muted-foreground">
              <span v-if="plugin.require_github_distribution" class="rounded bg-muted px-2 py-1">
                {{ $st('GitHub provenance required') }}
              </span>
              <span v-if="plugin.require_signature" class="rounded bg-muted px-2 py-1">
                {{ $st('Signature required') }}
              </span>
              <span
                v-if="!plugin.require_github_distribution && !plugin.require_signature"
                class="rounded bg-muted px-2 py-1"
              >
                {{ $st('Digest verified') }}
              </span>
            </div>
          </div>
          <div class="flex items-center justify-end gap-2">
            <Button
              v-if="outdatedById.has(plugin.plugin_id)"
              size="sm"
              variant="outline"
              :disabled="Boolean(actionKey)"
              @click="upgradePlugin(plugin.plugin_id)"
            >
              {{ $st('Upgrade') }}
            </Button>
            <Button size="sm" variant="ghost" :disabled="Boolean(actionKey)" @click="uninstallPlugin(plugin.plugin_id)">
              {{ $st('Uninstall') }}
            </Button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>
