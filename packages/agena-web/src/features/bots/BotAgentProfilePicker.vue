<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { RiAddLine, RiRobot2Line } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import {
  agentProfileCatalog,
  agentProfileDisplayName,
  selectableAgentProfiles,
  withBuiltinAgentProfiles,
  type AgentProfileCatalog,
} from '@/lib/agentProfiles'
import { readRuntimeSettingSources } from '@/lib/runtimeSettings'

defineProps<{ disabled?: boolean }>()
const profile = defineModel<string>({ required: true })
const emit = defineEmits<{ create: [] }>()
const catalog = ref<AgentProfileCatalog>({})
const loading = ref(true)
const error = ref('')
const abort = new AbortController()
const options = computed(() =>
  Object.entries(selectableAgentProfiles(catalog.value, false))
    .map(([id, item]) => ({
      value: id,
      label: agentProfileDisplayName(id, item),
      description: item.description,
    }))
    .sort((a, b) => a.label.localeCompare(b.label)),
)

onMounted(async () => {
  try {
    const [profiles, builtins] = await Promise.all([
      readRuntimeSettingSources('profiles', abort.signal),
      readRuntimeSettingSources('builtin_profiles', abort.signal),
    ])
    if (!abort.signal.aborted)
      catalog.value = withBuiltinAgentProfiles(
        agentProfileCatalog(builtins.effective.value),
        agentProfileCatalog(profiles.global.value),
      )
  } catch (cause) {
    if (!abort.signal.aborted) error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    loading.value = false
  }
})
onBeforeUnmount(() => abort.abort())
</script>

<template>
  <section class="min-w-0 space-y-2" data-bot-agent-profile>
    <h3 class="text-sm font-medium">{{ $st('Agent profile') }}</h3>
    <div class="flex min-w-0 flex-wrap items-center gap-2">
      <OptionPicker
        v-model="profile"
        class="min-w-0 flex-1"
        trigger-class="w-full"
        :options="options"
        :title="$st('Agent profile')"
        :empty-label="`${$st('Default')} · main`"
        :search-placeholder="$st('Search profiles…')"
        :icon="RiRobot2Line"
        :disabled="disabled || loading || !!error"
        data-testid="bot-agent-profile"
      />
      <Button
        type="button"
        variant="outline"
        :disabled="disabled"
        data-testid="bot-create-agent-profile"
        @click="emit('create')"
      >
        <RiAddLine class="me-1.5 h-4 w-4" aria-hidden="true" />
        {{ $st('New profile') }}
      </Button>
    </div>
    <p v-if="error" role="alert" class="break-words text-xs text-destructive">{{ error }}</p>
  </section>
</template>
