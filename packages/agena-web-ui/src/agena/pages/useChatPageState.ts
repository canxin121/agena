import { computed, reactive, ref } from 'vue'

import type {
  MessagePart,
  MessageResource,
  ProviderModel,
  ProviderSummary,
  RewindCheckpointResource,
  RuntimeStatus,
  SessionExecutionResource,
  SessionPart,
  SessionResource,
  SessionTreeResource,
  WorkspaceResource,
} from '../lib/agenaApi'
import type { ComposerAttachmentDraft } from './chatAttachmentModel'
import type { ComposerQueueItem } from './chatQueueModel'
import type { ComposerSkillDraft } from './chatSkillModel'
import type { ComposerTextArtifactDraft } from './chatTextArtifactModel'
import { partsToMessages } from './chatRenderModel'

export function useChatPageState() {
  const runtime = ref<RuntimeStatus | null>(null)
  const providers = ref<ProviderSummary[]>([])
  const providerModels = reactive<Record<string, ProviderModel[]>>({})
  const workspaces = ref<WorkspaceResource[]>([])
  const sessions = ref<SessionResource[]>([])
  const inspectedMessage = ref<MessageResource | null>(null)
  const inspectedMessageParts = ref<MessagePart[]>([])
  const inspectedPart = ref<MessagePart | null>(null)
  const sessionState = ref<SessionExecutionResource | null>(null)
  // v2 canonical conversation parts — the single source of truth for the
  // transcript. The renderable message view is derived from it.
  const parts = ref<SessionPart[]>([])
  const messages = computed(() => partsToMessages(parts.value, sessionState.value?.session.id ?? 0))
  const sessionTree = ref<SessionTreeResource[]>([])
  const rewindCheckpoints = ref<RewindCheckpointResource[]>([])

  const selectedWorkspaceId = ref<number | null>(null)
  const selectedSessionId = ref<number | null>(null)
  const workspacePath = ref('')
  const sessionSearch = ref('')
  const sessionViewMode = ref<'all' | 'roots' | 'subtree'>('all')
  const newSessionTitle = ref('')
  const composer = ref('')
  const selectedProviderId = ref('')
  const selectedAdapterId = ref('')
  const selectedModelId = ref('')
  const selectedThinkingMode = ref('')
  const selectedSpeedMode = ref('')
  const selectedVerbosity = ref('')
  const selectedParallelToolCalls = ref('')
  const selectedTemperature = ref('')
  const selectedMaxOutput = ref('')
  const selectedSystemPrompt = ref('')
  const loading = ref(false)
  const sending = ref(false)
  const continuing = ref(false)

  const interactiveRequestInFlight = reactive<Record<string, boolean>>({})
  const userInputDrafts = reactive<Record<string, Record<string, string>>>({})
  const sessionImportJsonl = ref('')
  const attachments = ref<ComposerAttachmentDraft[]>([])
  const attachmentLoading = ref(false)
  const skillReferences = ref<ComposerSkillDraft[]>([])
  const skillPickerOpen = ref(false)
  const textArtifacts = ref<ComposerTextArtifactDraft[]>([])
  const composerQueue = ref<ComposerQueueItem[]>([])
  const queueDraining = ref(false)

  return {
    attachments,
    attachmentLoading,
    composerQueue,
    composer,
    continuing,
    interactiveRequestInFlight,
    loading,
    messages,
    newSessionTitle,
    inspectedMessage,
    inspectedMessageParts,
    inspectedPart,
    parts,
    providerModels,
    providers,
    queueDraining,
    rewindCheckpoints,
    runtime,
    selectedAdapterId,
    selectedModelId,
    selectedProviderId,
    selectedThinkingMode,
    selectedSpeedMode,
    selectedVerbosity,
    selectedParallelToolCalls,
    selectedTemperature,
    selectedMaxOutput,
    selectedSystemPrompt,
    selectedSessionId,
    selectedWorkspaceId,
    sending,
    sessionImportJsonl,
    sessionSearch,
    sessionViewMode,
    sessionState,
    sessions,
    sessionTree,
    skillPickerOpen,
    skillReferences,
    textArtifacts,
    userInputDrafts,
    workspacePath,
    workspaces,
  }
}
