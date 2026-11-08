<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiTaskLine, RiAddLine, RiRefreshLine, RiArrowRightLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import Dialog from '@/components/ui/Dialog.vue'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { taskApi, type Task, type TaskApi, type TaskState } from '@/lib/taskApi'
import {
  TaskBoardController,
  newTaskBoardState,
  newTaskForm,
  canCancelTask,
  canResumeTask,
  needsAttention,
  validGoal,
  TASK_STATES,
} from './taskBoard'
import { loadTaskDraft, saveTaskDraft } from './taskDraft'

const props = defineProps<{
  sourceSessionId: number | null
  selectedTaskId?: string
  compose?: boolean
  api?: TaskApi
}>()
const emit = defineEmits<{ openSession: [id: number]; selected: [id: string]; leave: [] }>()
const { t, locale } = useI18n()
const state = reactive(newTaskBoardState(props.sourceSessionId))
const controller = new TaskBoardController(state, props.api || taskApi)
const visible = usePaneVisibility()
const inputId = useId()
const showDetail = ref(false)
const restored = ref(false)
const originalSource = props.sourceSessionId
const restoredForm = loadTaskDraft(originalSource)
if (restoredForm) {
  state.form = restoredForm
  restored.value = true
} else if (props.compose) state.form = newTaskForm('create', originalSource, null)

watch(
  () => state.form,
  (form) => saveTaskDraft(originalSource, form),
  { deep: true },
)

const selected = computed(() => state.selected)
const formTitle = computed(() => t(`tasks.${state.form?.kind || 'create'}`))
const valid = computed(() => {
  const form = state.form
  if (!form || state.busy || form.conflict) return false
  if (form.kind === 'create') return Boolean(form.source) && validGoal(form.text)
  if (form.kind === 'resume') return Boolean(form.target && canResumeTask(form.target)) && validGoal(form.text)
  if (form.kind === 'cancel') return Boolean(form.target && canCancelTask(form.target))
  return form.target?.state === 'review'
})

function date(ms: number): string {
  return new Date(ms).toLocaleString(String(locale.value))
}
function openForm(kind: 'create' | 'resume' | 'cancel' | 'accept') {
  if (state.busy) return
  state.actionError = ''
  restored.value = false
  state.form = newTaskForm(kind, originalSource, kind === 'create' ? null : selected.value)
}
function closeForm() {
  if (state.busy) return
  state.form = null
  state.actionError = ''
  restored.value = false
}
async function submit() {
  const task = await controller.submit()
  if (task) {
    showDetail.value = true
    emit('selected', task.id)
  }
}
async function select(task: Task) {
  state.actionError = ''
  showDetail.value = true
  emit('selected', task.id)
  await controller.select(task.id)
}
async function refresh() {
  await controller.refresh()
  if (state.selected) await controller.select(state.selected.id)
}
async function changeScope(value: string) {
  if (state.busy || state.form) return
  controller.setScope(value === 'all' ? null : originalSource)
  showDetail.value = false
  await controller.refresh()
}
async function changeState(value: string) {
  if (state.busy || state.form) return
  controller.setScope(state.source, value as TaskState | '')
  showDetail.value = false
  await controller.refresh()
}
async function page(delta: number) {
  if (state.loading || state.busy) return
  state.page = Math.min(500, Math.max(0, state.page + delta))
  await controller.refresh()
}
watch(
  visible,
  async (value) => {
    if (!value) return
    await controller.refresh()
    if (props.selectedTaskId) {
      await controller.select(props.selectedTaskId)
      showDetail.value = Boolean(state.selected)
    }
  },
  { immediate: true },
)
watch(
  () => props.selectedTaskId,
  async (id) => {
    if (id) {
      await controller.select(id)
      showDetail.value = Boolean(state.selected)
    }
  },
)
// Only the mounted, visible task pane polls. Reads are bounded; writes are
// never retried by this timer. Selected off-page tasks are refreshed as well.
let failures = 0,
  nextRead = 0
