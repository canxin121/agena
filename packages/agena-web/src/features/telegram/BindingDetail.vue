<script setup lang="ts">
import { computed, ref } from 'vue'
import Button from '@/components/ui/Button.vue'
import ChannelAccessPanel from '@/features/integrations/ChannelAccessPanel.vue'
import type { IntegrationBinding, IntegrationControlAction, IntegrationDetail } from '@/generated/agenaState'
import { useTelegramCopy } from './copy'
const props = defineProps<{ detail: IntegrationDetail; source: number; busy: boolean; botEnabled: boolean }>()
const emit = defineEmits<{
  control: [binding: IntegrationBinding, action: IntegrationControlAction]
  changed: [binding: IntegrationBinding]
  editing: [value: boolean]
  openSession: [id: number]
}>()
const { tx, stateLabel, date } = useTelegramCopy()
const tab = ref<'events' | 'deliveries' | 'access'>('events')
const search = ref('')
const events = computed(() =>
  props.detail.events.filter(
    (entry) =>
      !search.value ||
      `${entry.event.text} ${entry.event.actor} ${entry.state} ${entry.note || ''}`
        .toLowerCase()
        .includes(search.value.toLowerCase()),
  ),
)
const deliveries = computed(() =>
  props.detail.deliveries.filter(
    (entry) =>
      !search.value ||
      `${entry.text} ${entry.state} ${entry.error || ''}`.toLowerCase().includes(search.value.toLowerCase()),
  ),
)
</script>
<template>
  <section
    class="min-w-0 space-y-4 rounded-2xl border border-border/70 bg-card p-4 sm:p-5"
    data-testid="telegram-binding-detail"
  >
    <div class="flex flex-wrap justify-between gap-3">
      <div class="min-w-0">
        <h3 class="break-words font-semibold">{{ detail.binding.spec.title }}</h3>
        <p class="mt-1 break-all font-mono text-xs text-muted-foreground">
          {{ detail.binding.spec.resource
          }}<span v-if="detail.binding.spec.thread"> / {{ detail.binding.spec.thread }}</span>
        </p>
      </div>
      <span
        class="h-fit rounded-md bg-muted px-2 py-1 text-xs"
        :class="
          detail.binding.state === 'enabled'
            ? 'text-emerald-600 dark:text-emerald-400'
            : detail.binding.state === 'faulted'
              ? 'text-destructive'
              : 'text-muted-foreground'
        "
        >{{ stateLabel(detail.binding.state) }}</span
      >
    </div>
    <div class="grid grid-cols-2 gap-2 text-xs sm:grid-cols-3">
      <div class="rounded-lg bg-muted/40 p-3">
        <span class="block text-muted-foreground">{{ tx('daily') }}</span
        ><strong class="mt-1 block">{{ detail.binding.day_admitted }} / {{ detail.binding.spec.daily_limit }}</strong>
      </div>
      <div class="rounded-lg bg-muted/40 p-3">
        <span class="block text-muted-foreground">{{ tx('total') }}</span
        ><strong class="mt-1 block">{{ detail.binding.admitted }} / {{ detail.binding.spec.total_limit }}</strong>
      </div>
      <div class="col-span-2 rounded-lg bg-muted/40 p-3 sm:col-span-1">
        <span class="block text-muted-foreground">{{ tx('expiry') }}</span
        ><span class="mt-1 block">{{ date(detail.binding.spec.expires_at_ms) }}</span>
      </div>
    </div>
    <div class="flex flex-wrap gap-2">
      <Button
        size="sm"
        :disabled="busy || !botEnabled || detail.binding.state === 'enabled'"
        data-testid="telegram-enable"
        @click="emit('control', detail.binding, 'enable')"
        >{{ tx('enableBinding') }}</Button
      ><Button
        variant="outline"
        size="sm"
        :disabled="busy || detail.binding.state !== 'enabled'"
        data-testid="telegram-pause"
        @click="emit('control', detail.binding, 'pause')"
        >{{ tx('pauseBinding') }}</Button
      ><Button
        variant="ghost"
        size="sm"
        :disabled="busy || !botEnabled || detail.binding.state !== 'enabled'"
        data-testid="telegram-poll"
        @click="emit('control', detail.binding, 'poll_now')"
        >{{ tx('pollNow') }}</Button
      >
    </div>
    <p
      v-if="detail.binding.last_error"
      role="alert"
      class="break-words rounded-lg bg-destructive/10 p-3 text-sm text-destructive"
    >
      {{ detail.binding.last_error }}
    </p>
    <nav class="flex flex-wrap gap-1 border-b border-border pb-2" :aria-label="tx('activity')">
      <Button
        v-for="item in ['events', 'deliveries', 'access'] as const"
        :key="item"
        :variant="tab === item ? 'secondary' : 'ghost'"
        size="sm"
        :aria-pressed="tab === item"
        @click="tab = item"
        >{{ tx(item === 'events' ? 'received' : item === 'deliveries' ? 'deliveries' : 'permissions') }}
        <span v-if="item !== 'access'" class="ml-1 text-xs opacity-60">{{
          item === 'events' ? detail.events.length : detail.deliveries.length
        }}</span></Button
      >
    </nav>
    <ChannelAccessPanel
      v-if="tab === 'access'"
      :source="source"
      :detail="detail"
      :readonly="busy"
      @editing="emit('editing', $event)"
      @changed="emit('changed', $event)"
    />
    <template v-else>
      <input
        v-model="search"
        type="search"
        :aria-label="tx('messageFilter')"
        :placeholder="tx('messageFilter')"
        class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm focus-visible:ring-2 focus-visible:ring-ring"
      />
      <div v-if="tab === 'events'" class="max-h-[32rem] space-y-3 overflow-y-auto" data-testid="telegram-event-list">
        <p v-if="!events.length" class="py-6 text-center text-sm text-muted-foreground">{{ tx('none') }}</p>
        <article v-for="entry in events" :key="entry.event.id" class="space-y-2 rounded-xl bg-muted/40 p-3">
          <div class="flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
            <span class="font-mono">{{ entry.event.actor }}</span
            ><span>{{ entry.state }} · {{ date(entry.received_at_ms) }}</span>
          </div>
          <p class="whitespace-pre-wrap break-words text-sm">{{ entry.event.text }}</p>
          <p v-if="entry.note" class="break-words text-xs text-muted-foreground">{{ entry.note }}</p>
          <Button v-if="entry.session_id" variant="ghost" size="sm" @click="emit('openSession', entry.session_id)">{{
            tx('openConversation')
          }}</Button>
        </article>
      </div>
      <div v-else class="max-h-[32rem] space-y-3 overflow-y-auto">
        <p v-if="!deliveries.length" class="py-6 text-center text-sm text-muted-foreground">{{ tx('none') }}</p>
        <article v-for="entry in deliveries" :key="entry.id" class="space-y-2 rounded-xl bg-muted/40 p-3">
          <div class="flex flex-wrap justify-between gap-2 text-[11px] text-muted-foreground">
            <span>{{ entry.state }} · {{ entry.attempts }}</span
            ><span>{{ date(entry.created_at_ms) }}</span>
          </div>
          <p class="whitespace-pre-wrap break-words text-sm">{{ entry.text }}</p>
          <p v-if="entry.error" class="break-words text-xs text-destructive">{{ entry.error }}</p>
          <p v-if="entry.remote_id" class="break-all font-mono text-[10px] text-muted-foreground">
            {{ entry.remote_id }}
          </p>
        </article>
      </div>
    </template>
  </section>
</template>
