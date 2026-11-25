import { ref } from 'vue'
import type PickerTrigger from './PickerTrigger.vue'

/** Wire a PickerTrigger to an OptionMenu without prescribing the menu's data or actions. */
export function usePickerMenu() {
  const trigger = ref<InstanceType<typeof PickerTrigger> | null>(null)
  const open = ref(false)
  const openAt = ref<'first' | 'last'>()
  const query = ref('')

  function setOpen(value: boolean) {
    open.value = value
    if (!value) {
      openAt.value = undefined
      query.value = ''
    }
  }

  function openFrom(edge: 'first' | 'last') {
    openAt.value = edge
    setOpen(true)
  }

  return { trigger, open, openAt, query, setOpen, openFrom }
}
