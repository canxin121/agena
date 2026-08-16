export type EffectiveDefaultsLike = {
  provider?: string
  adapter?: string
  model?: string
  thinkingMode?: string
  speedMode?: string
  verbosity?: string
  parallelToolCalls?: boolean
}

export type DeriveSendRunConfigInput = {
  selectedProviderId?: string
  selectedAdapterId?: string
  selectedModelId?: string
  selectedThinkingMode?: string
  selectedSpeedMode?: string
  effectiveDefaults?: EffectiveDefaultsLike | null
}

export type DerivedSendRunConfig = {
  providerID?: string
  adapterID?: string
  modelID?: string
  thinkingMode?: string
  speedMode?: string
  verbosity?: string
  parallelToolCalls?: boolean
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

export function deriveSendRunConfig(input: DeriveSendRunConfigInput): DerivedSendRunConfig {
  const defaults = input.effectiveDefaults || null
  const selectedProviderID = text(input.selectedProviderId)
  const selectedAdapterID = text(input.selectedAdapterId)
  const selectedModelID = text(input.selectedModelId)
  const defaultProviderID = text(defaults?.provider)
  const defaultAdapterID = text(defaults?.adapter)
  const defaultModelID = text(defaults?.model)
  const hasSelectedModel = Boolean(selectedProviderID && selectedModelID)
  const providerID = hasSelectedModel ? selectedProviderID : defaultProviderID
  const adapterID = hasSelectedModel ? selectedAdapterID : defaultAdapterID
  const modelID = hasSelectedModel ? selectedModelID : defaultModelID
  const usesDefaultModel =
    providerID === defaultProviderID && adapterID === defaultAdapterID && modelID === defaultModelID
  const thinkingMode = text(input.selectedThinkingMode) || (usesDefaultModel ? text(defaults?.thinkingMode) : '')
  const speedMode = text(input.selectedSpeedMode) || (usesDefaultModel ? text(defaults?.speedMode) : '')
  const verbosity = usesDefaultModel ? text(defaults?.verbosity) : ''

  const output: DerivedSendRunConfig = {}
  if (providerID && modelID) {
    output.providerID = providerID
    if (adapterID) output.adapterID = adapterID
    output.modelID = modelID
  }
  if (thinkingMode) output.thinkingMode = thinkingMode
  if (speedMode) output.speedMode = speedMode
  if (verbosity) output.verbosity = verbosity
  if (usesDefaultModel && typeof defaults?.parallelToolCalls === 'boolean') {
    output.parallelToolCalls = defaults.parallelToolCalls
  }
  return output
}
