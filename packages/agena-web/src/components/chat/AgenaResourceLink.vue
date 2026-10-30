<script setup lang="ts">
import { computed, ref, type Component } from 'vue'
import { RiExternalLinkLine, RiLinksLine } from '@remixicon/vue'
import { openResource } from '@/lib/authenticatedResources'
import { browserResourceUrl, isAuthenticatedResourceUrl } from '@/lib/resourceUrls'

const props = defineProps<{ href: string; label: string; description?: string; icon?: Component }>()
const destination = computed(() => browserResourceUrl(props.href))
const protectedResource = computed(() => isAuthenticatedResourceUrl(destination.value))
const openError = ref('')
const fullTitle = computed(() => [...new Set([props.label, props.description].filter(Boolean))].join(' — '))

async function open(event: Event) {
  if (!protectedResource.value) return
  event.preventDefault()
  openError.value = ''
  try {
    await openResource(destination.value, props.label)
  } catch (error) {
    openError.value = error instanceof Error ? error.message : String(error)
  }
}
</script>

<template>
  <component
    v-if="destination"
    :is="protectedResource ? 'button' : 'a'"
    :type="protectedResource ? 'button' : undefined"
    tabindex="0"
    :href="protectedResource ? undefined : destination"
    target="_blank"
    rel="noopener noreferrer"
    class="part-link interaction-width"
    :title="fullTitle"
    :aria-label="label"
    :aria-description="description || undefined"
    @click="open"
  >
    <span class="interaction-choice-marker" aria-hidden="true"><component :is="icon || RiLinksLine" /></span>
    <span class="interaction-choice-copy">
      <span class="interaction-choice-label">{{ label }}</span>
      <span v-if="description && description !== label" class="interaction-choice-description">{{ description }}</span>
    </span>
    <RiExternalLinkLine class="interaction-choice-arrow" aria-hidden="true" />
  </component>
  <div v-else class="min-w-0">
    <p class="break-words text-[13px] font-medium">{{ label }}</p>
    <p v-if="description && description !== label" class="mt-1 break-words text-xs text-muted-foreground">
      {{ description }}
    </p>
  </div>
  <p v-if="openError" class="text-xs text-destructive" role="alert">{{ openError }}</p>
</template>
