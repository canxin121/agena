<script setup lang="ts">
import type { PermissionRequest } from '@/agena/lib/agenaApi'
import type { PermissionActionView } from '@/agena/lib/permissionFormatting'

const props = defineProps<{
  requests: PermissionRequest[]
  permissionActionView: (action: Record<string, unknown>) => PermissionActionView
  permissionRiskLabel: (risk?: string | null, action?: Record<string, unknown>) => string
  permissionExplainability: (value: {
    source?: string | null
    scope?: 'session' | 'workspace' | 'global' | null
    operator?: string | null
  }) => {
    summary: string
    details: string[]
  }
  permissionReplyPreview: (scope?: 'session' | 'workspace' | 'global') => string
  isInteractiveRequestBusy: (requestId: string) => boolean
  approvePermission: (
    requestId: string,
    kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always',
    scope?: 'session' | 'workspace' | 'global',
  ) => void | Promise<void>
}>()
</script>

<template>
  <section v-if="props.requests.length" class="card">
    <h3>Pending Permissions</h3>
    <div class="list">
      <div v-for="request in props.requests" :key="request.request_id" class="list-item">
        <div>
          <strong>{{ props.permissionActionView(request.action).title }}</strong>
        </div>
        <div class="muted mono">request_id={{ request.request_id }}</div>
        <div class="muted">{{ request.reason }}</div>
        <div class="muted">risk={{ props.permissionRiskLabel(request.risk, request.action) }}</div>
        <div v-if="request.explanation" class="muted">{{ request.explanation }}</div>
        <div v-if="request.trace?.length" class="muted mono">
          {{ request.trace.map((step) => step.summary).join(' · ') }}
        </div>
        <div v-if="request.related_actions?.length" class="muted mono">
          invocation={{ request.related_actions.map((action) => props.permissionActionView(action).title).join(' · ') }}
        </div>
        <div
          v-if="
            props.permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator })
              .summary
          "
          class="muted"
        >
          {{
            props.permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator })
              .summary
          }}
        </div>
        <div
          v-if="
            props.permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator })
              .details.length
          "
          class="muted mono"
        >
          {{
            props
              .permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator })
              .details.join(' · ')
          }}
        </div>
        <div class="muted mono">{{ props.permissionActionView(request.action).details.join(' · ') }}</div>
        <div class="button-row">
          <button
            class="button primary"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.approvePermission(request.request_id, 'allow_once')"
          >
            Allow Once
          </button>
          <button
            class="button"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.approvePermission(request.request_id, 'allow_always', 'session')"
          >
            Allow Always (Session)
          </button>
          <button
            class="button"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.approvePermission(request.request_id, 'allow_always', 'workspace')"
          >
            Allow Always (Workspace)
          </button>
          <button
            class="button"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.approvePermission(request.request_id, 'allow_always', 'global')"
          >
            Allow Always (Global)
          </button>
          <button
            class="button danger"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.approvePermission(request.request_id, 'deny_once')"
          >
            Deny Once
          </button>
          <button
            class="button danger"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.approvePermission(request.request_id, 'deny_always', 'session')"
          >
            Deny Always (Session)
          </button>
          <button
            class="button danger"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.approvePermission(request.request_id, 'deny_always', 'workspace')"
          >
            Deny Always (Workspace)
          </button>
          <button
            class="button danger"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.approvePermission(request.request_id, 'deny_always', 'global')"
          >
            Deny Always (Global)
          </button>
        </div>
        <div class="muted">
          once={{ props.permissionReplyPreview() }} · session={{ props.permissionReplyPreview('session') }} ·
          workspace={{ props.permissionReplyPreview('workspace') }} · global={{
            props.permissionReplyPreview('global')
          }}
        </div>
      </div>
    </div>
  </section>
</template>
