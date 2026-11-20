import { computed, onScopeDispose, ref, watch, type Ref } from 'vue'
import {
  agentProfileCatalog,
  agentProfileDisplayName,
  agentProfileLineage,
  mergedAgentProfiles,
  selectableAgentProfiles,
  withBuiltinAgentProfiles,
  type AgentProfileCatalog,
} from '@/lib/agentProfiles'
import { readRuntimeSettingSources, subscribeRuntimeSettingsChanges } from '@/lib/runtimeSettings'
import { captureResourceObservation } from '@/lib/resourceSync'
import { getSession, getSessionExecution, patchSessionMetadata } from '@/stores/chat/api'
import { sessionStateIsBusy, sessionStateRequests, type Session, type SessionState } from '@/types/chat'
import { settingsText as st } from '@/i18n/settingsText'
import type { OptionMenuGroup } from '@/components/ui/optionMenu.types'
import type { ComposerPickerKind } from './composerPickerTypes'

type ProfileExecution = {
  profile?: string | null
  profile_is_default?: boolean
  queued_messages?: unknown[]
}

type ProfilePickerChat = {
  selectedSessionId: string | null
  selectedSession: Session | null
  selectedSessionState: SessionState
  selectedSessionExecution: ProfileExecution | null
  refreshExecutionStatus: (sessionId: string, forceFresh?: boolean) => Promise<void>
}

