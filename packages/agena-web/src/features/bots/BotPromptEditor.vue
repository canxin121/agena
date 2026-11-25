<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId } from 'vue'
import { RiAddLine, RiCloseLine, RiEditLine, RiFileTextLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import PromptOrderRow from '@/components/settings/PromptOrderRow.vue'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import {
  builtinPromptApplicability,
  builtinPromptTitle,
  readBuiltinPrompts,
  reusableBuiltinPrompts,
} from '@/components/settings/promptCatalog'
import {
  availableSavedPrompts,
  composedPromptText,
  promptExcerpt,
  savedPromptDisplayName,
  savedPromptExcerpt,
} from '@/lib/promptLibrary'
import { emptyPromptScopeConfig, normalizePromptScopeConfig, type BuiltinPrompt } from '@/lib/promptLayers'
import { getRuntimeSettingLayer } from '@/lib/runtimeSettings'
import { type BotPersona } from '@/lib/bots'
import { BOT_INSTRUCTIONS_ID, MAX_INSTRUCTIONS_BYTES, botPromptOrder, instructionsBytes } from './draft'

const props = defineProps<{ disabled?: boolean }>()
const persona = defineModel<BotPersona>({ required: true })
const { t } = useI18n()
const editorId = useId()
const catalog = ref<BuiltinPrompt[]>([])
const global = ref(emptyPromptScopeConfig())
const loading = ref(true)
const errors = ref<string[]>([])
const selected = ref('')
const editing = ref(false)
const bodyInput = ref<HTMLTextAreaElement | null>(null)
const abort = new AbortController()
const byId = computed(
  () => new Map(catalog.value.filter((prompt) => !prompt.reusable).map((prompt) => [prompt.id, prompt])),
)
const saved = computed(() =>
  availableSavedPrompts(global.value, emptyPromptScopeConfig(), 'global', reusableBuiltinPrompts(catalog.value)),
)
const savedById = computed(() => new Map(saved.value.map(({ id, fragment }) => [id, fragment])))
const order = computed(() => botPromptOrder(persona.value, catalog.value, global.value))
const libraryChoices = computed(() =>
  saved.value
    .filter(({ id }) => id !== BOT_INSTRUCTIONS_ID && !order.value.includes(id) && !global.value.disabled.includes(id))
    .map(({ id, fragment }) => ({
      value: id,
      label: savedPromptDisplayName(fragment),
      description: savedPromptExcerpt(fragment, savedById.value) || fragment.category,
      keywords: fragment.category,
    })),
)
const activeBuiltin = computed(() => byId.value.get(selected.value))
const activeSaved = computed(() => savedById.value.get(selected.value))
const activeDetail = computed(() =>
  activeBuiltin.value ? builtinPromptApplicability(activeBuiltin.value.when) : activeSaved.value?.category,
)
const activeBody = computed(
  () =>
    activeBuiltin.value?.body ?? (activeSaved.value ? composedPromptText(activeSaved.value, savedById.value) : null),
)
const byteCount = computed(() => instructionsBytes(persona.value.instructions))
const hasSharedOrder = computed(() => persona.value.prompts?.ids.some((id) => id !== BOT_INSTRUCTIONS_ID))
const publish = computed(() => persona.value.prompts?.publish_to_channels ?? false)
const explicitOrder = computed(() => persona.value.prompts?.ids ?? [])

onMounted(async () => {
  const results = await Promise.allSettled([
    readBuiltinPrompts(abort.signal),
    getRuntimeSettingLayer('global', 'prompts', { signal: abort.signal }),
  ] as const)
  if (abort.signal.aborted) return
  const [builtins, settings] = results
  if (builtins.status === 'fulfilled') catalog.value = builtins.value
  if (settings.status === 'fulfilled') global.value = normalizePromptScopeConfig(settings.value.value)
  errors.value = results.flatMap((result) => (result.status === 'rejected' ? [String(result.reason)] : []))
  loading.value = false
})
onBeforeUnmount(() => abort.abort())

function title(id: string) {
  const builtin = byId.value.get(id)
  if (builtin) return builtinPromptTitle(builtin)
  if (id === BOT_INSTRUCTIONS_ID) return t('bot.profiles.instructions')
  const prompt = savedById.value.get(id)
  return prompt ? savedPromptDisplayName(prompt) : id
}

function excerpt(id: string) {
  const builtin = byId.value.get(id)
  if (builtin) return promptExcerpt(builtin.body)
  if (id === BOT_INSTRUCTIONS_ID) return promptExcerpt(persona.value.instructions)
  const prompt = savedById.value.get(id)
  return prompt ? savedPromptExcerpt(prompt, savedById.value) || prompt.category : ''
}

function updateOrder(ids: string[], publishToChannels = publish.value) {
  persona.value = { ...persona.value, prompts: { ids, publish_to_channels: publishToChannels } }
}

function select(id: string) {
  if (selected.value === id) {
    selected.value = ''
    editing.value = false
    return
  }
  selected.value = id
  editing.value = id === BOT_INSTRUCTIONS_ID
}

async function editInstructions() {
  selected.value = BOT_INSTRUCTIONS_ID
  editing.value = true
  await nextTick()
  bodyInput.value?.focus()
}

function updateInstructions(value: string) {
  const ids = explicitOrder.value.filter((id) => id !== BOT_INSTRUCTIONS_ID || value.trim())
  persona.value = {
    ...persona.value,
    instructions: value,
    ...(persona.value.prompts ? { prompts: { ids, publish_to_channels: publish.value } } : {}),
  }
}

function add(id: string) {
  if (props.disabled || !libraryChoices.value.some((item) => item.value === id)) return
  const ids = explicitOrder.value.slice()
  if (persona.value.instructions.trim() && !ids.includes(BOT_INSTRUCTIONS_ID)) ids.push(BOT_INSTRUCTIONS_ID)
  updateOrder([...ids, id])
  selected.value = id
  editing.value = false
}

function isRequired(id: string) {
  return byId.value.has(id) || id.startsWith('agena.system.')
}

function remove(id: string) {
  if (props.disabled || isRequired(id)) return
  if (id === BOT_INSTRUCTIONS_ID) updateInstructions('')
  else updateOrder(explicitOrder.value.filter((entry) => entry !== id))
  if (selected.value === id) {
    selected.value = ''
    editing.value = false
  }
}

function move(index: number, delta: number) {
  if (props.disabled || loading.value || errors.value.length) return
  const other = index + delta
  if (other < 0 || other >= order.value.length) return
  const ids = order.value.slice()
  const [moved] = ids.splice(index, 1)
  if (!moved) return
  ids.splice(other, 0, moved)
  updateOrder(ids)
}
</script>

<template>
  <section
    class="min-w-0 rounded-lg border border-border/70 bg-card/35"
    :aria-busy="loading"
    data-testid="bot-prompt-order"
  >
    <header class="flex min-w-0 flex-wrap items-center justify-between gap-2 border-b border-border/60 px-3 py-2">
      <h3 class="min-w-0 text-sm font-semibold">{{ $st('System prompt') }}</h3>
      <div class="flex min-w-0 flex-wrap items-center gap-1.5">
        <Button
          v-if="!editing"
          type="button"
          size="sm"
          variant="ghost"
          :disabled="disabled"
          data-testid="bot-add-instructions"
          @click="editInstructions"
        >
          <component
            :is="persona.instructions.trim() ? RiEditLine : RiAddLine"
            class="me-1 h-4 w-4"
            aria-hidden="true"
          />
          {{ t('bot.profiles.instructions') }}
        </Button>
        <OptionPicker
          model-value=""
          :options="libraryChoices"
          :title="$st('Prompt library')"
          :placeholder="$st('Add prompt')"
          :search-placeholder="$st('Search prompts…')"
          :include-empty="false"
          :disabled="disabled || loading || !libraryChoices.length"
          :icon="RiAddLine"
          size="sm"
          data-testid="bot-add-prompt"
          @update:model-value="add"
        />
      </div>
    </header>
    <p v-if="loading" role="status" class="px-3 py-2 text-xs text-muted-foreground">{{ $st('Loading…') }}</p>
    <div v-keyboard-collection class="max-h-64 overflow-y-auto overscroll-contain">
      <ol v-if="order.length" class="space-y-0.5 p-1.5">
        <PromptOrderRow
          v-for="(id, index) in order"
          :key="id"
          :row-id="id"
          :data-bot-prompt-id="id"
          :title="title(id)"
          :excerpt="excerpt(id)"
          :position="index + 1"
          selection="expanded"
          :selected="selected === id"
          reorderable
          removable
          :can-move-up="!loading && !errors.length && index > 0"
          :can-move-down="!loading && !errors.length && index + 1 < order.length"
          :disabled="disabled"
          :locked="isRequired(id)"
          @select="select(id)"
          @move="move(index, $event)"
          @remove="remove(id)"
        />
      </ol>
    </div>
    <div v-if="selected" class="min-w-0 space-y-2 border-t border-border/60 bg-muted/15 p-3">
      <div class="flex min-w-0 items-center gap-2 text-sm font-medium">
        <RiFileTextLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <label v-if="editing" :for="editorId">{{ title(selected) }}</label>
        <div v-else class="min-w-0 break-words">
          <h4>{{ title(selected) }}</h4>
          <p v-if="activeDetail" class="text-xs font-normal text-muted-foreground">{{ activeDetail }}</p>
        </div>
        <IconButton
          size="sm"
          class="ms-auto"
          :aria-label="$st('Close')"
          :tooltip="$st('Close')"
          @click="select(selected)"
        >
          <RiCloseLine class="h-4 w-4" aria-hidden="true" />
        </IconButton>
      </div>
      <template v-if="editing">
        <textarea
          :id="editorId"
          ref="bodyInput"
          :value="persona.instructions"
          rows="5"
          spellcheck="false"
          class="min-h-28 w-full resize-y rounded-md border border-input bg-background px-3 py-2 text-sm leading-relaxed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          data-testid="bot-edit-instructions"
          :aria-describedby="`${editorId}-count`"
          :aria-invalid="byteCount > MAX_INSTRUCTIONS_BYTES"
          :disabled="disabled"
          @input="updateInstructions(($event.target as HTMLTextAreaElement).value)"
        />
        <p
          :id="`${editorId}-count`"
          class="text-xs"
          :class="byteCount > MAX_INSTRUCTIONS_BYTES ? 'text-destructive' : 'text-muted-foreground'"
          data-testid="bot-instructions-count"
        >
          {{ t('bot.profiles.instructionsCount', { count: byteCount, max: MAX_INSTRUCTIONS_BYTES }) }}
        </p>
      </template>
      <pre
        v-else-if="activeBody !== null"
        class="max-h-56 overflow-auto whitespace-pre-wrap font-mono text-sm leading-relaxed [overflow-wrap:anywhere]"
        >{{ activeBody }}</pre
      >
      <p v-else class="text-xs text-muted-foreground">
        {{ $st('Open the saved session preview to inspect the resolved text.') }}
      </p>
    </div>
    <label v-if="hasSharedOrder" class="flex items-center gap-2 border-t border-border/60 px-3 py-2 text-sm">
      <input
        type="checkbox"
        :checked="publish"
        :disabled="disabled"
        @change="updateOrder(explicitOrder, ($event.target as HTMLInputElement).checked)"
      />
      {{ $st('Use the selected library prompts and order in connected channels') }}
    </label>
    <p
      v-for="(error, index) in errors"
      :key="index"
      role="alert"
      class="break-words px-3 py-2 text-xs text-destructive"
    >
      {{ error }}
    </p>
  </section>
</template>
