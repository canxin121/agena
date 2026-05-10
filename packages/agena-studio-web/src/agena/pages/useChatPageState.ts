import { reactive, ref } from 'vue'

import type {
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
    providerModels,
    providers,
    rewindCheckpoints,
    runtime,
    selectedModelId,
    selectedProviderId,
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
