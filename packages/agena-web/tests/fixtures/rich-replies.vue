<script setup>
// Real renderers with an explicit test workspace; backend operations use the
// isolated verification server supplied by the test runner's frontend proxy.
import { computed, ref } from 'vue'
import MarkdownRenderer from '../../src/components/markdown/MarkdownRenderer.vue'
import InteractiveAppPreview from '../../src/features/interactive/InteractiveAppPreview.vue'
import html from '../../../../examples/interactive-app/index.html?raw'
const query = new URLSearchParams(location.search)
const directory = query.get('directory') || '/workspace'
const owner = query.get('owner') || 'rich-reply-fixture'
const manifest = {
  id: 'flowboard',
  title: 'Flowboard',
  description: '真实后端 · Python + SQLite',
  height: 640,
  html,
  services: [
    {
      name: 'api',
      command: 'python3',
      args: ['server.py', '--port', '{port}', ...(query.has('database') ? ['--database', query.get('database')] : [])],
      cwd: 'examples/interactive-app',
      healthPath: '/health',
    },
  ],
}
const chart = ref({
  type: 'area',
  title: 'API requests · 本周',
  labels: ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'],
  series: [
    { name: 'Requests', values: [140, 190, null, 240, 180, 320, 290] },
    { name: 'Cached', values: [80, 100, 110, 130, 100, 210, 220] },
  ],
})
const suffix = ref(''),
  showPreview = ref(false)
const content = computed(
  () =>
    `## A little space to make progress\n\n这是回复中的交互应用，可以直接操作，并保存到真实数据库。\n\n\`\`\`agena-ui\n${JSON.stringify(manifest)}\n\`\`\`\n\n\`\`\`agena-chart\n${JSON.stringify(chart.value)}\n\`\`\`\n${suffix.value}`,
)
window.richReplyFixture = { append: (value) => (suffix.value += value), setType: (type) => (chart.value.type = type) }
</script>
<template>
  <main class="mx-auto max-w-4xl px-5 py-8 text-foreground">
    <button class="mb-4 text-xs" @click="showPreview = !showPreview">Toggle full preview</button
    ><InteractiveAppPreview v-if="showPreview" :owner="owner" app-id="flowboard" /><MarkdownRenderer
      v-else
      :content="content"
      allow-apps
      :app-owner="owner"
      :app-directory="directory"
      stream
    />
  </main>
</template>
