// Browser bootstrap for scripts/probe-workbench-list-reuse.cjs.
// The shared runner serves this module at /__workbench-lists and selects one
// component with ?case=... . All API traffic is fulfilled by the probe.
import { computed, createApp, defineComponent, h, onMounted, ref, type Component } from 'vue'
import { createPinia } from 'pinia'
import '@/style.css'
import { i18n } from '@/i18n'
import { settingsText } from '@/i18n/settingsText'
import SystemPromptPanel from '@/components/settings/SystemPromptPanel.vue'
import SessionStoragePanel from '@/components/settings/SessionStoragePanel.vue'
import ChannelAssignmentsPanel from '@/components/settings/ChannelAssignmentsPanel.vue'
import PromptReferenceList from '@/components/settings/PromptReferenceList.vue'
import BotPromptEditor from '@/features/bots/BotPromptEditor.vue'
import {
  emptyPermissionCatalog,
  readPermissionCatalog,
  type ChannelPermissionTarget,
  type PermissionCatalog,
} from '@/lib/permissionProfiles'
import { getRuntimeSettingLayer, setRuntimeSetting } from '@/lib/runtimeSettings'
import type { BotPersona } from '@/lib/bots'
import type { PromptFragment } from '@/lib/promptLayers'
import type { JsonValue } from '@/types/json'

// Keep labels deterministic instead of following the browser language.
i18n.global.locale.value = 'en-US'

const params = new URLSearchParams(window.location.search)
const caseName = params.get('case') || 'system-prompt'

function prompt(id: string, title: string, category: string | undefined, body: string): PromptFragment {
  return {
    id,
    title,
    ...(category ? { category } : {}),
    body,
    when: { models: [], requires_tools: null },
  }
}

const referenceOptions: PromptFragment[] = [
  prompt('alpha', 'Alpha prompt', 'Guides', 'Alpha body.'),
  prompt('beta', 'Beta prompt', 'Guides', 'Beta body.'),
  prompt('gamma', 'Gamma prompt', 'Runtime', 'Gamma body.'),
  prompt('delta', 'Delta prompt', undefined, 'Delta body.'),
]

const SystemPromptFixture = defineComponent({
  setup() {
    const scopeParam = params.get('scope')
    const scope = scopeParam === 'workspace' || scopeParam === 'session' ? scopeParam : 'global'
    return () =>
      h(
        'div',
        {
          // The real settings workbench supplies this named container; the
          // panel switches to a split layout through it.
          style:
            'container: settings-workbench / inline-size; width: 100%; max-width: 68rem; margin: 0 auto; padding: 1rem;',
        },
        [
          h(SystemPromptPanel, {
            scope,
            sessionId: scope === 'session' ? params.get('session') || '42' : undefined,
          }),
        ],
      )
  },
})

const BotPromptFixture = defineComponent({
  setup() {
    // ?empty-instructions exercises the add-instructions path and add-row
    // ordering without the implicit instruction prompt.
    const empty = params.has('empty-instructions')
    const persona = ref<BotPersona>({
      instructions: empty ? '' : 'Be concise.',
      prompts: { ids: ['saved.alpha', 'saved.beta'], publish_to_channels: false },
    })
    return () =>
      h('div', { style: 'max-width: 52rem; padding: 1rem;' }, [
        h(BotPromptEditor, {
          modelValue: persona.value,
          'onUpdate:modelValue': (value: BotPersona) => {
            persona.value = value
          },
        }),
        h('pre', { id: 'bot-persona', style: 'white-space: pre-wrap;' }, JSON.stringify(persona.value)),
      ])
  },
})

const PromptReferenceFixture = defineComponent({
  setup() {
    const ids = ref(
      (params.get('ids') || '')
        .split(',')
        .map((value) => value.trim())
        .filter(Boolean),
    )
    const rawLimit = Number(params.get('limit'))
    const limit = params.get('limit') && Number.isFinite(rawLimit) && rawLimit >= 0 ? rawLimit : undefined
    return () =>
      h('div', { style: 'max-width: 40rem; padding: 1rem;' }, [
        h(PromptReferenceList, {
          modelValue: ids.value,
          options: referenceOptions,
          ...(limit !== undefined ? { limit } : {}),
          'onUpdate:modelValue': (value: string[]) => {
            ids.value = value
          },
        }),
        h('pre', { id: 'reference-json' }, JSON.stringify(ids.value)),
      ])
  },
})

const SessionStorageFixture = defineComponent({
  setup() {
    const target = params.get('target') || undefined
    return () => h(SessionStoragePanel, { targetSessionId: target })
  },
})

function channelTarget(): ChannelPermissionTarget | null {
  const resource = params.get('room')
  if (!resource) return null
  const actor = params.get('actor') || undefined
  return {
    platform: params.get('platform') || 'telegram',
    tenant: params.get('tenant') || 'tenant-1',
    account: params.get('account') || 'acct-1',
    resource,
    ...(actor ? { actor } : {}),
  }
}

const ChannelAssignmentsFixture = defineComponent({
  setup() {
    const catalog = ref<PermissionCatalog>(emptyPermissionCatalog())
    const loading = ref(true)
    const saving = ref(false)
    const revision = ref<string | undefined>(undefined)
    const target = channelTarget()
    const targetSource = target ? Number(params.get('source')) || 5 : null
    const profiles = computed(() => catalog.value.profiles)

    async function save(next: PermissionCatalog) {
      saving.value = true
      try {
        // Mirrors PermissionProfilesPanel: re-read the revision before writing.
        const current = await getRuntimeSettingLayer('global', 'access_control')
        const response = await setRuntimeSetting(
          'access_control',
          next as unknown as JsonValue,
          { expected_revision: current.revision },
          'global',
        )
        catalog.value = readPermissionCatalog((response.current ?? next) as JsonValue)
        revision.value = response.after_revision
      } finally {
        saving.value = false
      }
    }

    onMounted(async () => {
      const response = await getRuntimeSettingLayer('global', 'access_control')
      catalog.value = readPermissionCatalog(response.value)
      revision.value = response.revision
      loading.value = false
    })

    return () => {
      if (loading.value) return h('p', { 'data-channel-loading': '' }, 'Loading…')
      return h(ChannelAssignmentsPanel, {
        catalog: catalog.value,
        profiles: profiles.value,
        target,
        targetSource,
        disabled: loading.value,
        saving: saving.value,
        save,
        canDiscard: async () => true,
        onRefresh: () => {},
        'data-channel-revision': revision.value,
      })
    }
  },
})

const cases: Record<string, Component> = {
  'system-prompt': SystemPromptFixture,
  'bot-prompt': BotPromptFixture,
  'prompt-reference': PromptReferenceFixture,
  'session-storage': SessionStorageFixture,
  'channel-assignments': ChannelAssignmentsFixture,
}

const app = createApp(cases[caseName] || SystemPromptFixture)
app.config.globalProperties.$st = settingsText
app.use(createPinia()).use(i18n).mount('#app')
