import { apiJson } from '@/lib/api'
import type { PluginCommandResult } from '@/lib/pluginOperations'
import type {
  TelegramAiBot,
  TelegramBotConfig,
  TelegramCatalog,
  TelegramInbox,
  TelegramVerification,
  TelegramRoom,
} from './model'

async function invoke<T>(command: string, input: unknown, signal?: AbortSignal): Promise<T> {
  const response = await apiJson<{ result: PluginCommandResult }>(
    `/api/v1/plugins/agena.telegram/commands/${command}/invoke`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      signal,
      body: JSON.stringify({ input, session_id: null, slash: null, raw: '' }),
    },
  )
  if (signal?.aborted) throw new DOMException('Operation cancelled', 'AbortError')
  if (response.result?.status !== 'succeeded') throw new Error(response.result?.summary || 'Telegram operation failed')
  return response.result.output as T
}
export const telegramApi = {
  list: (signal?: AbortSignal) => invoke<TelegramCatalog>('list_bots', {}, signal),
  aiBots: (signal?: AbortSignal) => invoke<{ bots: TelegramAiBot[] }>('list_ai_bots', {}, signal),
  save: (revision: number, bot: TelegramBotConfig, token?: string) =>
    invoke<{ revision: number; bot_id: string }>('save_bot', {
      expected_revision: revision,
      bot,
      token: token || null,
    }),
  remove: (revision: number, bot_id: string) =>
    invoke<{ revision: number }>('remove_bot', { expected_revision: revision, bot_id }),
  verify: (bot_id: string, signal?: AbortSignal) =>
    invoke<TelegramVerification>('check_connection', { bot_id }, signal),
  inventory: (bot_id: string, signal?: AbortSignal) => invoke<TelegramInbox>('bot_inventory', { bot_id }, signal),
  sync: (bot_id: string, signal?: AbortSignal) => invoke<TelegramInbox>('sync_chats', { bot_id }, signal),
  inspect: (bot_id: string, chat_id: string, signal?: AbortSignal) =>
    invoke<{ room: TelegramRoom }>('inspect_chat', { bot_id, chat_id }, signal),
  addChat: (bot_id: string, chat_id: string, kind: 'direct' | 'group' | 'broadcast', title?: string) =>
    invoke<{ bot_id: string; room: TelegramRoom }>('add_chat', { bot_id, chat_id, kind, title: title?.trim() || null }),
}
