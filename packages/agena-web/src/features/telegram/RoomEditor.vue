<script setup lang="ts">
import { ref, watch } from 'vue'
import Dialog from '@/components/ui/Dialog.vue'
import Button from '@/components/ui/Button.vue'
import { useTelegramCopy } from './copy'
import type { TelegramRoomDraft } from './model'
const props = defineProps<{ initial: TelegramRoomDraft; busy: boolean; pending: boolean; error: string }>()
const emit = defineEmits<{ close: []; submit: [value: TelegramRoomDraft] }>()
const draft = ref({ ...props.initial })
const { tx } = useTelegramCopy()
const field =
  'mt-1.5 h-10 w-full min-w-0 rounded-lg border border-border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring'
watch(
  () => draft.value.kind,
  (kind) => {
    if (kind === 'direct') {
      draft.value.threadMode = 'exact'
      draft.value.threadId = ''
      draft.value.allowUnlisted = false
    }
    draft.value.trigger = kind === 'group' ? 'direct_or_mention' : 'every_message'
  },
)
</script>
<template>
  <Dialog
    open
    :title="tx('addChat')"
    :description="tx('formHint')"
    max-width="max-w-2xl"
    @close="!busy && !pending && emit('close')"
  >
    <form class="space-y-4" data-testid="telegram-create-form" @submit.prevent="emit('submit', { ...draft })">
      <fieldset :disabled="busy || pending" class="space-y-4">
        <div class="grid gap-4 sm:grid-cols-2">
          <label class="block text-sm font-medium"
            >{{ tx('bindingTitle')
            }}<input v-model.trim="draft.title" :class="field" maxlength="120" data-testid="telegram-binding-title"
          /></label>
          <label class="block text-sm font-medium"
            >{{ tx('chats')
            }}<select v-model="draft.kind" :class="field" data-testid="telegram-chat-kind">
              <option value="direct">{{ tx('direct') }}</option>
              <option value="group">{{ tx('group') }}</option>
              <option value="broadcast">{{ tx('broadcast') }}</option>
            </select></label
          >
          <label class="block text-sm font-medium"
            >{{ tx('chatId')
            }}<input
              v-model.trim="draft.chatId"
              :class="field"
              required
              data-testid="telegram-chat-id"
              inputmode="text"
          /></label>
          <label class="block text-sm font-medium"
            >{{ tx('owner')
            }}<input
              v-model.trim="draft.ownerId"
              :class="field"
              required
              data-testid="telegram-owner-id"
              inputmode="numeric"
          /></label>
        </div>
        <p class="text-xs text-muted-foreground">{{ tx('ownerHint') }}</p>
        <label class="block text-sm font-medium"
          >{{ tx('members')
          }}<textarea
            v-model="draft.members"
            :class="field + ' h-20 py-2'"
            rows="2"
            data-testid="telegram-members"
          /><span class="mt-1 block text-xs font-normal text-muted-foreground">{{ tx('membersHint') }}</span></label
        >
        <details class="rounded-xl border border-border/70 p-4" open>
          <summary class="cursor-pointer text-sm font-semibold">{{ tx('behavior') }}</summary>
          <div class="mt-4 grid gap-4 sm:grid-cols-2">
            <label class="block text-xs"
              >{{ tx('trigger')
              }}<select v-model="draft.trigger" :class="field">
                <option value="direct_or_mention">{{ tx('mention') }}</option>
                <option value="every_message">{{ tx('every') }}</option>
              </select></label
            >
            <label class="block text-xs"
              >{{ tx('isolation')
              }}<select v-model="draft.sessionMode" :class="field">
                <option value="per_conversation">{{ tx('perConversation') }}</option>
                <option value="per_actor">{{ tx('perActor') }}</option>
              </select></label
            >
            <label v-if="draft.kind !== 'direct'" class="block text-xs"
              >{{ tx('threads')
              }}<select v-model="draft.threadMode" :class="field">
                <option value="per_thread">{{ tx('perThread') }}</option>
                <option value="exact">{{ tx('exactThread') }}</option>
              </select></label
            >
            <label v-if="draft.kind !== 'direct' && draft.threadMode === 'exact'" class="block text-xs"
              >{{ tx('threadId') }}<input v-model.trim="draft.threadId" :class="field" inputmode="numeric"
            /></label>
          </div>
          <label v-if="draft.kind === 'group'" class="mt-4 flex items-start gap-2 text-xs"
            ><input v-model="draft.allowUnlisted" type="checkbox" class="mt-0.5" />{{ tx('allowUnlisted') }}</label
          >
        </details>
        <details class="rounded-xl border border-border/70 p-4">
          <summary class="cursor-pointer text-sm font-semibold">{{ tx('quotas') }}</summary>
          <div class="mt-4 grid grid-cols-2 gap-4 sm:grid-cols-3">
            <label class="text-xs"
              >{{ tx('daily')
              }}<input v-model.number="draft.dailyLimit" type="number" min="1" max="100" required :class="field"
            /></label>
            <label class="text-xs"
              >{{ tx('total')
              }}<input v-model.number="draft.totalLimit" type="number" min="1" max="10000" required :class="field"
            /></label>
            <label class="text-xs"
              >{{ tx('perUser')
              }}<input v-model.number="draft.perUserLimit" type="number" min="1" max="100" required :class="field"
            /></label>
            <label class="text-xs"
              >{{ tx('expiry')
              }}<input v-model.number="draft.expiresDays" type="number" min="1" max="90" required :class="field"
            /></label>
            <label class="text-xs"
              >{{ tx('execution')
              }}<input
                v-model.number="draft.executionSeconds"
                type="number"
                min="30"
                max="3600"
                required
                :class="field"
            /></label>
            <label class="text-xs"
              >{{ tx('pollSeconds')
              }}<input v-model.number="draft.pollSeconds" type="number" min="1" max="86400" required :class="field"
            /></label>
          </div>
        </details>
        <label class="flex items-start gap-2 rounded-lg bg-muted/50 p-3 text-xs leading-relaxed"
          ><input
            v-model="draft.useSourcePermissions"
            type="checkbox"
            class="mt-0.5"
            required
            data-testid="telegram-source-consent"
          />{{ tx('consent') }}</label
        >
      </fieldset>
      <p v-if="error" role="alert" class="break-words text-sm text-destructive">{{ error }}</p>
      <p v-if="pending" class="text-xs text-muted-foreground">{{ tx('pending') }}</p>
      <div class="flex justify-end gap-2">
        <Button variant="ghost" :disabled="busy || pending" @click="emit('close')">{{ tx('cancel') }}</Button
        ><Button type="submit" :disabled="busy" data-testid="telegram-create">{{
          busy ? tx('creating') : pending ? tx('retry') : tx('create')
        }}</Button>
      </div>
    </form>
  </Dialog>
</template>
