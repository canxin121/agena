import { computed, reactive, ref, type Ref } from 'vue'

import { deleteMemory, listMemories, saveMemory, type MemoryResource, type MemoryType } from '../lib/agenaApi'

export type SettingsMemoryStateDeps = {
  deleteMemory: typeof deleteMemory
  listMemories: typeof listMemories
  saveMemory: typeof saveMemory
}

const defaultDeps: SettingsMemoryStateDeps = { deleteMemory, listMemories, saveMemory }

export function useSettingsMemoryState(
  status: { actionError: Ref<string>; actionMessage: Ref<string> },
  deps: SettingsMemoryStateDeps = defaultDeps,
) {
  const memories = ref<MemoryResource[]>([])
  const selectedName = ref('')
  const originalName = ref('')
  const loading = ref(false)
  const saving = ref(false)
  const search = ref('')
  const draft = reactive({
    name: '',
    description: '',
    memoryType: '' as MemoryType | '',
    body: '',
  })

  const filteredMemories = computed(() => {
    const query = search.value.trim().toLowerCase()
    if (!query) return memories.value
    return memories.value.filter((memory) =>
      [memory.name, memory.description, memory.memory_type || '', memory.body].join(' ').toLowerCase().includes(query),
    )
  })

  function replaceDraft(memory: MemoryResource | null) {
    selectedName.value = memory?.name || ''
    originalName.value = memory?.name || ''
    draft.name = memory?.name || ''
    draft.description = memory?.description || ''
    draft.memoryType = memory?.memory_type || ''
    draft.body = memory?.body || ''
  }

  function selectMemory(name: string) {
    replaceDraft(memories.value.find((memory) => memory.name === name) || null)
  }

  function createNewMemory() {
    replaceDraft(null)
  }

  async function load(preferredName?: string) {
    loading.value = true
    status.actionError.value = ''
    try {
      memories.value = await deps.listMemories()
      const target = preferredName || selectedName.value
      const selected = memories.value.find((memory) => memory.name === target) || memories.value[0] || null
      replaceDraft(selected)
    } catch (error) {
      status.actionError.value = error instanceof Error ? error.message : String(error)
    } finally {
      loading.value = false
    }
  }

  async function save() {
    const name = draft.name.trim().replace(/\.md$/i, '')
    if (!name || !draft.body.trim()) {
      status.actionError.value = 'Memory name and body are required.'
      return
    }
    saving.value = true
    status.actionError.value = ''
    status.actionMessage.value = ''
    try {
      const memory = await deps.saveMemory({
        name,
        description: draft.description.trim(),
        memoryType: draft.memoryType || null,
        body: draft.body,
      })
      status.actionMessage.value = `Saved memory ${memory.name}.`
      await load(memory.name)
    } catch (error) {
      status.actionError.value = error instanceof Error ? error.message : String(error)
    } finally {
      saving.value = false
    }
  }

  async function remove() {
    const name = originalName.value
    if (!name) return
    if (typeof window !== 'undefined' && !window.confirm(`Forget memory “${name}”?`)) return
    saving.value = true
    status.actionError.value = ''
    status.actionMessage.value = ''
    try {
      await deps.deleteMemory(name)
      status.actionMessage.value = `Forgot memory ${name}.`
      replaceDraft(null)
      await load()
    } catch (error) {
      status.actionError.value = error instanceof Error ? error.message : String(error)
    } finally {
      saving.value = false
    }
  }

  return {
    createNewMemory,
    draft,
    filteredMemories,
    load,
    loading,
    memories,
    originalName,
    remove,
    save,
    saving,
    search,
    selectMemory,
    selectedName,
  }
}
