export type DeriveSendRunConfigInput = {
  selectedProviderId?: string
  selectedAdapterId?: string
  selectedModelId?: string
  selectedThinkingMode?: string
  selectedSpeedMode?: string
  modelSource?: 'empty' | 'session' | 'default' | 'manual'
  thinkingModeSource?: 'empty' | 'session' | 'default' | 'model' | 'manual'
  speedModeSource?: 'empty' | 'session' | 'default' | 'model' | 'manual'
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
  // Effective values are display projections. Sending them back would turn
  // inherited profile defaults into permanent session overrides.
  if (input.modelSource === 'manual' && providerID && modelID) {
    output.providerID = providerID
    if (adapterID) output.adapterID = adapterID
    output.modelID = modelID
  }
  if (hasSelectedModel) {
    if (input.thinkingModeSource === 'manual' && thinkingMode) output.thinkingMode = thinkingMode
    if (input.speedModeSource === 'manual' && speedMode) output.speedMode = speedMode
  }
  return output
}
