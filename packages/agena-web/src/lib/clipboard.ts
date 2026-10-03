export async function copyTextToClipboard(text: string | Promise<string>): Promise<boolean> {
  // Start the browser write while the click still has user activation. A
  // promised ClipboardItem lets complete-history reads finish afterwards.
  if (typeof text !== 'string') {
    if (
      typeof navigator !== 'undefined' &&
      typeof window !== 'undefined' &&
      window.isSecureContext &&
      typeof ClipboardItem !== 'undefined' &&
      navigator.clipboard?.write
    ) {
      const blob = text.then((value) => new Blob([value], { type: 'text/plain' }))
      // A denied write can reject before the browser consumes the item.
      void blob.catch(() => {})
      try {
        await navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob })])
        return true
      } catch {
        // Keep the text/read error available to the caller below.
      }
    }
    text = await text
  }
  const value = String(text ?? '')
  if (!value) return false

  // Prefer modern clipboard API when available.
  try {
    if (
      typeof navigator !== 'undefined' &&
      typeof window !== 'undefined' &&
      window.isSecureContext &&
      navigator.clipboard?.writeText
    ) {
      await navigator.clipboard.writeText(value)
      return true
    }
  } catch {
    // Fall back.
  }

  // Fallback for non-secure contexts / older browsers.
  try {
    if (typeof document === 'undefined') return false
    const textarea = document.createElement('textarea')
    textarea.value = value
    textarea.setAttribute('readonly', '')
    textarea.style.position = 'fixed'
    textarea.style.top = '0'
    textarea.style.left = '-9999px'
    textarea.style.opacity = '0'
    document.body.appendChild(textarea)
    textarea.select()
    textarea.setSelectionRange(0, textarea.value.length)
    const ok = document.execCommand('copy')
    document.body.removeChild(textarea)
    return ok
  } catch {
    return false
  }
}

export async function readTextFromClipboard(): Promise<string> {
  try {
    if (
      typeof navigator !== 'undefined' &&
      typeof window !== 'undefined' &&
      window.isSecureContext &&
      navigator.clipboard?.readText
    ) {
      const text = await navigator.clipboard.readText()
      return String(text || '')
    }
  } catch {
    // Fall back.
  }

  return ''
}
