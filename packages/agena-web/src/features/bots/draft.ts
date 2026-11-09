export const MAX_INSTRUCTIONS_BYTES = 4096

export function instructionsBytes(instructions: string): number {
  return new TextEncoder().encode(instructions).length
}

export function botDraftFields(name: string, instructions: string) {
  return { name: name.trim(), persona: { instructions } }
}
