export type DeriveSendRunConfigInput = {
  selectedProviderId?: string
  selectedAdapterId?: string
  selectedModelId?: string
  selectedThinkingMode?: string
  selectedSpeedMode?: string
}

export type DerivedSendRunConfig = {
  providerID?: string
  adapterID?: string
  modelID?: string
  thinkingMode?: string
  speedMode?: string
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

export function deriveSendRunConfig(input: DeriveSendRunConfigInput): DerivedSendRunConfig {
  const selectedProviderID = text(input.selectedProviderId)
  const selectedAdapterID = text(input.selectedAdapterId)
  const selectedModelID = text(input.selectedModelId)
  const hasSelectedModel = Boolean(selectedProviderID && selectedModelID)
  const providerID = hasSelectedModel ? selectedProviderID : ''
  const adapterID = hasSelectedModel ? selectedAdapterID : ''
  const modelID = hasSelectedModel ? selectedModelID : ''
  const thinkingMode = text(input.selectedThinkingMode)
  const speedMode = text(input.selectedSpeedMode)

  const output: DerivedSendRunConfig = {}
  if (providerID && modelID) {
    output.providerID = providerID
    if (adapterID) output.adapterID = adapterID
    output.modelID = modelID
  }
  if (hasSelectedModel) {
    if (thinkingMode) output.thinkingMode = thinkingMode
    if (speedMode) output.speedMode = speedMode
  }
  return output
}
