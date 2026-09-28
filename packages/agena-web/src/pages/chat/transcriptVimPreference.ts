/**
 * Transcript Vim mode is a browser preference, not a server setting: the modal
 * keyboard model is only convenient where a hardware keyboard exists.
 *
 * The stored value always wins. Before the user ever touches the switch the
 * device decides the default — desktop on, mobile/tablet off — so a phone
 * never opens on a mode whose keys it cannot type, while flipping the switch
 * on any device is still honoured.
 */
export function resolveTranscriptVimEnabled(stored: unknown, isMobileDevice: boolean): boolean {
  if (typeof stored === 'boolean') return stored
  return !isMobileDevice
}
