<script setup lang="ts">
import { computed, ref } from 'vue'
import { RiGitBranchLine, RiSearchLine } from '@remixicon/vue'
import PluginEmptyState from './PluginEmptyState.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import {
  filterPluginHookEntries,
  PLUGIN_HOOK_GROUPS,
  pluginHookGroupLabel,
  pluginHookStateLabel,
  pluginHookTrustStatusLabel,
  type PluginHookEntry,
  type PluginHookGroup,
} from './pluginHooks'

const props = defineProps<{ hooks: PluginHookEntry[] }>()
const query = ref('')
const filteredHooks = computed(() => filterPluginHookEntries(props.hooks, query.value))
const groupedHooks = computed<Array<{ group: PluginHookGroup; hooks: PluginHookEntry[] }>>(() =>
  PLUGIN_HOOK_GROUPS.flatMap((group) => {
    const hooks = filteredHooks.value.filter((hook) => hook.group === group)
    return hooks.length ? [{ group, hooks }] : []
  }),
)
const activeCount = computed(() => props.hooks.filter((hook) => hook.state === 'active').length)
</script>

<template>
  <section class="space-y-4" data-plugin-hooks>
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="space-y-1">
        <h3 class="text-sm font-semibold">{{ $st('Hook subscriptions') }}</h3>
        <p class="text-xs leading-relaxed text-muted-foreground">
          {{ $st('Declared hooks are listed separately from active runtime registrations.') }}
        </p>
      </div>
      <p v-if="hooks.length" class="text-xs tabular-nums text-muted-foreground">
        {{ $st('{count} hooks declared or registered', { count: hooks.length }) }}
        · {{ $st('{count} active hooks', { count: activeCount }) }}
      </p>
    </div>
    <PluginEmptyState
      v-if="!hooks.length"
      :title="$st('No hooks available')"
      :description="$st('This plugin does not declare or register any hooks.')"
      :icon="RiGitBranchLine"
    />
    <template v-else>
      <SearchInput
        v-model="query"
        :placeholder="$st('Search hooks')"
        :show-search-button="false"
        :input-aria-label="$st('Search hooks')"
        :clear-aria-label="$st('Clear search')"
        :clear-title="$st('Clear search')"
      />
      <PluginEmptyState
        v-if="!filteredHooks.length"
        :title="$st('No matching hooks.')"
        :description="$st('Try another search term.')"
        :icon="RiSearchLine"
      />
      <div v-for="section in groupedHooks" :key="section.group" class="space-y-2">
        <h4 class="pt-2 text-xs font-semibold text-muted-foreground">
          {{ pluginHookGroupLabel(section.group) }}
        </h4>
        <div class="divide-y divide-border/50 overflow-hidden rounded-lg border border-border/60">
          <article v-for="hook in section.hooks" :key="hook.name" class="space-y-2 p-3 sm:p-4" data-plugin-hook>
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div class="min-w-0">
                <h5 class="text-sm font-medium">{{ hook.title }}</h5>
                <code class="break-all text-xs text-muted-foreground" dir="ltr">{{ hook.name }}</code>
              </div>
              <span
                class="shrink-0 rounded-md border px-2 py-1 text-[11px] font-medium"
                :class="
                  hook.state === 'active'
                    ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
                    : hook.state === 'disabled'
                      ? 'border-destructive/30 bg-destructive/5 text-destructive'
                      : 'border-border bg-muted/30 text-muted-foreground'
                "
                >{{ pluginHookStateLabel(hook.state) }}</span
              >
            </div>
            <p class="text-xs leading-relaxed text-muted-foreground">{{ hook.description }}</p>
            <details v-if="hook.registered" class="rounded-md bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
              <summary class="cursor-pointer select-none font-medium text-foreground">
                {{ $st('Registration details') }}
              </summary>
              <dl class="mt-3 grid gap-3 sm:grid-cols-2">
                <div v-if="hook.source">
                  <dt>{{ $st('Registration source') }}</dt>
                  <dd class="mt-1 break-all font-mono text-foreground">{{ hook.source }}</dd>
                </div>
                <div v-if="hook.trustStatus">
                  <dt>{{ $st('Trust status') }}</dt>
                  <dd class="mt-1 break-all text-foreground">
                    {{ pluginHookTrustStatusLabel(hook.trustStatus) }}
                  </dd>
                </div>
                <div v-if="hook.trustLevel">
                  <dt>{{ $st('Trust level') }}</dt>
                  <dd class="mt-1 break-all font-mono text-foreground">{{ hook.trustLevel }}</dd>
                </div>
                <div v-if="hook.sourcePath">
                  <dt>{{ $st('Source path') }}</dt>
                  <dd class="mt-1 break-all font-mono text-foreground" dir="auto">{{ hook.sourcePath }}</dd>
                </div>
                <div v-if="hook.provenance?.length">
                  <dt>{{ $st('Provenance') }}</dt>
                  <dd class="mt-1 break-all text-foreground">{{ hook.provenance.join(' · ') }}</dd>
                </div>
                <div v-if="hook.currentHash">
                  <dt>{{ $st('Manifest hash') }}</dt>
                  <dd class="mt-1 break-all font-mono text-foreground" dir="ltr">{{ hook.currentHash }}</dd>
                </div>
              </dl>
            </details>
          </article>
        </div>
      </div>
    </template>
  </section>
</template>
