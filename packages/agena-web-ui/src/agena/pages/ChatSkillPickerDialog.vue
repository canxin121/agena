import { userErrorMessage } from '@/lib/api'
<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'

import { invokePluginUiTool } from '@/agena/lib/agenaApi'
import {
  createComposerSkillDraft,
  parseSkillCatalogPage,
  SKILL_PICKER_PAGE_SIZE,
  type ComposerSkillDraft,
  type SkillCatalogItem,
} from './chatSkillModel'

const props = defineProps<{
  open: boolean
  sessionId: number | null
  selectedIds: string[]
}>()

const emit = defineEmits<{
  close: []
  select: [skill: ComposerSkillDraft]
}>()

const dialog = ref<HTMLElement | null>(null)
const items = ref<SkillCatalogItem[]>([])
const total = ref(0)
const offset = ref(0)
const loading = ref(false)
const selecting = ref('')
const error = ref('')
let requestGeneration = 0

const pageNumber = computed(() => Math.floor(offset.value / SKILL_PICKER_PAGE_SIZE) + 1)
const pageCount = computed(() => Math.max(1, Math.ceil(total.value / SKILL_PICKER_PAGE_SIZE)))
const hasPrevious = computed(() => offset.value > 0)
const hasNext = computed(() => offset.value + items.value.length < total.value)

async function loadPage(nextOffset = offset.value) {
  const sessionId = props.sessionId
  if (!sessionId) {
    items.value = []
    total.value = 0
    error.value = 'Select or create a session before attaching a Skill.'
    return
  }
  const generation = ++requestGeneration
  loading.value = true
  error.value = ''
  try {
    const response = await invokePluginUiTool({
      tool: 'agena.skills.list',
      sessionId,
      payload: {
        kind: 'skill',
        offset: Math.max(0, nextOffset),
        limit: SKILL_PICKER_PAGE_SIZE,
        verbose: true,
      },
    })
    if (generation !== requestGeneration) return
    const page = parseSkillCatalogPage(response)
    items.value = page.items
    total.value = page.total
    offset.value = page.offset
  } catch (reason) {
    if (generation !== requestGeneration) return
    items.value = []
    error.value = userErrorMessage(reason)
  } finally {
    if (generation === requestGeneration) loading.value = false
  }
}

async function refreshCatalog() {
  if (!props.sessionId) return
  loading.value = true
  error.value = ''
  try {
    await invokePluginUiTool({
      tool: 'agena.skills.refresh',
      sessionId: props.sessionId,
      payload: { verbose: false },
    })
    await loadPage(0)
  } catch (reason) {
    error.value = userErrorMessage(reason)
    loading.value = false
  }
}

async function selectSkill(item: SkillCatalogItem) {
  if (!props.sessionId || selecting.value) return
  selecting.value = item.name
  error.value = ''
  try {
    const response = await invokePluginUiTool({
      tool: 'agena.skills.get',
      sessionId: props.sessionId,
      payload: { name: item.name },
    })
    emit('select', createComposerSkillDraft(response))
    emit('close')
  } catch (reason) {
    error.value = userErrorMessage(reason)
  } finally {
    selecting.value = ''
  }
}

watch(
  () => props.open,
  (open) => {
    if (!open) {
      requestGeneration += 1
      return
    }
    offset.value = 0
    void loadPage(0)
    void nextTick(() => dialog.value?.focus())
  },
  { immediate: true },
)
</script>

