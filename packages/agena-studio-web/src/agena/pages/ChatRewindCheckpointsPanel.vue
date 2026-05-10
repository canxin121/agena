<script setup lang="ts">
const props = defineProps<{
  selectedSessionId: number | null
  rewindCheckpointFacts: Array<{
    key: string
    label: string
    summary: string
  }>
  loadRewindCheckpoints: (sessionId: number) => void | Promise<void>
}>()
</script>

<template>
  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3 style="margin: 0">Rewind Checkpoints</h3>
        <p class="muted">Inspect available rewind anchors before rolling the session back.</p>
      </div>
      <div class="button-row">
        <button
          class="button ghost"
          :disabled="!props.selectedSessionId"
          @click="props.selectedSessionId && props.loadRewindCheckpoints(props.selectedSessionId)"
        >
          Reload Checkpoints
        </button>
      </div>
    </div>
    <div v-if="props.rewindCheckpointFacts.length" class="list">
      <div v-for="item in props.rewindCheckpointFacts" :key="item.key" class="list-item">
        <div>
          <strong>{{ item.label }}</strong>
        </div>
        <div class="muted mono">{{ item.summary }}</div>
      </div>
    </div>
    <p v-else class="muted">No rewind checkpoints are available for the active session.</p>
  </section>
</template>
