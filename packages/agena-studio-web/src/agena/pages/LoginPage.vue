<script setup lang="ts">
import { computed, ref } from 'vue'

import { useAuthStore } from '@/stores/auth'

const auth = useAuthStore()
const password = ref('')
const pending = ref(false)

const errorMessage = computed(() => auth.lastError || '')

async function submit() {
  pending.value = true
  try {
    await auth.login(password.value)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <div class="boot-screen">
    <div class="panel">
      <div class="eyebrow">Agena Studio</div>
      <h1>Unlock UI</h1>
      <p>Enter the UI password configured for this studio instance.</p>

      <div class="field" style="margin-top: 18px">
        <label class="label" for="login-password">Password</label>
        <input
          id="login-password"
          v-model="password"
          class="input"
          type="password"
          autocomplete="current-password"
          @keyup.enter="submit"
        />
      </div>

      <p v-if="errorMessage" class="notice" style="margin-top: 16px">{{ errorMessage }}</p>

      <div class="button-row" style="margin-top: 18px">
        <button class="button primary" :disabled="pending || !password.trim()" @click="submit">
          {{ pending ? 'Signing in…' : 'Sign in' }}
        </button>
      </div>
    </div>
  </div>
</template>
