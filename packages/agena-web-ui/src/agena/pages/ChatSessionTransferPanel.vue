<script setup lang="ts">
const props = defineProps<{
  loading: boolean
  sessionImportJsonl: string
  importSessionFromJsonl: () => void | Promise<void>
}>()

const emit = defineEmits<{
  'update:sessionImportJsonl': [value: string]
}>()
</script>

<template>
  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3 style="margin: 0">Session Transfer</h3>
        <p class="muted">
          Use existing export/import APIs to move sessions between runtimes or resume from saved JSONL.
        </p>
      </div>
      <button
        class="button ghost"
        :disabled="props.loading || !props.sessionImportJsonl.trim()"
        @click="props.importSessionFromJsonl"
      >
        Import Session
      </button>
    </div>
    <div class="field" style="margin-top: 12px">
      <label class="label" for="session-import-jsonl">Session JSONL</label>
      <textarea
        id="session-import-jsonl"
        :value="props.sessionImportJsonl"
        class="input mono"
        rows="8"
        placeholder="Paste exported session JSONL here"
        @input="emit('update:sessionImportJsonl', ($event.target as HTMLTextAreaElement | null)?.value || '')"
      />
    </div>
  </section>
</template>
