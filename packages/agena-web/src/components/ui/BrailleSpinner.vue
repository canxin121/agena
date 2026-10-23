<script setup lang="ts">
const frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'].join('\n')
</script>

<template>
  <span class="braille-spinner" aria-hidden="true">
    <span class="braille-spinner__frames">{{ frames }}</span>
  </span>
</template>

<style scoped>
.braille-spinner {
  display: inline-block;
  width: 1em;
  height: 1em;
  overflow: hidden;
  line-height: 1;
  text-align: center;
  vertical-align: middle;
}

.braille-spinner__frames {
  display: block;
  white-space: pre;
  animation: braille-frames 800ms steps(10, end) infinite;
}

@keyframes braille-frames {
  to {
    transform: translateY(-100%);
  }
}

@media (prefers-reduced-motion: reduce) {
  .braille-spinner__frames {
    animation: none;
  }
}
</style>
