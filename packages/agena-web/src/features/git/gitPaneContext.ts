import type { InjectionKey, Ref } from 'vue'

export const gitPaneContextKey: InjectionKey<{ projectRoot: Ref<string | null>; repoRoot: Ref<string | null> }> =
  Symbol('git-pane')
