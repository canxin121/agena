import { reactive, ref } from 'vue'

import type {
  DomainEventRecord,
  MessagePart,
  MessageResource,
  ProviderModel,
  ProviderSummary,
  RewindCheckpointResource,
  RuntimeStatus,
  SessionExecutionResource,
  SessionResource,
  SessionTreeResource,
  WorkspaceResource,
} from '../lib/agenaApi'
import type { ComposerAttachmentDraft } from './chatAttachmentModel'
import type { ComposerQueueItem } from './chatQueueModel'

export function useChatPageState() {
  const runtime = ref<RuntimeStatus | null>(null)
  const providers = ref<ProviderSummary[]>([])
  const providerModels = reactive<Record<string, ProviderModel[]>>({})
  const workspaces = ref<WorkspaceResource[]>([])
  const sessions = ref<SessionResource[]>([])
  const messages = ref<MessageResource[]>([])
  const timelineEvents = ref<DomainEventRecord[]>([])
  const inspectedMessage = ref<MessageResource | null>(null)
  const inspectedMessageParts = ref<MessagePart[]>([])
  const inspectedPart = ref<MessagePart | null>(null)
  const sessionState = ref<SessionExecutionResource | null>(null)
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
  const errorMessage = ref('')

  const interactiveRequestInFlight = reactive<Record<string, boolean>>({})
  const userInputDrafts = reactive<Record<string, Record<string, string>>>({})
  const localCommandNotice = ref('')
  const sessionImportJsonl = ref('')
  const attachments = ref<ComposerAttachmentDraft[]>([])
  const attachmentLoading = ref(false)
  const composerQueue = ref<ComposerQueueItem[]>([])
  const queueDraining = ref(false)

  return {
    attachments,
    attachmentLoading,
    composerQueue,
    composer,
    continuing,
    errorMessage,
    interactiveRequestInFlight,
    loading,
    localCommandNotice,
    messages,
    newSessionTitle,
    inspectedMessage,
    inspectedMessageParts,
    inspectedPart,
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
    timelineEvents,
    userInputDrafts,
    workspacePath,
    workspaces,
  }
}
