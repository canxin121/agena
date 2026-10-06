import { apiJson } from './api'
import { captureResourceObservation } from './resourceSync'
import type { JsonValue } from '../types/json'

const listeners = new Set<() => void>()
let generation = 0
let inventoryGeneration = 0
export const modelConfigurationGeneration = () => generation
export const modelInventoryGeneration = () => inventoryGeneration
export function subscribeModelConfiguration(callback: () => void) {
  listeners.add(callback)
  return () => listeners.delete(callback)
}

export function notifyModelConfigurationChanged(scope: number, inventoryChanged: boolean) {
  if (scope !== captureResourceObservation('sessions').scope) return
  generation++
  if (inventoryChanged) inventoryGeneration++
  for (const listener of listeners) listener()
}

/** Successful provider/default writes also refresh already-mounted pickers. */
export async function mutateModelConfiguration<T = JsonValue>(path: string, init: RequestInit): Promise<T> {
  const scope = captureResourceObservation('sessions').scope
  const result = await apiJson<T>(path, init)
  notifyModelConfigurationChanged(scope, path !== '/api/v1/settings')
  return result
}