const timer = setInterval(async () => {
  if (!visible.value || state.loading || state.busy || Date.now() < nextRead) return
  await refresh()
  failures = state.error ? Math.min(failures + 1, 4) : 0
  nextRead = Date.now() + 2000 * 2 ** failures
}, 2000)
onBeforeUnmount(() => {
  clearInterval(timer)
  controller.dispose()
})
</script>

<template>
  <section
    class="task-board h-full min-h-0 flex flex-col bg-background"
    data-testid="task-board"
    :aria-label="t('tasks.title')"
  >
    <header class="shrink-0 border-b border-border p-4 sm:p-6">
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div class="min-w-0">
          <h1 class="flex items-center gap-2 text-lg font-semibold">
            <RiTaskLine class="h-5 w-5" />{{ t('tasks.title') }}
          </h1>
          <p class="mt-1 max-w-2xl text-sm text-muted-foreground">{{ t('tasks.description') }}</p>
        </div>
        <div class="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            :disabled="state.loading"
            @click="refresh"
            :aria-label="t('tasks.refresh')"
            ><RiRefreshLine class="h-4 w-4" /><span>{{ t('tasks.refresh') }}</span></Button
          >
          <Button size="sm" :disabled="!originalSource || state.busy" @click="openForm('create')" data-testid="task-new"
            ><RiAddLine class="h-4 w-4" />{{ t('tasks.create') }}</Button
          >
        </div>
      </div>
      <p v-if="!originalSource" class="mt-3 text-sm text-muted-foreground">
        {{ t('tasks.noSession') }} <button class="underline" @click="emit('leave')">{{ t('tasks.chat') }}</button>
      </p>
      <div class="mt-4 flex flex-wrap items-center gap-3">
        <label class="flex items-center gap-2 text-xs"
          >{{ t('tasks.scope') }}
          <select
            class="rounded-md border border-border bg-background px-2 py-2 text-sm"
            :value="state.source ? 'session' : 'all'"
            :disabled="state.busy || !!state.form"
            @change="changeScope(($event.target as HTMLSelectElement).value)"
          >
            <option v-if="originalSource" value="session">{{ t('tasks.currentSession') }} #{{ originalSource }}</option>
            <option value="all">{{ t('tasks.allTasks') }}</option>
          </select>
        </label>
        <label class="flex items-center gap-2 text-xs"
          >{{ t('tasks.state') }}
          <select
            class="rounded-md border border-border bg-background px-2 py-2 text-sm"
            :value="state.filter"
            :disabled="state.busy || !!state.form"
            @change="changeState(($event.target as HTMLSelectElement).value)"
          >
            <option value="">{{ t('tasks.allStates') }}</option>
            <option v-for="s in TASK_STATES" :key="s" :value="s">{{ t(`tasks.states.${s}`) }}</option>
          </select>
        </label>
        <span class="text-xs text-muted-foreground">{{ t('tasks.serial') }}</span>
      </div>
    </header>
    <div
      v-if="state.error"
      role="alert"
      class="m-3 rounded-md border border-destructive/40 p-3 text-sm text-destructive"
    >
      {{ state.error }} <button class="underline" @click="refresh">{{ t('tasks.retry') }}</button>
    </div>
    <div class="task-columns min-h-0 flex-1 grid" :data-detail="showDetail">
      <section class="task-list flex min-h-0 flex-col border-e border-border" :aria-label="t('tasks.list')">
        <div v-if="state.loading && !state.items.length" role="status" class="p-6 text-sm text-muted-foreground">
          {{ t('tasks.loading') }}
        </div>
        <div v-else-if="!state.items.length" class="m-auto p-6 text-center text-muted-foreground">
          <RiTaskLine class="mx-auto mb-3 h-8 w-8" />
          <p>{{ t('tasks.empty') }}</p>
        </div>
        <ul class="min-h-0 flex-1 overflow-y-auto p-2" :aria-busy="state.loading">
          <li v-for="task in state.items" :key="task.id" class="mb-1">
            <button
              class="task-row w-full rounded-lg border p-3 text-start hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              :class="selected?.id === task.id ? 'border-primary bg-primary/5' : 'border-transparent'"
              :aria-pressed="selected?.id === task.id"
              @click="select(task)"
              :data-task-id="task.id"
            >
              <div class="flex items-center justify-between gap-2">
                <span class="task-status text-[11px]" :data-attention="needsAttention(task)" :data-state="task.state">{{
                  t(`tasks.states.${task.state}`)
                }}</span
                ><span class="text-[10px] text-muted-foreground">{{ t('tasks.attempt') }} {{ task.attempt }}</span>
              </div>
              <p class="task-goal mt-2 text-sm font-medium">{{ task.goal }}</p>
              <p class="mt-2 text-[10px] text-muted-foreground">{{ date(task.updated_at_ms) }}</p>
            </button>
          </li>
        </ul>
        <footer class="flex shrink-0 items-center justify-between border-t border-border p-3 text-xs">
          <Button variant="ghost" size="sm" :disabled="state.page === 0 || state.loading" @click="page(-1)">{{
            t('tasks.previous')
          }}</Button>
          <span>{{ t('tasks.page', { page: state.page + 1 }) }}</span>
          <Button
            variant="ghost"
            size="sm"
            :disabled="!state.hasMore || state.loading || state.page >= 500"
            @click="page(1)"
            >{{ t('tasks.next') }}</Button
          >
        </footer>
      </section>
      <section class="task-detail min-h-0 overflow-y-auto p-4 sm:p-6" :aria-label="t('tasks.details')" tabindex="-1">
        <button class="task-back mb-4 text-sm underline" @click="showDetail = false">{{ t('tasks.back') }}</button>
        <template v-if="selected">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <span class="task-status" :data-attention="needsAttention(selected)" :data-state="selected.state">{{
              t(`tasks.states.${selected.state}`)
            }}</span
            ><span class="font-mono text-[10px] text-muted-foreground break-all">{{ selected.id }}</span>
          </div>
          <h2 class="mt-4 whitespace-pre-wrap break-words text-base font-semibold">{{ selected.goal }}</h2>
          <dl class="mt-5 grid grid-cols-2 gap-4 text-xs">
            <div>
              <dt class="text-muted-foreground">{{ t('tasks.attempt') }}</dt>
              <dd class="mt-1">{{ selected.attempt }}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">{{ t('tasks.updated') }}</dt>
              <dd class="mt-1">{{ date(selected.updated_at_ms) }}</dd>
            </div>
          </dl>
          <p
            v-if="selected.note"
            class="mt-4 rounded-md border border-border bg-muted/30 p-3 text-sm whitespace-pre-wrap break-words"
          >
            {{ selected.note }}
          </p>
          <p v-if="selected.state === 'waiting'" class="mt-3 text-sm text-muted-foreground">
            {{ t('tasks.waitingHint') }}
          </p>
          <p v-if="selected.state === 'review'" class="mt-3 text-sm text-muted-foreground">
            {{ t('tasks.reviewHint') }}
          </p>
          <div class="mt-5 flex flex-wrap gap-2">
            <Button
              v-if="selected.worker_session_id"
              @click="emit('openSession', selected.worker_session_id)"
              data-testid="task-worker"
              >{{ t('tasks.worker') }}<RiArrowRightLine class="h-4 w-4"
            /></Button>
            <Button variant="outline" @click="emit('openSession', selected.source_session_id)">{{
              t('tasks.source')
            }}</Button>
          </div>
          <div class="my-5 border-t border-border" />
          <div class="flex flex-wrap gap-2">
            <Button
              v-if="selected.state === 'review'"
              :disabled="state.busy"
              @click="openForm('accept')"
              data-testid="task-accept"
              >{{ t('tasks.accept') }}</Button
            >
            <Button
              v-if="canResumeTask(selected)"
              variant="outline"
              :disabled="state.busy"
              @click="openForm('resume')"
              data-testid="task-resume"
              >{{ t('tasks.resume') }}</Button
            >
            <Button
              v-if="canCancelTask(selected)"
              variant="outline"
              :disabled="state.busy"
              @click="openForm('cancel')"
              data-testid="task-cancel"
              >{{ t('tasks.cancel') }}</Button
            >
          </div>
          <div v-if="selected.instruction !== selected.goal" class="mt-6">
            <h3 class="text-xs text-muted-foreground">{{ t('tasks.feedback') }}</h3>
            <p class="mt-2 whitespace-pre-wrap break-words text-sm">{{ selected.instruction }}</p>
          </div>
        </template>
        <div v-else class="grid h-full place-content-center text-sm text-muted-foreground">{{ t('tasks.select') }}</div>
      </section>
    </div>
    <Dialog
      :open="!!state.form"
      :title="formTitle"
      :description="t('tasks.formHint')"
      @close="closeForm"
      max-width="max-w-xl"
    >
      <form v-if="state.form" class="space-y-4 p-4" @submit.prevent="submit" :aria-busy="state.busy">
        <p v-if="restored" class="text-sm text-muted-foreground">{{ t('tasks.restored') }}</p>
        <p v-if="state.form.target" class="line-clamp-3 text-sm font-medium break-words">
          {{ state.form.target.goal }}
        </p>
        <template v-if="state.form.kind === 'create' || state.form.kind === 'resume'">
          <label :for="inputId" class="block text-sm font-medium">{{
            t(state.form.kind === 'create' ? 'tasks.goal' : 'tasks.feedback')
          }}</label>
          <textarea
            :id="inputId"
            v-model="state.form.text"
            class="min-h-32 w-full rounded-md border border-border bg-background p-3 text-sm outline-none focus:ring-2 focus:ring-ring"
            :readonly="state.busy || !!state.form.submitted"
            :placeholder="t('tasks.placeholder')"
            maxlength="32768"
            @keydown.ctrl.enter.prevent="valid && submit()"
            @keydown.meta.enter.prevent="valid && submit()"
          />
          <p v-if="state.form.text && !validGoal(state.form.text)" role="alert" class="text-sm text-destructive">
            {{ t('tasks.invalid') }}
          </p>
        </template>
        <p v-else class="text-sm">
          {{ t(state.form.kind === 'cancel' ? 'tasks.confirmCancel' : 'tasks.confirmAccept') }}
        </p>
        <p
          v-if="state.form.submitted && state.actionError && !state.form.conflict"
          class="text-sm text-muted-foreground"
        >
          {{ t('tasks.uncertain') }}
        </p>
        <div
          v-if="state.actionError"
          role="alert"
          class="rounded-md border border-destructive/40 p-3 text-sm text-destructive"
        >
          {{ state.actionError }}
        </div>
        <p v-if="state.form.conflict" class="text-sm text-muted-foreground">{{ t('tasks.conflict') }}</p>
        <div class="flex flex-wrap justify-end gap-2">
          <Button variant="ghost" type="button" :disabled="state.busy" @click="closeForm">{{
            t('tasks.close')
          }}</Button>
          <Button v-if="state.form.conflict" variant="outline" type="button" @click="controller.rebase()">{{
            t('tasks.rebase')
          }}</Button>
          <Button type="submit" :disabled="!valid" data-testid="task-submit">{{
            state.busy
              ? t('tasks.saving')
              : state.actionError && !state.form.conflict
                ? t('tasks.retry')
                : t('tasks.confirm')
          }}</Button>
        </div>
      </form>
    </Dialog>
  </section>
</template>

<style scoped>
.task-columns {
  grid-template-columns: minmax(240px, 34%) minmax(0, 1fr);
}
.task-goal {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  overflow-wrap: anywhere;
}
.task-status {
  display: inline-block;
  border-radius: 6px;
  padding: 3px 8px;
  background: color-mix(in srgb, currentColor 7%, transparent);
  font-size: 12px;
}
.task-status[data-attention='true'] {
  color: #a16207;
  background: #ca8a0414;
}
.task-status[data-state='completed'] {
  color: #15803d;
  background: #16a34a14;
}
.task-back {
  display: none;
}
@media (max-width: 640px) {
  .task-columns {
    grid-template-columns: minmax(0, 1fr);
  }
  .task-detail {
    display: none;
  }
  .task-columns[data-detail='true'] .task-list {
    display: none;
  }
  .task-columns[data-detail='true'] .task-detail {
    display: block;
  }
  .task-back {
    display: inline-block;
  }
}
</style>
