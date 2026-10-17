import { afterEach, expect, test } from 'bun:test'
import { computed, createRenderer, defineComponent, nextTick, ref } from 'vue'
import { i18n } from '../src/i18n'
import type { TranscriptDisplayPart } from '../src/components/chat/messageList.types'
import { errorPartHeadline, retryDuration } from '../src/pages/chat/errorPartPresentation'
import { useErrorPartHeadline } from '../src/composables/useErrorPartHeadline'
import { workspacePaneContextKey, type WorkspacePaneContext } from '../src/app/workspace/workspacePaneContext'

const previousLocale = i18n.global.locale.value
afterEach(() => {
  i18n.global.locale.value = previousLocale
})

function errorPart(retry: Record<string, unknown> = {}): TranscriptDisplayPart {
  return {
    key: 'part:102',
    id: '102',
    kind: 'error',
    status: 'failed',
    role: 'assistant',
    title: 'Error',
    summary: '',
    copyText: '',
    toggleable: true,
    defaultExpanded: false,
    source: {
      id: '102',
      agenaKind: 'error',
      agenaContent: {
        message: 'The provider request timed out.',
        retry: { attempt: 2, max_retries: 10, outcome: 'retrying', phase: 'backoff', next_at_ms: 15_001, ...retry },
      },
    },
  }
}

test('collapsed error headline counts down from the server deadline without inferring a request start', () => {
  i18n.global.locale.value = 'en-US'
  const part = errorPart()
  expect(errorPartHeadline(part, 12_000)).toMatchObject({
    title: 'Waiting to retry',
    progress: 'Retry 2/10',
    timing: 'Retry in 4s',
    clockActive: true,
  })
  expect(errorPartHeadline(part, 14_001).timing).toBe('Retry in 1s')
  const expired = errorPartHeadline(part, 15_001)
  expect(expired.timing).toBe('Waiting for retry to start')
  expect(expired.clockActive).toBe(false)
  expect(expired.status.spinning).toBe(false)
  expect(errorPartHeadline(errorPart({ phase: undefined }), 90_000).timing).toBe('Waiting for retry to start')
})

test('request and response phases show elapsed time; terminal history has no live clock', () => {
  i18n.global.locale.value = 'en-US'
  const requesting = errorPart({ phase: 'requesting', started_at_ms: 15_001, next_at_ms: undefined })
  expect(errorPartHeadline(requesting, 27_001)).toMatchObject({
    title: 'Retrying',
    timing: 'Waiting for response · 12s elapsed',
    clockActive: true,
    status: { spinning: true },
  })
  expect(errorPartHeadline(requesting, 10_000).timing).toBe('Waiting for response · 0s elapsed')
  expect(errorPartHeadline(errorPart({ phase: 'streaming', started_at_ms: 15_001 }), 27_001).timing).toBe(
    'Receiving response · 12s elapsed',
  )
  for (const [outcome, title, text] of [
    ['recovered', 'Recovered', 'Recovered after 2 retries'],
    ['failed', 'Retry failed', 'Retry 2/10'],
    ['cancelled', 'Retry cancelled', 'Retry 2/10'],
  ]) {
    const header = errorPartHeadline(errorPart({ outcome }), 12_000)
    expect(header.title).toBe(title)
    expect(header.progress).toBe(text)
    expect(header.clockActive).toBe(false)
    expect(header.status.spinning).toBe(false)
  }
  expect(errorPartHeadline(errorPart({ attempt: 10, outcome: 'failed' }), 12_000).timing).toBe('All 10 retries used')
  expect(errorPartHeadline(errorPart({ outcome: 'failed' }), 12_000).timing).toBe('Automatic retry stopped')
  expect(retryDuration(65)).toBe('1m 5s')
  expect(retryDuration(3661)).toBe('1h 1m')
})

