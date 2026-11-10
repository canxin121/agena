// One implementation of "render a stored conversation as Markdown", shared by
// the chat page actions and the Bot sidebar row menu.
import { formatTranscript } from '@/lib/transcript'
import { listSessionMessages } from '@/stores/chat/api'

export type TranscriptSourceSession = {
  id: string
  title?: string
  time?: { created?: number; updated?: number }
}

export async function sessionTranscriptText(
  session: TranscriptSourceSession | null | undefined,
  title: string,
  options: { thinking: boolean },
): Promise<string> {
  if (!session?.id) return ''
  const messages = await listSessionMessages(session.id)
  if (!messages.length) return ''
  return formatTranscript(
    { id: session.id, title: title || session.id, time: session.time },
    messages,
    { thinking: options.thinking, toolDetails: false, assistantMetadata: true },
  )
}

export function downloadMarkdown(filename: string, content: string) {
  const blob = new Blob([content], { type: 'text/markdown' })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  document.body.appendChild(link)
  link.click()
  link.remove()
  URL.revokeObjectURL(url)
}
