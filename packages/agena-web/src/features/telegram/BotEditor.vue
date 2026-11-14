<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import Dialog from '@/components/ui/Dialog.vue'
import Button from '@/components/ui/Button.vue'
import { confirmAction } from '@/lib/appConfirm'
import { useTelegramCopy } from './copy'
import { validateTelegramConnection, type TelegramAiBot, type TelegramBotConfig } from './model'
const props = defineProps<{
  initial: TelegramBotConfig
  existing: boolean
  hasToken: boolean
  busy: boolean
  error: string
  aiBots: TelegramAiBot[]
  aiBotsLoading?: boolean
}>()
const emit = defineEmits<{ close: []; save: [bot: TelegramBotConfig, token: string]; refreshAiBots: [] }>()
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
      connection: validateTelegramConnection(draft.value.connection, true),
    }
    if (bot.name.length > 80) throw new Error(tx('displayNameTooLong'))
    if (!bot.ai_bot_id) throw new Error(tx('aiBotRequired'))
    if (bot.credential_kind === 'vault' && !props.hasToken && !token.value.trim()) throw new Error(tx('tokenRequired'))
    emit('save', bot, token.value.trim())
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
        <div>
          <label class="block text-sm font-medium"
            >{{ tx('botName')
            }}<input
              v-model="draft.name"
              :class="field"
              maxlength="80"
              data-testid="telegram-bot-name"
              :placeholder="tx('nameHint')"
          /></label>
          <p v-if="existing && draft.connection.account" class="mt-2 text-xs text-muted-foreground">
            @{{ draft.connection.account }}
          </p>
        </div>
        <label v-if="draft.credential_kind === 'vault'" class="block text-sm font-medium"
          >{{ tx('token') }}
          <input
            v-model="token"
            type="password"
            :class="field"
            autocomplete="new-password"
            spellcheck="false"
            data-testid="telegram-token"
            :placeholder="hasToken ? tx('savedToken') : '123456789:…'"
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
        <div>
          <div class="flex items-center justify-between gap-3">
            <label for="telegram-bot-ai" class="text-sm font-medium">{{ tx('aiBot') }}</label>
            <button
              type="button"
              class="text-xs text-primary hover:underline disabled:opacity-50"
              :disabled="aiBotsLoading"
              @click="emit('refreshAiBots')"
            >
              {{ tx('refreshAiBots') }}
            </button>
          </div>
          <select id="telegram-bot-ai" v-model="draft.ai_bot_id" :class="field" required data-testid="telegram-bot-ai">
            <option :value="null" disabled>{{ tx('chooseAiBot') }}</option>
            <option
              v-if="draft.ai_bot_id && !aiBots.some((item) => item.id === draft.ai_bot_id)"
              :value="draft.ai_bot_id"
              disabled
            >
              {{ tx('unavailableAiBot') }}
            </option>
            <option v-for="item in aiBots" :key="item.id" :value="item.id">{{ item.name }}</option>
          </select>
          <p class="mt-2 text-xs leading-relaxed text-muted-foreground">
            {{ aiBotsLoading ? tx('aiBotsLoading') : aiBots.length ? tx('aiBotHint') : tx('noAiBots') }}
          </p>
        </div>
        <details class="rounded-xl border border-border/70 p-4">
          <summary class="cursor-pointer text-sm font-medium">{{ tx('advanced') }}</summary>
          <div class="mt-4 space-y-4">
            <label class="block text-sm"
              >{{ tx('credential') }}
              <select v-model="draft.credential_kind" :class="field" data-testid="telegram-credential-kind">
                <option value="vault">{{ tx('vault') }}</option>
                <option value="environment">{{ tx('environment') }}</option>
              </select>
            </label>
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
