import { userErrorMessage } from '@/lib/api'
import { defineStore } from 'pinia'
import { ref } from 'vue'

import { fetchStudioHealth, type StudioHealth } from '@/agena/lib/agenaApi'

export const useHealthStore = defineStore('health', () => {
  const data = ref<StudioHealth | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  async function refresh() {
    loading.value = true
    error.value = null
    try {
      data.value = await fetchStudioHealth()
    } catch (err) {
      error.value = userErrorMessage(err)
      data.value = null
    } finally {
      loading.value = false
    }
  }

  return {
    data,
    loading,
    error,
    refresh,
  }
})
