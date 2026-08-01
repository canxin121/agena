import {
  invokePluginUiTool,
  runPluginUiAction,
  type PluginCommandOutput,
  type PluginUiActionRunResponse,
  type PluginUiToolInvokeResponse,
} from './agenaApi'

export type ResolvedPluginCommandEffect =
  | { kind: 'none' }
  | { kind: 'notice'; message: string }
  | { kind: 'submit_prompt'; prompt: string }
  | { kind: 'open_route'; route: string }
  | { kind: 'open_url'; url: string }

export function isPluginCommandOutput(value: unknown): value is PluginCommandOutput {
  return Boolean(value && typeof value === 'object' && 'kind' in value)
}

export function isPluginUiToolInvokeResponse(value: unknown): value is PluginUiToolInvokeResponse {
  return Boolean(value && typeof value === 'object' && 'status' in value && 'output_text' in value)
}

export async function resolvePluginCommandOutput(input: {
  pluginId: string
  result: unknown
  sessionId?: number | null
  fallbackNotice?: string
  invokeTool?: typeof invokePluginUiTool
  invokeCommand?: typeof runPluginUiAction
  maxCommandDepth?: number
}): Promise<ResolvedPluginCommandEffect> {
  const invokeTool = input.invokeTool || invokePluginUiTool
  const invokeCommand = input.invokeCommand || runPluginUiAction
  const fallbackNotice = input.fallbackNotice?.trim() || ''
  const maxCommandDepth = input.maxCommandDepth ?? 8

  if (maxCommandDepth < 0) {
    return fallbackNotice
      ? { kind: 'notice', message: fallbackNotice }
      : { kind: 'notice', message: 'Plugin command recursion limit exceeded.' }
  }

  if (!isPluginCommandOutput(input.result)) {
    return fallbackNotice ? { kind: 'notice', message: fallbackNotice } : { kind: 'none' }
  }

  const result = input.result
  if (result.kind === 'message') {
    return result.text.trim()
      ? { kind: 'notice', message: result.text }
      : fallbackNotice
        ? { kind: 'notice', message: fallbackNotice }
        : { kind: 'none' }
  }
  if (result.kind === 'submit_prompt') {
    return { kind: 'submit_prompt', prompt: result.prompt }
  }
  if (result.kind === 'open_route') {
    return { kind: 'open_route', route: result.route }
  }
  if (result.kind === 'open_url') {
    return { kind: 'open_url', url: result.url }
  }
  if (result.kind === 'invoke_tool') {
    const response = await invokeTool({
      tool: result.tool,
      pluginId: input.pluginId,
      payload: result.input ?? undefined,
      sessionId: input.sessionId,
    })
    const output = response.output_text.trim()
    if (response.status !== 'completed') {
      return {
        kind: 'notice',
        message:
          output ||
          (
            {
              capability_unavailable: 'The current runtime does not provide the required capability.',
              tool_unavailable: 'The requested tool is unavailable.',
            } as const
          )[response.status],
      }
    }
    if (result.submit_output_as_prompt && output) {
      return { kind: 'submit_prompt', prompt: output }
    }
    if (output) {
      return { kind: 'notice', message: output }
    }
  }
  if (result.kind === 'invoke_command') {
    const response: PluginUiActionRunResponse = await invokeCommand({
      pluginId: input.pluginId,
      actionId: result.command,
      payload: result.input ?? undefined,
      sessionId: input.sessionId,
    })
    return resolvePluginCommandOutput({
      pluginId: input.pluginId,
      result: response.result,
      sessionId: input.sessionId,
      fallbackNotice,
      invokeTool,
      invokeCommand,
      maxCommandDepth: maxCommandDepth - 1,
    })
  }

  return fallbackNotice ? { kind: 'notice', message: fallbackNotice } : { kind: 'none' }
}
