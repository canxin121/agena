import { computed, onMounted, onScopeDispose, ref, watch, type Ref } from 'vue'
import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'
import { errorPartHeadline } from '@/pages/chat/errorPartPresentation'
import { usePaneVisibility } from './usePaneVisibility'

export function useErrorPartHeadline(part: Ref<TranscriptDisplayPart>) {
  const visible = usePaneVisibility()
  const mounted = ref(false)
  const now = ref(Date.now())
  const headline = computed(() => (part.value.kind === 'error' ? errorPartHeadline(part.value, now.value) : null))
  let timer: ReturnType<typeof setInterval> | undefined
  const stop = () => {
    if (timer !== undefined) clearInterval(timer)
    timer = undefined
  }
  watch(
    [() => mounted.value && visible.value && headline.value?.clockActive, () => part.value],
    ([active]) => {
      stop()
      now.value = Date.now()
      if (active)
        timer = setInterval(() => {
          now.value = Date.now()
        }, 1000)
    },
    { immediate: true },
  )
  onMounted(() => {
    mounted.value = true
  })
  onScopeDispose(stop)
  return headline
}
