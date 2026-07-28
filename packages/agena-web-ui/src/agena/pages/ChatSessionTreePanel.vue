<script setup lang="ts">
import type { SessionTreeResource } from '@/agena/lib/agenaApi'

const props = defineProps<{
  selectedSessionId: number | null
  sessionTreeRows: Array<{
    session: SessionTreeResource
    depth: number
  }>
  ancestorSessions: Array<{ id: number }>
  loadSessionTree: (rootId: number) => void | Promise<void>
  selectSession: (sessionId: number) => void | Promise<void>
}>()
</script>

<template>
  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3 style="margin: 0">Session Tree</h3>
        <p class="muted">Browse branch lineage from the current root and jump directly across forks.</p>
      </div>
      <div class="button-row">
        <button
          class="button ghost"
          :disabled="!props.selectedSessionId"
          @click="
            props.selectedSessionId && props.loadSessionTree(props.ancestorSessions[0]?.id ?? props.selectedSessionId)
          "
        >
          Reload Tree
        </button>
      </div>
    </div>
    <div v-if="props.sessionTreeRows.length" class="list">
      <button
        v-for="row in props.sessionTreeRows"
        :key="`tree-${row.session.id}`"
        class="list-item"
        :class="{ active: row.session.id === props.selectedSessionId }"
        :style="{ paddingLeft: `${14 + row.depth * 18}px` }"
        @click="props.selectSession(row.session.id)"
      >
        <div>
          <strong>#{{ row.session.id }} {{ row.session.title }}</strong>
        </div>
        <div class="muted mono">
          parent={{ row.session.parent_id ?? 'root' }} · messages={{ row.session.message_count }} · children={{
            row.session.child_session_count
          }}
        </div>
      </button>
    </div>
    <p v-else class="muted">No session tree is loaded yet.</p>
  </section>
</template>
