<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { RiAddLine, RiArrowLeftLine, RiFileTextLine, RiLock2Line, RiStackLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { apiJson } from '@/lib/api'
import { confirmAction } from '@/lib/appConfirm'
import {
  availableSavedPrompts,
  composedPromptText,
  promptExcerpt,
  savedPromptDisplayName,
  savedPromptExcerpt,
} from '@/lib/promptLibrary'
import {
  emptyPromptLayer,
  normalizePromptLayer,
  normalizePromptScopeConfig,
  promptLayerAddOrder,
  promptLayerMoveOrder,
  promptLayersEqual,
  promptScopeConfigWithLayer,
  systemPromptOrder,
  type BuiltinPrompt,
  type PromptLayer,
  type PromptResolution,
  type PromptResolutionFragment,
} from '@/lib/promptLayers'
import { readRuntimeSettingSources, setRuntimeSetting, type RuntimeSettingsReadBundle } from '@/lib/runtimeSettings'
import { getSessionPromptPreview, setSessionPrompts, type AgenaExecutionState } from '@/stores/chat/api'
import {
  builtinPromptApplicability,
  builtinPromptTitle,
  readBuiltinPrompts,
  reusableBuiltinPrompts,
} from './promptCatalog'
import PromptOrderRow from './PromptOrderRow.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import SettingsListDetail from './workbench/SettingsListDetail.vue'

const props = defineProps<{ scope: 'global' | 'workspace' | 'session'; sessionId?: string }>()
const emit = defineEmits<{ 'workspace-config-path': [path: string] }>()

const catalog = ref<BuiltinPrompt[]>([])
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const sessionGlobal = ref(false)
const baseline = ref(emptyPromptLayer())
const draft = ref(emptyPromptLayer())
const selected = ref('')
const showDetail = ref(false)
const previewing = ref(false)
const preview = ref<PromptResolution | null>(null)
const loading = ref(false)
const saving = ref(false)
const previewLoading = ref(false)
const error = ref('')
const previewError = ref('')
const listDetail = ref<InstanceType<typeof SettingsListDetail> | null>(null)
const detailScroll = ref<HTMLElement | null>(null)
let request: AbortController | undefined
let previewRequest: AbortController | undefined

const scopeSessionId = computed(() => (props.scope === 'session' ? props.sessionId : undefined))
const globalConfig = computed(() => normalizePromptScopeConfig(sources.value?.global.value))
const workspaceConfig = computed(() => normalizePromptScopeConfig(sources.value?.workspace.value))
const inherited = computed(() => {
  if (props.scope === 'global') return []
  if (props.scope === 'workspace' || sessionGlobal.value) return [globalConfig.value]
  return [globalConfig.value, workspaceConfig.value]
})
const builtinById = computed(
  () => new Map(catalog.value.filter((item) => !item.reusable).map((item) => [item.id, item])),
)
const savedById = computed(() => {
  const prompts = new Map(
    availableSavedPrompts(
      globalConfig.value,
      workspaceConfig.value,
      sessionGlobal.value ? 'global' : props.scope,
      reusableBuiltinPrompts(catalog.value),
    ).map(({ id, fragment }) => [id, fragment]),
  )
  for (const prompt of draft.value.inline) prompts.set(prompt.id, prompt)
  return prompts
})
const blocked = computed(() => new Set(inherited.value.flatMap((layer) => layer.disabled)))
const order = computed(() => systemPromptOrder(catalog.value, inherited.value, draft.value))
const effective = computed(() => preview.value?.fragments ?? [])
const previewByPosition = computed(() => new Map(effective.value.map((item) => [String(item.position), item])))
const rows = computed(() => (previewing.value ? [...previewByPosition.value.keys()] : order.value))
const choices = computed(() =>
  [...savedById.value.values()]
    .filter((prompt) => !order.value.includes(prompt.id) && !blocked.value.has(prompt.id))
    .map((prompt) => ({
      value: prompt.id,
      label: savedPromptDisplayName(prompt),
      description: savedPromptExcerpt(prompt, savedById.value) || prompt.category,
      keywords: prompt.category,
    }))
    .sort((a, b) => a.label.localeCompare(b.label)),
)
const modes = computed(() => [
  { value: 'append', label: st('Append to inherited prompts') },
  { value: 'replace', label: st('Replace inherited prompts') },
])

const activeBuiltin = computed(() => builtinFor(selected.value))
const activeSaved = computed(() => savedById.value.get(selected.value))
const activeResolved = computed(() => previewByPosition.value.get(selected.value))
const body = computed(() => {
  if (previewing.value) return activeResolved.value?.text
  if (activeBuiltin.value) return activeBuiltin.value.body
  if (!activeSaved.value) return undefined
  return composedPromptText(activeSaved.value, savedById.value) ?? undefined
})
const ready = computed(() => Boolean(sources.value) && (props.scope !== 'session' || Boolean(scopeSessionId.value)))
const editable = computed(() => ready.value && !loading.value && !saving.value)
const dirty = computed(() => !promptLayersEqual(draft.value, baseline.value))

function builtinFor(key: string) {
  if (!previewing.value) return builtinById.value.get(key)
  const fragment = previewByPosition.value.get(key)
  return fragment?.scope === 'builtin' ? builtinById.value.get(fragment.id) : undefined
}

function titleFor(key: string) {
  const builtin = builtinFor(key)
  if (builtin) return builtinPromptTitle(builtin)
  const resolved = previewing.value ? previewByPosition.value.get(key) : undefined
  if (resolved?.scope === 'runtime' && resolved.id === 'agena.runtime.session-relation')
    return st('Session relationship')
  if (resolved) {
    const reusable = catalog.value.find((item) => item.reusable && item.id === resolved.id)
    return reusable ? builtinPromptTitle(reusable) : resolved.title || resolved.id
  }
  const saved = savedById.value.get(key)
  return saved ? savedPromptDisplayName(saved) : key
}

function scopeLabel(scope: PromptResolutionFragment['scope']) {
  switch (scope) {
    case 'builtin':
      return st('Built-in')
    case 'global':
      return st('Global layer')
    case 'workspace':
      return st('Workspace layer')
    case 'model':
      return st('Model')
    case 'profile':
      return st('Agent profile')
    case 'session':
      return st('Current session')
    case 'bot':
      return st('AI Bot')
    case 'request':
      return st('Request')
    case 'runtime':
      return st('Runtime context')
  }
}

function detailFor(key: string) {
  const resolved = previewing.value ? previewByPosition.value.get(key) : undefined
  if (resolved && resolved.scope !== 'builtin') return scopeLabel(resolved.scope)
  const builtin = builtinFor(key)
  if (builtin) return builtinPromptApplicability(builtin.when)
  const saved = savedById.value.get(key)
  return saved?.category || (saved?.parts?.length ? st('Prompt composition') : '')
}

function excerptFor(key: string) {
  const resolved = previewing.value ? previewByPosition.value.get(key) : undefined
  const body = resolved?.text ?? builtinFor(key)?.body
  const saved = previewing.value ? undefined : savedById.value.get(key)
  const excerpt = body === undefined ? saved && savedPromptExcerpt(saved, savedById.value) : promptExcerpt(body)
  return excerpt || detailFor(key)
}

function select(id: string, inspect = false) {
  selected.value = id
  if (!inspect) return
  showDetail.value = true
  void nextTick(() => {
    detailScroll.value?.scrollTo({ top: 0 })
    listDetail.value?.focusDetail()
  })
}

function add(id: string) {
  if (!editable.value || !choices.value.some((choice) => choice.value === id)) return
  draft.value = promptLayerAddOrder(draft.value, id)
  select(id)
}

function remove(id: string) {
  if (!editable.value || builtinById.value.has(id)) return
  const position = order.value.indexOf(id)
  draft.value = { ...draft.value, order: draft.value.order.filter((entry) => entry !== id) }
  if (selected.value === id) select(order.value[Math.min(position, order.value.length - 1)] || '')
}

function move(id: string, delta: number) {
  if (!editable.value) return
  draft.value = promptLayerMoveOrder({ ...draft.value, order: order.value }, id, delta)
  select(id)
}

function changeMode(value: string) {
  if (editable.value && (value === 'append' || value === 'replace')) {
    draft.value = { ...draft.value, mode: value }
  }
}

function cancelPreview() {
  previewRequest?.abort()
  previewRequest = undefined
  previewing.value = false
  previewLoading.value = false
  preview.value = null
  previewError.value = ''
}

function closePreview() {
  cancelPreview()
  showDetail.value = false
  select(order.value.includes(selected.value) ? selected.value : order.value[0] || '')
}

async function loadPreview() {
  const id = props.sessionId
  if (!id) return
  cancelPreview()
  previewing.value = true
  previewLoading.value = true
  showDetail.value = false
  const controller = new AbortController()
  previewRequest = controller
  try {
    const result = await getSessionPromptPreview(id, controller.signal)
    if (controller.signal.aborted) return
    preview.value = result
    select(rows.value[0] || '')
  } catch (reason) {
    if (!controller.signal.aborted) previewError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (!controller.signal.aborted) previewLoading.value = false
  }
}

async function load() {
  request?.abort()
  cancelPreview()
  const controller = new AbortController()
  request = controller
  const id = scopeSessionId.value
  sources.value = null
  draft.value = baseline.value = emptyPromptLayer()
  loading.value = true
  error.value = ''
  showDetail.value = false
  try {
    const [settings, builtins, state] = await Promise.all([
      readRuntimeSettingSources('prompts', controller.signal),
      readBuiltinPrompts(controller.signal),
      id
        ? apiJson<AgenaExecutionState>(`/api/v1/sessions/${encodeURIComponent(id)}/state?include_parts=false`, {
            signal: controller.signal,
          })
        : null,
    ])
    if (controller.signal.aborted) return
    sessionGlobal.value = Boolean(state && state.session.workspace_id == null)
    sources.value = settings
    catalog.value = builtins
    emit('workspace-config-path', settings.workspace.config_path || '')
    baseline.value = normalizePromptLayer(
      props.scope === 'session' ? state?.execution?.prompts : settings[props.scope].value,
    )
    draft.value = baseline.value
    select(order.value[0] || '')
  } catch (reason) {
    if (!controller.signal.aborted) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (!controller.signal.aborted) loading.value = false
  }
}

async function refresh() {
  if (saving.value) return
  if (
    dirty.value &&
    !(await confirmAction({
      title: st('Discard unsaved changes?'),
      confirmText: st('Discard changes'),
      cancelText: st('Keep editing'),
    }))
  )
    return
  await load()
}

async function save() {
  if (!editable.value || !dirty.value || !sources.value) return
  const { scope } = props
  const layer = draft.value
  const controller = request
  saving.value = true
  error.value = ''
  try {
    if (scope === 'session') {
      if (!scopeSessionId.value) return
      await setSessionPrompts(scopeSessionId.value, layer)
    } else {
      const base = sources.value[scope]
      if (base.revision === undefined)
        throw new Error(st('The configuration revision is missing. Refresh and try again.'))
      const next = promptScopeConfigWithLayer(scope === 'global' ? globalConfig.value : workspaceConfig.value, layer)
      const response = await setRuntimeSetting(
        'prompts',
        next,
        {
          expected_revision: base.revision,
          workspace_id: scope === 'workspace' && base.workspace_id != null ? String(base.workspace_id) : undefined,
        },
        scope,
      )
      if (controller?.signal.aborted) return
      sources.value = {
        ...sources.value,
        [scope]: { ...base, value: response.current ?? next, revision: response.after_revision },
      }
    }
    if (!controller?.signal.aborted) baseline.value = layer
  } catch (reason) {
    if (!controller?.signal.aborted) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}

function discard() {
  draft.value = baseline.value
  if (!order.value.includes(selected.value)) select(order.value[0] || '')
  error.value = ''
}

watch([() => props.scope, scopeSessionId], load, { immediate: true })
watch(
  () => props.sessionId,
  () => {
    if (previewing.value) closePreview()
  },
)
onBeforeUnmount(() => {
  request?.abort()
  previewRequest?.abort()
})
defineExpose({ dirty, saving, refresh })
</script>

<template>
  <section
    id="settings-system-prompt"
    class="settings-viewport"
    :aria-busy="loading || saving || previewLoading"
    data-system-prompt
    @keydown.ctrl.s.prevent="save"
    @keydown.meta.s.prevent="save"
  >
    <div class="prompt-order-toolbar">
      <div v-if="!previewing" class="min-w-0 space-y-1">
        <p class="text-sm text-muted-foreground">
          {{ $st('Arrange the built-in and selected prompts for this layer.') }}
        </p>
        <p v-if="scope !== 'global'" class="text-xs text-muted-foreground">
          {{ $st('Preview the saved session to see inherited prompts, model rules, and runtime context.') }}
        </p>
      </div>
      <p v-else class="text-sm text-muted-foreground">
        {{ $st('Saved system prompt for the selected session and its current model.') }}
      </p>
      <div class="settings-toolbar">
        <OptionPicker
          v-if="!previewing && scope !== 'global'"
          :model-value="draft.mode"
          :options="modes"
          :title="$st('Layer mode')"
          :include-empty="false"
          :disabled="!editable"
          size="sm"
          @update:model-value="changeMode"
        />
        <Button v-if="previewing" variant="outline" size="sm" @click="closePreview">
          <RiArrowLeftLine class="me-1.5 h-4 w-4 rtl:rotate-180" aria-hidden="true" />
          {{ $st('Back to prompt order') }}
        </Button>
        <Button v-else-if="sessionId" variant="outline" size="sm" :disabled="loading || saving" @click="loadPreview">
          {{ $st('Preview saved prompt') }}
        </Button>
      </div>
    </div>

    <p
      v-if="error || previewError"
      role="alert"
      class="break-words rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
    >
      {{ previewing ? previewError : error }}
    </p>
    <p v-if="loading || previewLoading" role="status" class="px-3 py-6 text-sm text-muted-foreground">
      {{ $st('Loading…') }}
    </p>
    <p v-else-if="!ready && !previewing && !error" class="px-3 py-6 text-sm text-muted-foreground">
      {{ $st('Select a session to continue.') }}
    </p>
    <SettingsListDetail
      v-else-if="rows.length"
      ref="listDetail"
      v-model:detail-open="showDetail"
      class="prompt-order-layout"
      :list-label="previewing ? $st('Resolved order') : $st('Prompt order')"
      :detail-label="selected ? titleFor(selected) : $st('Prompt order')"
      :back-label="$st('Prompt order')"
      :has-detail="Boolean(selected)"
    >
      <template #list-header>
        <header class="flex min-w-0 flex-wrap items-center justify-between gap-2">
          <div class="flex min-w-0 items-center gap-2">
            <RiStackLine class="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
            <h2 data-settings-list-heading tabindex="-1" class="text-sm font-semibold outline-none">
              {{ previewing ? $st('Resolved order') : $st('Prompt order') }}
            </h2>
            <span class="text-xs tabular-nums text-muted-foreground">{{ rows.length }}</span>
          </div>
          <OptionPicker
            v-if="!previewing"
            model-value=""
            :options="choices"
            :title="$st('Add prompt')"
            :placeholder="$st('Add prompt')"
            :search-placeholder="$st('Search prompts…')"
            :include-empty="false"
            :disabled="!editable || !choices.length"
            :icon="RiAddLine"
            size="sm"
            data-system-prompt-add
            @update:model-value="add"
          />
        </header>
      </template>
      <template #list>
        <ol class="grid gap-0.5">
          <PromptOrderRow
            v-for="(id, index) in rows"
            :key="id"
            :row-id="id"
            :title="titleFor(id)"
            :excerpt="excerptFor(id)"
            :position="index + 1"
            selection="pressed"
            :selected="selected === id"
            :reorderable="!previewing"
            :removable="!previewing"
            :can-move-up="index > 0"
            :can-move-down="index + 1 < rows.length"
            :disabled="!editable"
            :locked="Boolean(builtinFor(id))"
            :data-system-prompt-entry="previewing ? previewByPosition.get(id)?.id : id"
            @select="select(id, true)"
            @move="move(id, $event)"
            @remove="remove(id)"
          />
        </ol>
        <div
          v-if="previewing && (preview?.warnings.length || preview?.over_budget)"
          role="status"
          class="m-2 space-y-1 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs"
        >
          <p v-if="preview?.over_budget">
            {{ $st('The combined prompt exceeds the size limit.') }}
          </p>
          <p v-for="(warning, index) in preview?.warnings" :key="index" class="break-words">
            {{ warning.id }} · {{ warning.detail }}
          </p>
        </div>
      </template>
      <template #detail>
        <article v-if="selected" class="flex min-h-0 min-w-0 flex-1 flex-col">
          <header class="prompt-order-heading">
            <div class="flex min-w-0 items-center gap-2.5">
              <component
                :is="activeBuiltin ? RiLock2Line : RiFileTextLine"
                class="h-4 w-4 shrink-0 text-primary"
                aria-hidden="true"
              />
              <div class="min-w-0">
                <h2 data-settings-detail-heading tabindex="-1" class="break-words text-sm font-semibold outline-none">
                  {{ titleFor(selected) }}
                </h2>
                <p v-if="activeBuiltin || detailFor(selected)" class="break-words text-xs text-muted-foreground">
                  {{
                    activeBuiltin ? `${$st('Required built-in prompt')} · ${detailFor(selected)}` : detailFor(selected)
                  }}
                </p>
              </div>
            </div>
            <span v-if="previewing && activeResolved" class="text-xs tabular-nums text-muted-foreground">
              {{ $st('{count} bytes', { count: activeResolved.bytes }) }}
            </span>
          </header>
          <div ref="detailScroll" class="settings-viewport-scroll px-4 py-3 sm:px-5 sm:py-4">
            <pre v-if="body !== undefined" class="prompt-instructions">{{ body }}</pre>
            <p v-else class="text-sm text-muted-foreground">
              {{ $st('Open the saved session preview to inspect the resolved text.') }}
            </p>
          </div>
        </article>
      </template>
    </SettingsListDetail>
    <SettingsSaveBar
      v-if="!previewing && (dirty || saving)"
      :dirty="dirty"
      :saving="saving"
      sticky
      @save="save"
      @discard="discard"
    />
  </section>
</template>

<style scoped>
@reference '../../style.css';

.prompt-order-toolbar {
  @apply flex flex-wrap items-center justify-between gap-2.5 rounded-lg border border-border/70 bg-card/30 px-3 py-2.5;
}

.prompt-order-heading {
  @apply flex min-w-0 shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border/60 bg-muted/20 px-3 py-2;
}

.prompt-instructions {
  @apply whitespace-pre-wrap font-mono text-sm leading-relaxed text-foreground;
  overflow-wrap: anywhere;
}

@container settings-workbench (min-width: 42rem) {
  .prompt-order-layout {
    grid-template-columns: minmax(17rem, 44%) minmax(0, 1fr);
  }
}
</style>