<template>
  <Teleport to="body">
    <div v-if="props.open" class="skill-picker-backdrop" @click.self="emit('close')">
      <section
        ref="dialog"
        class="skill-picker-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="skill-picker-title"
        tabindex="-1"
        @keydown.esc="emit('close')"
      >
        <header class="skill-picker-header">
          <div>
            <h2 id="skill-picker-title">Attach a Skill</h2>
            <p>
              Select a Skill to attach its exact instructions to the next message. This is message-scoped guidance; it
              does not grant permissions or change the model.
            </p>
          </div>
          <button class="button ghost" :disabled="loading || Boolean(selecting)" @click="emit('close')">Close</button>
        </header>

        <div class="skill-picker-toolbar">
          <span class="badge neutral">{{ total }} Skill{{ total === 1 ? '' : 's' }}</span>
          <span class="muted mono">Page {{ pageNumber }} / {{ pageCount }}</span>
          <button class="button ghost" :disabled="loading || !props.sessionId" @click="refreshCatalog">Refresh</button>
        </div>

        <div v-if="error" class="skill-picker-error">{{ error }}</div>
        <div v-if="loading && !items.length" class="skill-picker-empty muted">Loading Skills…</div>
        <div v-else-if="!items.length && !error" class="skill-picker-empty muted">No Skills were discovered.</div>
        <div v-else class="skill-picker-grid">
          <button
            v-for="item in items"
            :key="`${item.name}:${item.contentHash}`"
            class="skill-picker-item"
            :class="{ selected: props.selectedIds.includes(`${item.name}:${item.contentHash}`) }"
            :disabled="Boolean(selecting)"
            @click="selectSkill(item)"
          >
            <div class="skill-picker-item-title">
              <strong>{{ item.name }}</strong>
              <span v-if="props.selectedIds.includes(`${item.name}:${item.contentHash}`)" class="badge">Attached</span>
            </div>
            <p>{{ item.summary || 'No description provided.' }}</p>
            <div v-if="item.aliases.length" class="muted mono">aliases={{ item.aliases.join(', ') }}</div>
            <div class="skill-picker-item-meta">
              <span>{{ item.source }}</span>
              <span v-if="selecting === item.name">Reading instructions…</span>
            </div>
          </button>
        </div>

        <footer class="skill-picker-footer">
          <button
            class="button"
            :disabled="loading || Boolean(selecting) || !hasPrevious"
            @click="loadPage(Math.max(0, offset - SKILL_PICKER_PAGE_SIZE))"
          >
            Previous
          </button>
          <span class="muted mono"
            >{{ offset + (items.length ? 1 : 0) }}–{{ offset + items.length }} of {{ total }}</span
          >
          <button
            class="button"
            :disabled="loading || Boolean(selecting) || !hasNext"
            @click="loadPage(offset + SKILL_PICKER_PAGE_SIZE)"
          >
            Next
          </button>
        </footer>
      </section>
    </div>
  </Teleport>
</template>

<style scoped>
.skill-picker-backdrop {
  position: fixed;
  inset: 0;
  z-index: 80;
  display: grid;
  place-items: center;
  padding: 24px;
  background: color-mix(in srgb, #071018 74%, transparent);
  backdrop-filter: blur(8px);
}

.skill-picker-dialog {
  width: min(920px, 100%);
  max-height: min(760px, calc(100vh - 48px));
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: 18px;
  background: var(--surface);
  box-shadow: 0 28px 90px rgb(0 0 0 / 45%);
  outline: none;
}

.skill-picker-header,
.skill-picker-toolbar,
.skill-picker-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 18px 20px;
}

.skill-picker-header {
  align-items: flex-start;
  border-bottom: 1px solid var(--border);
}

.skill-picker-header h2,
.skill-picker-header p,
.skill-picker-item p {
  margin: 0;
}

.skill-picker-header p {
  max-width: 680px;
  margin-top: 6px;
  color: var(--muted);
}

.skill-picker-toolbar,
.skill-picker-footer {
  border-bottom: 1px solid var(--border);
}

.skill-picker-footer {
  border-top: 1px solid var(--border);
  border-bottom: 0;
}

.skill-picker-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
  padding: 20px;
}

.skill-picker-item {
  display: grid;
  gap: 9px;
  min-height: 142px;
  padding: 16px;
  color: inherit;
  text-align: left;
  border: 1px solid var(--border);
  border-radius: 14px;
  background: var(--surface-subtle);
  cursor: pointer;
}

.skill-picker-item:hover,
.skill-picker-item:focus-visible,
.skill-picker-item.selected {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 22%, transparent);
}

.skill-picker-item:disabled {
  cursor: wait;
  opacity: 0.68;
}

.skill-picker-item-title,
.skill-picker-item-meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.skill-picker-item-meta {
  align-self: end;
  flex-wrap: wrap;
  color: var(--ink-dim);
  font-size: 0.78rem;
}

.skill-picker-error,
.skill-picker-empty {
  margin: 20px;
  padding: 16px;
  border-radius: 12px;
}

.skill-picker-error {
  color: var(--danger);
  border: 1px solid color-mix(in srgb, var(--danger) 45%, var(--border));
  background: color-mix(in srgb, var(--danger) 9%, transparent);
}

.skill-picker-empty {
  text-align: center;
  border: 1px dashed var(--border);
}

@media (max-width: 720px) {
  .skill-picker-backdrop {
    padding: 10px;
  }

  .skill-picker-grid {
    grid-template-columns: 1fr;
  }

  .skill-picker-header {
    flex-direction: column;
  }
}
</style>
