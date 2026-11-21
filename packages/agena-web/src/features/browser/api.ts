import type { InjectionKey } from 'vue'
import { apiJson } from '@/lib/api'

export const browserTakeoverKind = 'browser_takeover'
export const openBrowserKey: InjectionKey<(sessionId: string, pageId?: string) => void> = Symbol('openBrowser')

export type BrowserState = {
  kind: 'state'
  tabs: Array<{ page_id: string; opener_page_id: string | null; title: string }>
  human_control: boolean
  help: { page_id: string; reason: string; ready_to_resume: boolean } | null
}

export type BrowserFrame = {
  kind: 'frame'
  jpeg: string
  origin: string
  width: number
  height: number
  dialog: { kind: string; message: string } | null
}

type Point = { x: number; y: number; modifiers: number }
export type BrowserInput =
  | (Point & {
      kind: 'pointer'
      action: 'down' | 'up' | 'move'
      button: 'none' | 'left' | 'middle' | 'right'
      buttons: number
      clicks: number
    })
  | (Point & { kind: 'wheel'; delta_x: number; delta_y: number })
  | { kind: 'key'; down: boolean; key: string; code: string; key_code: number; modifiers: number; repeat: boolean }
  | { kind: 'text'; text: string }
  | { kind: 'dialog'; accept: boolean; text?: string }

type BrowserCommand =
  | { kind: 'frame'; page_id: string; control_id?: string }
  | { kind: 'take_control'; page_id: string }
  | { kind: 'input'; page_id: string; control_id: string; events: BrowserInput[] }
  | { kind: 'return_control'; control_id: string; page_id: string }
  | { kind: 'close'; control_id: string }

type CommandResult = BrowserFrame | { kind: 'control'; control_id: string } | { kind: 'done' }

const browserUrl = (sessionId: string) => `/api/v1/sessions/${encodeURIComponent(sessionId)}/browser`

export function readBrowserState(sessionId: string, signal: AbortSignal) {
  return apiJson<BrowserState>(browserUrl(sessionId), { signal, cache: 'no-store' })
}

export function browserCommand(sessionId: string, command: BrowserCommand, signal?: AbortSignal) {
  return apiJson<CommandResult>(browserUrl(sessionId), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(command),
    signal,
    cache: 'no-store',
  })
}
