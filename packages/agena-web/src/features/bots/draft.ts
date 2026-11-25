// The server accepts one Bot's additional system instructions up to this
// storage ceiling. The real limit for a conversation is the selected model's
// context window, which the server reports as a prompt-budget error when the
// assembled prompt is too large. This value mirrors
// `agena_assistant::MAX_BOT_INSTRUCTIONS_BYTES`.
export const MAX_INSTRUCTIONS_BYTES = 256 * 1024

export function instructionsBytes(instructions: string): number {
  return new TextEncoder().encode(instructions).length
}

export function botDraftFields(name: string, instructions: string, avatar?: string | null) {
  return { name: name.trim(), persona: { instructions }, ...(avatar === undefined ? {} : { avatar }) }
}
import { emptyPromptLayer, systemPromptOrder, type BuiltinPrompt, type PromptLayer } from '@/lib/promptLayers'
import type { BotPersona } from '@/generated/agenaState'

export const BOT_INSTRUCTIONS_ID = 'agena.bot.instructions'

export function botPromptOrder(persona: BotPersona, builtins: BuiltinPrompt[], global: PromptLayer): string[] {
  const order = (persona.prompts?.ids ?? []).filter((id) => id !== BOT_INSTRUCTIONS_ID || persona.instructions.trim())
  if (persona.instructions.trim() && !order.includes(BOT_INSTRUCTIONS_ID)) order.push(BOT_INSTRUCTIONS_ID)
  return systemPromptOrder(builtins, [global], { ...emptyPromptLayer(), order })
}
