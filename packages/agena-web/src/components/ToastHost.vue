<script setup lang="ts">
import { useToastsStore } from '../stores/toasts'
import { computed, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import Sonner from './ui/Sonner.vue'

const store = useToastsStore()
const route = useRoute()
// The permanent Bot composer must remain usable while the first-visit
// notification offer is open. Keep transient notices below the top chrome.
const isBot = computed(() => route.path === '/bot')
const botDesktopToastOffset = {
  right: 'calc(var(--oc-safe-area-right, 0px) + 12px)',
  top: 'calc(var(--oc-safe-area-top, 0px) + 84px)',
}
const botMobileToastOffset = {
  left: 'calc(var(--oc-safe-area-left, 0px) + 10px)',
  right: 'calc(var(--oc-safe-area-right, 0px) + 10px)',
  top: 'calc(var(--oc-safe-area-top, 0px) + var(--oc-header-height, 48px) + 48px)',
}

const desktopToastOffset = {
  right: 'calc(var(--oc-safe-area-right, 0px) + 12px)',
  bottom: 'calc(var(--oc-safe-area-bottom, 0px) + 12px)',
}

const mobileToastOffset = {
  left: 'calc(var(--oc-safe-area-left, 0px) + 10px)',
  right: 'calc(var(--oc-safe-area-right, 0px) + 10px)',
  bottom:
    'calc(var(--oc-safe-area-bottom, 0px) + var(--oc-bottom-nav-height, 56px) - clamp(0px, var(--oc-keyboard-inset, 0px), var(--oc-bottom-nav-height, 56px)) + 10px)',
}

onMounted(() => {
  store.markHostReady()
})
</script>

<template>
  <Teleport to="body">
    <Sonner
      class="oc-toaster"
      :position="isBot ? 'top-right' : 'bottom-right'"
      :offset="isBot ? botDesktopToastOffset : desktopToastOffset"
      :mobile-offset="isBot ? botMobileToastOffset : mobileToastOffset"
    />
  </Teleport>
</template>
