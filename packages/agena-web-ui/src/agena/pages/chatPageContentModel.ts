import type { ChatSidebarState } from './useChatSidebarState'

export type ChatPageContentState = {
  sidebar: ChatSidebarState
  [key: string]: any
}

export function createChatPageContentState(state: ChatPageContentState) {
  return state
}
