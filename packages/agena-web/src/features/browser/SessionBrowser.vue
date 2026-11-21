<script setup lang="ts">
import { computed, ref, shallowRef, watch, watchEffect } from 'vue'
import { RiHand, RiRefreshLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import { apiUrl } from '@/lib/api'
import { sessionStorageKeys } from '@/lib/persistence/storageKeys'
import { readServerAuthTokenVersion, subscribeServerAuthTokenVersion } from '@/lib/serverAuthToken'
import { useChatStore } from '@/stores/chat'
import BrowserViewport from './BrowserViewport.vue'
import { browserCommand, readBrowserState, type BrowserFrame, type BrowserInput, type BrowserState } from './api'
import { useBrowserCopy } from './copy'

const props = defineProps<{ open: boolean; sessionId: string; requestId?: string }>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()
const chat = useChatStore()
const t = useBrowserCopy()
const state = shallowRef<BrowserState>()
const frame = shallowRef<BrowserFrame | null>(null)
const pageId = ref('')
const busy = ref(false)
const error = ref('')
const previewError = ref('')
const controlBySession = new Map<string, string>()
const controlId = ref('')
const authVersion = ref(readServerAuthTokenVersion())
const completed = ref<{ id: string; discard: boolean }>()
const answer = computed(() => {
  const id = props.requestId
  if (!id) return
  if (completed.value?.id === id) return completed.value
  if (state.value?.help?.ready_to_resume) return { id, discard: false }
})
const canInput = computed(() => Boolean(controlId.value && pageId.value && frame.value) && !busy.value)
let inputQueue: BrowserInput[] = []
let sending: Promise<void> | undefined
let inputTimer: ReturnType<typeof setTimeout> | undefined
let generation = 0
let controlRevision = 0
let inputFailures = 0

function controlKey(sessionId: string) {
  return apiUrl('/') + '#' + sessionId
}

try {
  const saved: unknown = JSON.parse(sessionStorage.getItem(sessionStorageKeys.browser.control) || '[]')
  if (Array.isArray(saved)) {
    for (const pair of saved) {
      if (Array.isArray(pair) && typeof pair[0] === 'string' && typeof pair[1] === 'string') {
        controlBySession.set(pair[0], pair[1])
      }
    }
  }
} catch {
  // Browsers that disable session storage can still take control in memory.
}

function saveControls() {
  try {
    sessionStorage.setItem(sessionStorageKeys.browser.control, JSON.stringify([...controlBySession]))
  } catch {
    // The active view still owns its control grant.
  }
}

function resetInputs() {
  inputQueue = []
  if (inputTimer !== undefined) clearTimeout(inputTimer)
  inputTimer = undefined
}

function message(error: unknown) {
  return error instanceof Error ? error.message : t('failed')
}

function poll(interval: number, task: (signal: AbortSignal) => Promise<void>) {
  const controller = new AbortController()
  let timer: ReturnType<typeof setTimeout> | undefined
  const next = async () => {
    if (controller.signal.aborted) return
    try {
      await task(controller.signal)
    } finally {
      if (!controller.signal.aborted) timer = setTimeout(next, interval)
    }
  }
  void next()
  return () => {
    controller.abort()
    if (timer !== undefined) clearTimeout(timer)
  }
}

watch(
  () => props.sessionId,
  (sessionId) => {
    ++generation
    resetInputs()
    state.value = undefined
    pageId.value = ''
    frame.value = null
    controlId.value = controlBySession.get(controlKey(sessionId)) || ''
    completed.value = undefined
    error.value = ''
    busy.value = false
  },
  { immediate: true, flush: 'sync' },
)

watchEffect((cleanup) => {
  cleanup(
    subscribeServerAuthTokenVersion((version) => {
      ++generation
      controlBySession.clear()
      saveControls()
      controlId.value = ''
      frame.value = null
      state.value = undefined
      completed.value = undefined
      error.value = ''
      busy.value = false
      resetInputs()
      authVersion.value = version
    }),
  )
})

watch(
  [() => props.open, () => props.sessionId, authVersion],
  ([open, sid], _, cleanup) => {
    frame.value = null
    if (!open || !sid) return
    if (!controlId.value) pageId.value = ''
    cleanup(
      poll(1200, async (signal) => {
        const revision = controlRevision
        try {
          const result = await readBrowserState(sid, signal)
          if (signal.aborted || revision !== controlRevision) return
          const oldTabs = new Set(state.value?.tabs.map((tab) => tab.page_id) || [])
          const newHelp = result.help && result.help.page_id !== state.value?.help?.page_id
          state.value = result
          if ((newHelp && !controlId.value) || !result.tabs.some((tab) => tab.page_id === pageId.value)) {
            const requested = result.tabs.find((tab) => tab.page_id === result.help?.page_id)
            pageId.value = requested?.page_id || result.tabs[0]?.page_id || ''
          } else if (controlId.value) {
            const popup = result.tabs.find((tab) => tab.opener_page_id === pageId.value && !oldTabs.has(tab.page_id))
            if (popup && !sending && inputQueue.length === 0) pageId.value = popup.page_id
          }
          if (!result.human_control && !busy.value && controlId.value) {
            controlBySession.delete(controlKey(sid))
            saveControls()
            controlId.value = ''
            resetInputs()
          }
          if (result.tabs.length === 0) previewError.value = ''
        } catch (cause) {
          if (!signal.aborted) previewError.value = message(cause)
        }
      }),
    )
  },
  { immediate: true },
)

watch(
  [() => props.open, () => props.sessionId, pageId, controlId, () => state.value?.human_control, authVersion],
  ([open, sid, page, control, human], _, cleanup) => {
    frame.value = null
    previewError.value = ''
    if (!open || !page || (human && !control)) return
    cleanup(
      poll(180, async (signal) => {
        try {
          const result = await browserCommand(
            sid,
            { kind: 'frame', page_id: page, control_id: control || undefined },
            signal,
          )
          if (!signal.aborted && result.kind === 'frame') {
            frame.value = result
            previewError.value = ''
          }
        } catch (cause) {
          if (!signal.aborted) previewError.value = message(cause)
        }
      }),
    )
  },
  { immediate: true },
)

function sendInputBatch() {
  if (inputTimer !== undefined) clearTimeout(inputTimer)
  inputTimer = undefined
  if (sending || !inputQueue.length) return sending
  const sid = props.sessionId
  const control = controlId.value
  const page = pageId.value
  const current = generation
  if (!control || !page) {
    resetInputs()
    return
  }
  const events: BrowserInput[] = []
  // UTF-8 JSON can expand due to quoting and escaping. Keep pasted text in its
  // own request so batching pointer events cannot exceed the API body limit.
  while (events.length < 16 && inputQueue.length) {
    if (inputQueue[0].kind === 'text' && events.length) break
    const event = inputQueue.shift()!
    events.push(event)
    if (event.kind === 'text') break
  }
  sending = browserCommand(sid, { kind: 'input', page_id: page, control_id: control, events })
    .then(() => {})
    .catch((cause) => {
      if (current === generation) {
        ++inputFailures
        error.value = message(cause)
        resetInputs()
      }
    })
    .finally(() => {
      sending = undefined
      if (inputQueue.length) void sendInputBatch()
    })
  return sending
}

function queueInput(event: BrowserInput) {
  if (!controlId.value || !pageId.value || busy.value) return
  const previous = inputQueue.at(-1)
  if (
    event.kind === 'pointer' &&
    event.action === 'move' &&
    previous?.kind === 'pointer' &&
    previous.action === 'move'
  ) {
    inputQueue[inputQueue.length - 1] = event
  } else if (inputQueue.length < 128) {
    inputQueue.push(event)
  } else {
    error.value = t('failed')
    return
  }
  if (inputTimer === undefined && !sending) inputTimer = setTimeout(sendInputBatch, 16)
}

async function flushInputs() {
  while (sending || inputQueue.length) await sendInputBatch()
}

async function selectPage(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  const current = generation
  await flushInputs()
  if (current === generation) pageId.value = value
}

async function takeControl() {
  if (!pageId.value || busy.value) return
  const sid = props.sessionId
  const version = authVersion.value
  const current = generation
  const key = controlKey(sid)
  busy.value = true
  ++controlRevision
  error.value = ''
  try {
    const result = await browserCommand(sid, { kind: 'take_control', page_id: pageId.value })
    if (result.kind !== 'control' || version !== authVersion.value) return
    ++controlRevision
    controlBySession.set(key, result.control_id)
    saveControls()
    if (current === generation) {
      controlId.value = result.control_id
      if (state.value) state.value = { ...state.value, human_control: true }
    }
  } catch (cause) {
    if (current === generation) error.value = message(cause)
  } finally {
    if (current === generation) busy.value = false
  }
}

async function answerRequest() {
  const request = answer.value
  if (!request) return
  if (request.discard) await chat.rejectQuestion(props.sessionId, request.id)
  else await chat.replyQuestion(props.sessionId, request.id, [['Resume browser task']])
  if (completed.value === request) completed.value = undefined
}

async function endControl(discard: boolean) {
  const control = controlId.value
  if (!control || busy.value) return
  const sid = props.sessionId
  const requestId = props.requestId
  const version = authVersion.value
  const current = generation
  const key = controlKey(sid)
  busy.value = true
  ++controlRevision
  error.value = ''
  try {
    const failures = inputFailures
    await flushInputs()
    if (version !== authVersion.value || current !== generation || (!discard && inputFailures !== failures)) return
    await browserCommand(
      sid,
      discard
        ? { kind: 'close', control_id: control }
        : { kind: 'return_control', control_id: control, page_id: pageId.value },
    )
    if (version !== authVersion.value) return
    ++controlRevision
    controlBySession.delete(key)
    saveControls()
    if (current !== generation) return
    controlId.value = ''
    frame.value = null
    if (state.value)
      state.value = { ...state.value, human_control: false, help: null, tabs: discard ? [] : state.value.tabs }
    if (requestId) {
      completed.value = { id: requestId, discard }
      await answerRequest()
    }
  } catch (cause) {
    if (current === generation) error.value = message(cause)
  } finally {
    if (current === generation) busy.value = false
  }
}

async function retryAnswer() {
  if (busy.value) return
  const current = generation
  busy.value = true
  error.value = ''
  try {
    await answerRequest()
  } catch (cause) {
    if (current === generation) error.value = message(cause)
  } finally {
    if (current === generation) busy.value = false
  }
}
</script>

<template>
  <FormDialog
    :open="open"
    :title="t('browser')"
    :description="t('signInHere')"
    max-width="max-w-5xl"
    @update:open="emit('update:open', $event)"
  >
    <div class="space-y-3" data-testid="session-browser">
      <div v-if="state?.help" role="status" class="rounded-md border border-primary/30 bg-primary/5 p-2.5">
        <p class="text-sm font-semibold">{{ t('requested') }}</p>
        <p class="mt-1 break-words text-sm">{{ state.help.reason }}</p>
      </div>
      <p
        v-if="error || previewError"
        role="alert"
        class="rounded-md border border-destructive/40 px-2.5 py-2 text-sm text-destructive"
      >
        {{ error || previewError }}
      </p>
      <div v-if="state?.tabs.length" class="flex flex-wrap items-center justify-between gap-2">
        <label class="flex min-w-0 max-w-full items-center gap-2 text-sm">
          <span class="sr-only">{{ t('tabs') }}</span>
          <select
            :value="pageId"
            :disabled="busy"
            class="max-w-full rounded-md border border-border bg-background px-2 py-1"
            @change="selectPage"
          >
            <option v-for="tab in state.tabs" :key="tab.page_id" :value="tab.page_id">{{ tab.title }}</option>
          </select>
        </label>
        <span aria-live="polite" class="text-xs text-muted-foreground">
          {{ controlId ? t('youControl') : state.human_control ? t('otherView') : t('previewOnly') }}
        </span>
      </div>
      <p v-if="frame?.origin" class="truncate font-mono text-xs text-muted-foreground" :title="frame.origin">
        {{ frame.origin }}
      </p>
      <BrowserViewport
        v-if="pageId && (!state?.human_control || controlId)"
        :key="pageId"
        :frame="frame"
        :interactive="canInput"
        @input="queueInput"
      />
      <p v-else-if="!state" role="status" class="py-5 text-sm text-muted-foreground">{{ t('loading') }}</p>
      <p v-else-if="!state.tabs.length" class="py-5 text-sm text-muted-foreground">{{ t('empty') }}</p>
      <div class="flex flex-wrap justify-end gap-2">
        <Button
          v-if="answer && !controlId"
          type="button"
          variant="outline"
          size="sm"
          :disabled="busy"
          @click="retryAnswer"
        >
          <RiRefreshLine aria-hidden="true" />{{ answer.discard ? t('cancelRequest') : t('resume') }}
        </Button>
        <template v-if="controlId">
          <Button size="sm" variant="outline" :disabled="busy" @click="endControl(true)">{{ t('discard') }}</Button>
          <Button size="sm" :disabled="busy" data-testid="browser-return-control" @click="endControl(false)">
            {{ t('giveBack') }}
          </Button>
        </template>
        <Button
          v-else-if="pageId && !state?.human_control"
          size="sm"
          :disabled="busy"
          data-testid="browser-take-control"
          @click="takeControl"
          ><RiHand aria-hidden="true" />{{ t('take') }}</Button
        >
      </div>
    </div>
  </FormDialog>
</template>
