import type { ComputedRef, Ref } from 'vue'

import type { ModelCatalogEntry, ProviderModel, ProviderSummary, RuntimeStatus } from '../lib/agenaApi'
import type { OperatorCard, SessionExecutionFact } from './runtimePageModel'

export type RuntimeOverviewStateInput = {
  catalogEntries: Ref<ModelCatalogEntry[]>
  operatorCards: ComputedRef<OperatorCard[]>
  providerModels: Record<string, ProviderModel[]>
  providers: Ref<ProviderSummary[]>
  runtime: Ref<RuntimeStatus | null>
  runtimeSnapshotFacts: ComputedRef<SessionExecutionFact[]>
  sessionCacheFacts: ComputedRef<SessionExecutionFact[]>
}

export function useRuntimeOverviewState(input: RuntimeOverviewStateInput) {
  return {
    catalogEntries: input.catalogEntries,
    operatorCards: input.operatorCards,
    providerModels: input.providerModels,
    providers: input.providers,
    runtime: input.runtime,
    runtimeSnapshotFacts: input.runtimeSnapshotFacts,
    sessionCacheFacts: input.sessionCacheFacts,
  }
}
