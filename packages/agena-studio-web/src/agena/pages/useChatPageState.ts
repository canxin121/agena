import { reactive, ref } from 'vue'

import type {
  MessagePart,
  MessageResource,
  ProviderModel,
  ProviderSummary,
  RewindCheckpointResource,
  RuntimeStatus,
  SessionExecutionResource,
  SessionResource,
  SessionTreeResource,
  TimelineEventRecord,
  WorkspaceResource,
} from '../lib/agenaApi'

export function useChatPageState() {
  const runtime = ref<RuntimeStatus | null>(null)
  const providers = ref<ProviderSummary[]>([])
  const providerModels = reactive<Record<string, ProviderModel[]>>({})
  const workspaces = ref<WorkspaceResource[]>([])
  const sessions = ref<SessionResource[]>([])
  const messages = ref<MessageResource[]>([])
  const timelineEvents = ref<TimelineEventRecord[]>([])
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
  const newSessionTitle = ref('')
  const composer = ref('')
  const selectedProviderId = ref('')
  const selectedModelId = ref('')
  const selectedVariant = ref('')
  const loading = ref(false)
  const sending = ref(false)
  const continuing = ref(false)
  const errorMessage = ref('')

  const userInputDrafts = reactive<Record<string, Record<string, string>>>({})
  const localCommandNotice = ref('')
  const sessionImportJsonl = ref('')

  return {
    composer,
    continuing,
    errorMessage,
    loading,
    localCommandNotice,
    messages,
    newSessionTitle,
    inspectedMessage,
    inspectedMessageParts,
    inspectedPart,
    providerModels,
    providers,
    rewindCheckpoints,
    runtime,
    selectedModelId,
    selectedProviderId,
    selectedVariant,
    selectedSessionId,
    selectedWorkspaceId,
    sending,
    sessionImportJsonl,
    sessionSearch,
    sessionState,
    sessions,
    sessionTree,
    timelineEvents,
    userInputDrafts,
    workspacePath,
    workspaces,
  }
}
