<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import Dialog from '@/components/ui/Dialog.vue'
import Button from '@/components/ui/Button.vue'
import { confirmAction } from '@/lib/appConfirm'
import { useTelegramCopy } from './copy'
import { validateTelegramConnection, type TelegramBotConfig } from './model'
const props = defineProps<{
  initial: TelegramBotConfig
  existing: boolean
  hasToken: boolean
  busy: boolean
  error: string
}>()
const emit = defineEmits<{ close: []; save: [bot: TelegramBotConfig, token: string] }>()
const { tx } = useTelegramCopy()
const draft = ref<TelegramBotConfig>(JSON.parse(JSON.stringify(props.initial)))
const token = ref('')
const localError = ref('')
const field =
  'mt-1.5 h-10 w-full min-w-0 rounded-lg border border-border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring'
const dirty = computed(() => token.value.length > 0 || JSON.stringify(draft.value) !== JSON.stringify(props.initial))
async function close() {
  if (props.busy) return
  if (
    dirty.value &&
    !(await confirmAction({
      title: tx('discard'),
      confirmText: tx('confirm'),
      cancelText: tx('cancel'),
      variant: 'default',
    }))
  )
    return
  token.value = ''
  emit('close')
}
function save() {
  localError.value = ''
  try {
    const bot = {
      ...draft.value,
      name: draft.value.name.trim(),
      connection: validateTelegramConnection(draft.value.connection),
    }
    if (!bot.name || bot.name.length > 80) throw new Error('Display name: 1–80 characters')
    if (bot.credential_kind === 'vault' && !props.hasToken && !token.value.trim())
      throw new Error('Enter a BotFather token.')
    emit('save', bot, token.value)
  } catch (error) {
    localError.value = error instanceof Error ? error.message : String(error)
  }
}
onBeforeUnmount(() => {
  token.value = ''
})
</script>
<template>
  <Dialog
    open
    :title="existing ? tx('edit') : tx('addBot')"
    :description="tx('accountHint')"
    max-width="max-w-2xl"
    @close="close"
  >
    <form class="space-y-5" data-testid="telegram-bot-editor" @submit.prevent="save">
      <fieldset :disabled="busy" class="space-y-5">
        <div class="grid gap-4 sm:grid-cols-2">
          <label class="block text-sm font-medium"
            >{{ tx('botName')
            }}<input
              v-model="draft.name"
              :class="field"
              maxlength="80"
              required
              data-testid="telegram-bot-name"
              :placeholder="tx('nameHint')"
          /></label>
          <label class="block text-sm font-medium"
            >{{ tx('account')
            }}<input
              v-model.trim="draft.connection.account"
              :class="field"
              :readonly="existing"
              required
              data-testid="telegram-account"
              autocomplete="off"
              spellcheck="false"
              placeholder="my_agent_bot"
          /></label>
        </div>
        <label class="block text-sm font-medium"
          >{{ tx('credential') }}
          <select v-model="draft.credential_kind" :class="field" data-testid="telegram-credential-kind">
            <option value="vault">{{ tx('vault') }}</option>
            <option value="environment">{{ tx('environment') }}</option>
          </select>
        </label>
        <label v-if="draft.credential_kind === 'vault'" class="block text-sm font-medium"
          >{{ tx('token') }}
          <input
            v-model="token"
            type="password"
            :class="field"
            autocomplete="new-password"
            spellcheck="false"
            data-testid="telegram-token"
            :placeholder="hasToken ? '•••••••• (saved)' : '123456789:…'"
            :required="!hasToken"
          />
          <span class="mt-2 block text-xs font-normal leading-relaxed text-muted-foreground">{{
            tx('tokenHint')
          }}</span>
        </label>
        <label v-else class="block text-sm font-medium"
          >{{ tx('envName') }}
          <input
            v-model.trim="draft.connection.bot_token_env"
            :class="field"
            required
            autocomplete="off"
            spellcheck="false"
            data-testid="telegram-env"
          />
          <span class="mt-2 block text-xs font-normal text-muted-foreground">{{ tx('envHint') }}</span>
        </label>
        <details class="rounded-xl border border-border/70 p-4">
          <summary class="cursor-pointer text-sm font-medium">{{ tx('advanced') }}</summary>
          <div class="mt-4 space-y-4">
            <label class="block text-sm"
              >{{ tx('apiBase')
              }}<input
                v-model.trim="draft.connection.api_base"
                :class="field"
                :readonly="existing"
                type="url"
                required
                data-testid="telegram-api-base"
            /></label>
            <label class="block text-sm"
              >{{ tx('longPoll')
              }}<input
                v-model.number="draft.connection.long_poll_seconds"
                :class="field"
                type="number"
                min="0"
                max="10"
                step="1"
                required
            /></label>
            <label class="flex items-start gap-2 text-xs"
              ><input
                v-model="draft.connection.allow_loopback_http"
                type="checkbox"
                class="mt-0.5"
                data-testid="telegram-loopback"
              />{{ tx('loopback') }}</label
            >
          </div>
        </details>
        <label class="flex items-center gap-2 text-sm"
          ><input v-model="draft.enabled" type="checkbox" data-testid="telegram-bot-enabled" />{{
            tx('enableSetting')
          }}</label
        >
      </fieldset>
      <p
        v-if="localError || error"
        role="alert"
        class="break-words rounded-lg bg-destructive/10 p-3 text-sm text-destructive"
      >
        {{ localError || error }}
      </p>
      <div class="flex justify-end gap-2 border-t border-border/60 pt-4">
        <Button variant="ghost" :disabled="busy" @click="close">{{ tx('cancel') }}</Button
        ><Button type="submit" :disabled="busy" data-testid="telegram-save">{{
          busy ? tx('saving') : tx('save')
        }}</Button>
      </div>
    </form>
  </Dialog>
</template>
