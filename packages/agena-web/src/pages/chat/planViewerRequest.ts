import type { JsonValue } from '@/types/json'

export type PlanTool = 'get' | 'phase'
export type PlanToolInput = Record<string, JsonValue>

export interface PlanToolInvocationRequest {
  plugin_id: 'agena.plan'
  tool: PlanTool
  input: PlanToolInput
  session_id: number
}

export function buildPlanToolInvocationRequest(
  sessionId: string | null,
  tool: PlanTool,
  input: PlanToolInput,
): PlanToolInvocationRequest | null {
  const numericSessionId = Number(sessionId)
  if (!Number.isSafeInteger(numericSessionId) || numericSessionId <= 0) return null

  return {
    plugin_id: 'agena.plan',
    tool,
    input,
    session_id: numericSessionId,
  }
}
