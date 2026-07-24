<script setup lang="ts">
import { computed } from 'vue'

import type { SessionExecutionResource, SessionResource, WorkspaceResource } from '@/agena/lib/agenaApi'

const props = defineProps<{
  selectedSession: SessionResource | null
  selectedWorkspace: WorkspaceResource | null
  selectedSessionId: number | null
  loading: boolean
  continuing: boolean
  sessionState: SessionExecutionResource | null
  sessionLineageLabel: string
  ancestorSessions: SessionResource[]
  executionFacts: string[]
  contextUsageLabel: string
  sessionUsageSummaryFacts: string[]
  siblingSessions: SessionResource[]
  childSessions: SessionResource[]
  parentSession: SessionResource | null
  selectSession: (sessionId: number) => void | Promise<void>
  renameCurrentSession: () => void | Promise<void>
  forkCurrentSession: () => void | Promise<void>
  deleteCurrentSession: () => void | Promise<void>
  exportCurrentSession: () => void | Promise<void>
  continueCurrentSession: () => void | Promise<void>
  cancelCurrentSessionRun: () => void | Promise<void>
  formatMessageTime: (value: string) => string
}>()

const activeGoal = computed(
  () => props.sessionState?.goal || props.sessionState?.session.goal || props.selectedSession?.goal || null,
)
</script>

<template>
  <section class="card">
    <h3>Active Session</h3>
    <div v-if="props.selectedSession">
      <div>
        <strong>{{ props.selectedSession.title }}</strong>
      </div>
      <div class="muted">workspace={{ props.selectedWorkspace?.path || 'unknown' }}</div>
      <div class="muted">{{ props.sessionLineageLabel }}</div>
      <div class="muted">
        workflow={{ props.sessionState?.workflow_state || 'unknown' }}, execution={{
          props.sessionState?.active_execution?.phase || 'inactive'
        }}
      </div>
      <div v-if="activeGoal" class="goal-summary">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <strong>{{ activeGoal.objective }}</strong>
          </div>
          <span class="badge">goal={{ activeGoal.status }}</span>
        </div>
      </div>
      <div class="button-row" style="margin-top: 8px">
        <button v-if="props.parentSession" class="button ghost" @click="props.selectSession(props.parentSession.id)">
          Open Parent #{{ props.parentSession.id }}
        </button>
        <button
          class="button ghost"
          :disabled="!props.selectedSessionId || props.loading"
          @click="props.renameCurrentSession"
        >
          Rename Session
        </button>
        <button
          class="button ghost"
          :disabled="!props.selectedSessionId || props.loading"
          @click="props.forkCurrentSession"
        >
          Fork Current Session
        </button>
        <button
          class="button ghost"
          :disabled="!props.selectedSessionId || props.loading"
          @click="props.exportCurrentSession"
        >
          Export Session
        </button>
        <button
          class="button ghost"
          :disabled="
            !props.selectedSessionId ||
            props.continuing ||
            Boolean(props.sessionState?.active_execution) ||
            props.sessionState?.workflow_state !== 'blocked'
          "
          @click="props.continueCurrentSession"
        >
          {{ props.continuing ? 'Continuing…' : 'Continue Run' }}
        </button>
        <button
          class="button danger"
          :disabled="!props.selectedSessionId || props.continuing || !props.sessionState?.active_execution"
          @click="props.cancelCurrentSessionRun"
        >
          {{ props.continuing ? 'Cancelling…' : 'Cancel Run' }}
        </button>
        <button
          class="button danger"
          :disabled="!props.selectedSessionId || props.loading"
          @click="props.deleteCurrentSession"
        >
          Delete Session
        </button>
      </div>
      <template v-if="props.ancestorSessions.length">
        <div class="muted">ancestors={{ props.ancestorSessions.map((session) => `#${session.id}`).join(' → ') }}</div>
      </template>
      <template v-if="props.executionFacts.length">
        <div class="muted mono">{{ props.executionFacts.join(' · ') }}</div>
      </template>
      <div v-if="props.contextUsageLabel" class="muted mono">{{ props.contextUsageLabel }}</div>
      <template v-if="props.sessionState?.execution">
        <div v-if="props.sessionState.execution.agent_system_prompt" class="muted mono">
          agent_system_prompt={{ props.sessionState.execution.agent_system_prompt }}
        </div>
      </template>
      <template v-if="props.sessionState?.automation">
        <div class="muted">automation_jobs={{ props.sessionState.automation.job_count }}</div>
        <div v-if="props.sessionState.automation.latest_job?.last_run" class="muted">
          automation_status={{ props.sessionState.automation.latest_job.last_run.status }} · triggered
          {{ props.formatMessageTime(props.sessionState.automation.latest_job.last_run.triggered_at) }}
        </div>
        <div v-else-if="props.sessionState.automation.latest_job?.next_fire_at" class="muted">
          next_automation={{ props.formatMessageTime(props.sessionState.automation.latest_job.next_fire_at) }}
        </div>
        <div v-if="props.sessionState.automation.latest_job?.last_run?.error_message" class="muted">
          automation_error={{ props.sessionState.automation.latest_job.last_run.error_message }}
        </div>
      </template>
      <template v-if="props.sessionUsageSummaryFacts.length">
        <div class="muted mono">session_usage={{ props.sessionUsageSummaryFacts.join(' · ') }}</div>
      </template>
      <template v-if="props.siblingSessions.length">
        <div class="muted" style="margin-top: 8px">siblings={{ props.siblingSessions.length }}</div>
        <div class="button-row" style="margin-top: 6px">
          <button
            v-for="sibling in props.siblingSessions"
            :key="`sibling-${sibling.id}`"
            class="button ghost"
            @click="props.selectSession(sibling.id)"
          >
            #{{ sibling.id }} {{ sibling.title }}
          </button>
        </div>
      </template>
      <template v-if="props.childSessions.length">
        <div class="muted" style="margin-top: 8px">child_sessions={{ props.childSessions.length }}</div>
        <div class="button-row" style="margin-top: 6px">
          <button
            v-for="child in props.childSessions"
            :key="`child-${child.id}`"
            class="button ghost"
            @click="props.selectSession(child.id)"
          >
            #{{ child.id }} {{ child.title }}
          </button>
        </div>
      </template>
    </div>
    <p v-else class="muted">Send a prompt to create a new session, or pick an existing session.</p>
  </section>
</template>
