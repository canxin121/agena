<script setup>
import GitPage from '../../src/pages/GitPage.vue'
import { useDirectoryStore } from '../../src/stores/directory'
import { useUiStore } from '../../src/stores/ui'
const directory = new URLSearchParams(location.search).get('directory') || '/workspace'
useDirectoryStore().setDirectory(directory)
const ui = useUiStore()
ui.openWorkspaceWindow('git', { query: { directory } })
</script>
<template>
  <main class="h-screen bg-background text-foreground">
    <div class="h-full w-[345px] max-w-full border-r border-border" data-git-fixture><GitPage embedded /></div>
  </main>
</template>