export function useChatProfilePicker(options: {
  chat: ProfilePickerChat
  open: Ref<ComposerPickerKind | null>
  directory: () => string
  locallyBusy: () => boolean
  close: () => void
  openDefaults: (scope: 'global' | 'workspace', sessionId: string) => void
}) {
  const { chat } = options
  const query = ref('')
  const catalog = ref<AgentProfileCatalog>({})
  const summary = ref<Session | null>(null)
  const snapshot = ref<ProfileExecution | null>(null)
  const snapshotState = ref<SessionState>()
  const storeRevision = ref(0)
  const snapshotStoreRevision = ref(-1)
  const loading = ref(false)
  const saving = ref(false)
  const error = ref('')
  const loadedContext = ref('')
  let generation = 0
  let controller: AbortController | null = null
  let disposed = false

  function contextKey() {
    return JSON.stringify([captureResourceObservation('sessions').scope, chat.selectedSessionId, options.directory()])
  }

  const session = computed(() =>
    loadedContext.value === contextKey() ? (summary.value ?? chat.selectedSession) : chat.selectedSession,
  )
  const subagent = computed(() => session.value?.relation_kind === 'subagent')
  const fallback = computed(() => (subagent.value ? 'worker' : 'main'))
  const useSnapshot = computed(
    () =>
      options.open.value === 'profile' &&
      loadedContext.value === contextKey() &&
      snapshotStoreRevision.value === storeRevision.value,
  )
  const execution = computed(() =>
    useSnapshot.value ? (snapshot.value ?? chat.selectedSessionExecution) : chat.selectedSessionExecution,
  )
  const state = computed(() =>
    useSnapshot.value ? (snapshotState.value ?? chat.selectedSessionState) : chat.selectedSessionState,
  )
  const isDefault = computed(() => execution.value?.profile_is_default || !execution.value?.profile)
  const selected = computed(() => (isDefault.value ? '' : execution.value?.profile || ''))
  const busy = computed(
    () =>
      options.locallyBusy() ||
      state.value.kind === 'creating' ||
      state.value.kind === 'awaiting_interaction' ||
      sessionStateIsBusy(state.value) ||
      sessionStateRequests(state.value).length > 0 ||
      (execution.value?.queued_messages?.length || 0) > 0,
  )
  const compatible = computed(() => selectableAgentProfiles(catalog.value, subagent.value))
  const entries = computed(() => {
    const needle = query.value.trim().toLocaleLowerCase()
    return Object.entries(compatible.value)
      .map(([id, profile]) => {
        const description = agentProfileLineage(catalog.value, id).reduce(
          (body, ancestor) => catalog.value[ancestor]?.description?.trim() || body,
          '',
        )
        return { id, label: agentProfileDisplayName(id, profile), description }
      })
      .filter((entry) => `${entry.id} ${entry.label} ${entry.description}`.toLocaleLowerCase().includes(needle))
      .sort((left, right) => left.label.localeCompare(right.label))
  })
  const helperText = computed(() => {
    if (error.value) return error.value
    if (saving.value) return st('Saving…')
    if (busy.value) return st('Finish or stop the current run and clear queued messages before changing profiles.')
    if (query.value.trim() && !entries.value.length && !loading.value) return st('No compatible profiles found.')
    return st('Choose the agent profile for the current session.')
  })
  const groups = computed<OptionMenuGroup[]>(() => {
    const disabled = saving.value || busy.value || loading.value || loadedContext.value !== contextKey()
    const result: OptionMenuGroup[] = []
    if (loadedContext.value === contextKey()) {
      result.push({
        id: 'profile-default-role',
        items: [
          {
            id: 'profile:restore',
            label: `${st('Restore default role')} · ${fallback.value}`,
            checked: isDefault.value,
            disabled,
          },
        ],
      })
      result.push({
        id: 'profile-roles',
        title: st('Agent profiles'),
        items: entries.value.map((entry) => ({
          id: `profile:select:${entry.id}`,
          label: entry.label,
          description: [entry.label === entry.id ? '' : entry.id, entry.description].filter(Boolean).join(' · '),
          checked: selected.value === entry.id,
          disabled,
        })),
      })
    }
    result.push({
      id: 'profile-settings',
      items: [
        {
          id: 'profile:settings',
          label: st('Default profile settings'),
          description:
            session.value?.workspace_id == null ? st('Global default profiles') : st('Workspace default profiles'),
          disabled: saving.value,
        },
      ],
    })
    return result
  })

  async function load() {
    const target = chat.selectedSessionId
    if (!target || disposed) return
    const context = contextKey()
    const revision = ++generation
    const currentStoreRevision = storeRevision.value
    controller?.abort()
    const request = new AbortController()
    controller = request
    loading.value = true
    error.value = ''
    loadedContext.value = ''
    try {
      // A missing saved profile can prevent execution projection. Summary and
      // catalog still let the user select a valid role or restore the default.
      const [profiles, builtins, currentSession, state] = await Promise.all([
        readRuntimeSettingSources('profiles', request.signal),
        readRuntimeSettingSources('builtin_profiles', request.signal),
        getSession(target, request.signal),
        getSessionExecution(target).catch(() => null),
      ])
      if (disposed || revision !== generation || context !== contextKey()) return
      const globalProfiles = agentProfileCatalog(profiles.global.value)
      catalog.value = withBuiltinAgentProfiles(
        agentProfileCatalog(builtins.effective.value),
        currentSession.workspace_id == null
          ? globalProfiles
          : mergedAgentProfiles(globalProfiles, agentProfileCatalog(profiles.workspace.value)),
      )
      summary.value = currentSession
      snapshot.value = state
        ? { ...state.execution, queued_messages: state.queued_messages || state.execution?.queued_messages || [] }
        : null
      snapshotState.value = state?.session.state
      snapshotStoreRevision.value = currentStoreRevision
      loadedContext.value = context
    } catch (reason) {
      if (!disposed && revision === generation && context === contextKey() && !request.signal.aborted) {
        catalog.value = {}
        error.value = reason instanceof Error ? reason.message : String(reason)
      }
    } finally {
      if (revision === generation) loading.value = false
    }
  }

  async function select(id: string) {
    const target = chat.selectedSessionId
    if (!target || saving.value || disposed) return
    if (id === 'profile:settings') {
      const scope = session.value?.workspace_id == null ? 'global' : 'workspace'
      options.close()
      options.openDefaults(scope, target)
      return
    }
    if (loading.value || busy.value || loadedContext.value !== contextKey()) return
    const profile =
      id === 'profile:restore' ? '' : id.startsWith('profile:select:') ? id.slice('profile:select:'.length) : null
    if (profile === null || (profile && !Object.hasOwn(compatible.value, profile))) return
    const context = contextKey()
    saving.value = true
    error.value = ''
    try {
      await patchSessionMetadata(target, { profile })
      // Do not issue the follow-up read against another backend after navigation.
      if (disposed || context !== contextKey()) return
      await chat.refreshExecutionStatus(target, true)
      if (!disposed && context === contextKey() && options.open.value === 'profile') options.close()
    } catch (reason) {
      if (!disposed && context === contextKey()) error.value = reason instanceof Error ? reason.message : String(reason)
    } finally {
      saving.value = false
    }
  }

  watch(
    () => [chat.selectedSessionId, options.directory()] as const,
    () => {
      generation++
      controller?.abort()
      loadedContext.value = ''
      catalog.value = {}
      snapshot.value = null
      snapshotState.value = undefined
      summary.value = null
      loading.value = false
      error.value = ''
      query.value = ''
      if (options.open.value === 'profile') options.close()
    },
    { flush: 'sync' },
  )
  watch(
    () => [chat.selectedSessionExecution, chat.selectedSessionState],
    () => {
      storeRevision.value++
    },
    { deep: true, flush: 'sync' },
  )
  watch(options.open, (kind) => {
    if (kind === 'profile') {
      query.value = ''
      void load()
    }
  })
  const unsubscribe = subscribeRuntimeSettingsChanges(() => {
    if (options.open.value === 'profile' && !saving.value) void load()
  })
  onScopeDispose(() => {
    disposed = true
    generation++
    controller?.abort()
    unsubscribe()
  })

  return { query, loading, saving, helperText, groups, load, select }
}
