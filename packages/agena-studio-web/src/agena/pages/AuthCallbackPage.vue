<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'

const route = useRoute()

const code = computed(() => (typeof route.query.code === 'string' ? route.query.code.trim() : ''))
const state = computed(() => (typeof route.query.state === 'string' ? route.query.state.trim() : ''))
const error = computed(() => (typeof route.query.error === 'string' ? route.query.error.trim() : ''))
const payload = computed(() => ({
  type: 'agena-auth-callback',
  code: code.value,
  state: state.value,
  error: error.value,
}))

if (typeof window !== 'undefined') {
  try {
    if (window.opener && !window.opener.closed) {
      window.opener.postMessage(payload.value, window.location.origin)
    }
  } catch {
    // ignore opener messaging failures
  }
}
</script>

<template>
  <section class="page">
    <header class="page-header">
      <div>
        <h1 class="page-title">Authentication Callback</h1>
        <p class="page-description">This window can be closed after returning the authorization result to Agena Studio.</p>
      </div>
    </header>

    <section class="card">
      <div v-if="error" class="muted mono">error={{ error }}</div>
      <div v-else class="muted mono">code={{ code || 'missing' }} · state={{ state || 'missing' }}</div>
    </section>
  </section>
</template>
