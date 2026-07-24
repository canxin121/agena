<script setup lang="ts">
import type { useSettingsMemoryState } from './useSettingsMemoryState'

const props = defineProps<{
  memory: ReturnType<typeof useSettingsMemoryState>
}>()
</script>

<template>
  <div class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Durable Context</p>
          <h3 class="settings-panel-title">Memory</h3>
          <p class="record-subtitle">Manage workspace memory records loaded by Agena across sessions.</p>
        </div>
        <div class="button-row">
          <span class="badge neutral">{{ props.memory.memories.value.length }}</span>
          <button class="button" :disabled="props.memory.loading.value" @click="props.memory.load()">Refresh</button>
          <button class="button primary" @click="props.memory.createNewMemory">New Memory</button>
        </div>
      </div>
      <div class="field">
        <label class="label" for="memory-search">Search memories</label>
        <input
          id="memory-search"
          v-model="props.memory.search.value"
          class="input"
          placeholder="name, type, or content"
        />
      </div>
    </section>

    <div class="memory-layout">
      <section class="settings-panel memory-list-panel">
        <div class="settings-panel-header">
          <h3 class="settings-panel-title">Records</h3>
          <span class="badge">{{ props.memory.filteredMemories.value.length }}</span>
        </div>
        <div v-if="props.memory.filteredMemories.value.length" class="record-list">
          <button
            v-for="item in props.memory.filteredMemories.value"
            :key="item.name"
            class="record-card memory-record-button"
            :class="{ active: props.memory.selectedName.value === item.name }"
            @click="props.memory.selectMemory(item.name)"
          >
            <span>
              <strong>{{ item.name }}</strong>
              <span class="record-subtitle">{{ item.description || 'No description' }}</span>
            </span>
            <span class="badge neutral">{{ item.memory_type || 'untyped' }}</span>
          </button>
        </div>
        <div v-else class="empty-state">No memory records matched.</div>
      </section>

      <section class="settings-panel">
        <div class="settings-panel-header">
          <div>
            <p class="settings-panel-kicker">{{ props.memory.originalName.value ? 'Edit Record' : 'New Record' }}</p>
            <h3 class="settings-panel-title">{{ props.memory.originalName.value || 'Create memory' }}</h3>
          </div>
          <button
            v-if="props.memory.originalName.value"
            class="button danger"
            :disabled="props.memory.saving.value"
            @click="props.memory.remove"
          >
            Forget
          </button>
        </div>
        <div class="form-grid">
          <div class="field">
            <label class="label" for="memory-name">Name</label>
            <input
              id="memory-name"
              v-model="props.memory.draft.name"
              class="input mono"
              :disabled="Boolean(props.memory.originalName.value)"
              placeholder="project_decisions"
            />
          </div>
          <div class="field">
            <label class="label" for="memory-type">Type</label>
            <select id="memory-type" v-model="props.memory.draft.memoryType" class="select">
              <option value="">Untyped</option>
              <option value="user">User</option>
              <option value="feedback">Feedback</option>
              <option value="project">Project</option>
              <option value="reference">Reference</option>
              <option value="other">Other</option>
            </select>
          </div>
          <div class="field full">
            <label class="label" for="memory-description">Description</label>
            <input
              id="memory-description"
              v-model="props.memory.draft.description"
              class="input"
              placeholder="One-line retrieval hint"
            />
          </div>
          <div class="field full">
            <label class="label" for="memory-body">Body</label>
            <textarea
              id="memory-body"
              v-model="props.memory.draft.body"
              class="textarea mono memory-editor"
              placeholder="Durable context, why it matters, and how it should be applied."
            />
          </div>
        </div>
        <div class="button-row">
          <button
            class="button primary"
            :disabled="props.memory.saving.value || !props.memory.draft.name.trim() || !props.memory.draft.body.trim()"
            @click="props.memory.save"
          >
            {{ props.memory.saving.value ? 'Saving…' : 'Save Memory' }}
          </button>
        </div>
        <p v-if="props.memory.originalName.value" class="muted mono">
          {{ props.memory.memories.value.find((item) => item.name === props.memory.originalName.value)?.path }}
        </p>
      </section>
    </div>
  </div>
</template>
