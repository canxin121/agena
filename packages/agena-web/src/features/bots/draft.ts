// The server accepts one Bot's additional system instructions up to this
// storage ceiling. The real limit for a conversation is the selected model's
// context window, which the server reports as a prompt-budget error when the
// assembled prompt is too large. This value mirrors
// `agena_assistant::MAX_BOT_INSTRUCTIONS_BYTES`.
export const MAX_INSTRUCTIONS_BYTES = 256 * 1024

export function instructionsBytes(instructions: string): number {
  return new TextEncoder().encode(instructions).length
}

export function botDraftFields(name: string, instructions: string) {
  return { name: name.trim(), persona: { instructions } }
}
