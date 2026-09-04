import { nextTick, type Ref } from 'vue'

type PickerKind = 'model' | 'thinking' | 'speed'

export function useModelSelectionPickerUi(opts: {
  composerPickerOpen: Ref<null | PickerKind>
  modelPickerQuery: Ref<string>
  onOpenComposerPicker: () => void
  commandOpen: Ref<boolean>
  commandQuery: Ref<string>
  commandIndex: Ref<number>
}) {
  const {
    composerPickerOpen,
    modelPickerQuery,
    onOpenComposerPicker,
    commandOpen,
    commandQuery,
    commandIndex,
  } = opts

  function closeComposerPicker() {
    composerPickerOpen.value = null
  }

  let pickerToggleSeq = 0

  async function toggleComposerPicker(kind: PickerKind) {
    const seq = ++pickerToggleSeq
    if (composerPickerOpen.value === kind) {
      composerPickerOpen.value = null
      return
    }

    onOpenComposerPicker()
    commandOpen.value = false
    commandQuery.value = ''
    commandIndex.value = 0

    // Close the previous picker branch for one render turn before opening the
    // next one. Positioning now belongs entirely to OptionMenu/Popper.
    if (composerPickerOpen.value) composerPickerOpen.value = null

    await nextTick()
    if (seq !== pickerToggleSeq) return

    composerPickerOpen.value = kind
    if (kind === 'model') modelPickerQuery.value = ''
  }

  return {
    closeComposerPicker,
    toggleComposerPicker,
  }
}