test('Chinese progress is readable, and invalid or absent retry fields preserve ordinary error presentation', () => {
  i18n.global.locale.value = 'zh-CN'
  expect(errorPartHeadline(errorPart(), 12_000)).toMatchObject({
    title: '等待重试',
    progress: '第 2/10 次重试',
    timing: '4 秒后重试',
  })
  const legacy = errorPart({ outcome: undefined })
  expect(errorPartHeadline(legacy, 12_000)).toMatchObject({
    title: 'Error',
    message: 'The provider request timed out.',
    progress: '',
    clockActive: false,
  })
  expect(errorPartHeadline(errorPart({ next_at_ms: 'invalid', attempt: -1, max_retries: null }), 12_000)).toMatchObject(
    { progress: '第 0 次重试', timing: '等待重试启动', clockActive: false },
  )
})

test('live headline clock pauses in hidden panes, catches up from timestamps, and cleans up on recovery/unmount', async () => {
  i18n.global.locale.value = 'en-US'
  const previousNow = Date.now
  const previousInterval = globalThis.setInterval
  const previousClear = globalThis.clearInterval
  const previousDocument = globalThis.document
  let now = 12_000
  const timers = new Map<number, () => void>()
  let timerId = 0
  Date.now = () => now
  globalThis.setInterval = ((callback: () => void) => {
    timers.set(++timerId, callback)
    return timerId
  }) as unknown as typeof setInterval
  globalThis.clearInterval = ((id: number) => {
    timers.delete(id)
  }) as unknown as typeof clearInterval
  const doc = Object.assign(new EventTarget(), { hidden: false, visibilityState: 'visible' })
  globalThis.document = doc as unknown as Document
  const visible = ref(true)
  const part = ref(errorPart())
  let headline!: ReturnType<typeof useErrorPartHeadline>
  const renderer = createRenderer<object, object>({
    createElement: () => ({}),
    createText: () => ({}),
    createComment: () => ({}),
    insert() {},
    remove() {},
    setText() {},
    setElementText() {},
    patchProp() {},
    parentNode: () => null,
    nextSibling: () => null,
  })
  const app = renderer.createApp(
    defineComponent({
      setup() {
        headline = useErrorPartHeadline(part)
        return () => null
      },
    }),
  )
  app.provide(workspacePaneContextKey, { isVisible: computed(() => visible.value) } as WorkspacePaneContext)
  try {
    app.mount({})
    await nextTick()
    expect(timers.size).toBe(1)
    now = 14_001
    for (const tick of [...timers.values()]) tick()
    await nextTick()
    expect(headline.value?.timing).toBe('Retry in 1s')
    visible.value = false
    await nextTick()
    expect(timers.size).toBe(0)
    now = 20_000
    visible.value = true
    await nextTick()
    expect(headline.value?.timing).toBe('Waiting for retry to start')
    expect(timers.size).toBe(0)
    part.value = errorPart({ phase: 'requesting', started_at_ms: 16_000, next_at_ms: undefined })
    await nextTick()
    expect(timers.size).toBe(1)
    expect(headline.value?.timing).toBe('Waiting for response · 4s elapsed')
    doc.hidden = true
    doc.dispatchEvent(new Event('visibilitychange'))
    await nextTick()
    expect(timers.size).toBe(0)
    now = 31_000
    doc.hidden = false
    doc.dispatchEvent(new Event('visibilitychange'))
    await nextTick()
    expect(headline.value?.timing).toBe('Waiting for response · 15s elapsed')
    part.value = errorPart({ outcome: 'recovered' })
    await nextTick()
    expect(timers.size).toBe(0)
    part.value = errorPart({ phase: 'streaming', started_at_ms: 16_000 })
    await nextTick()
    expect(timers.size).toBe(1)
    app.unmount()
    expect(timers.size).toBe(0)
  } finally {
    app.unmount()
    Date.now = previousNow
    globalThis.setInterval = previousInterval
    globalThis.clearInterval = previousClear
    globalThis.document = previousDocument
  }
})
