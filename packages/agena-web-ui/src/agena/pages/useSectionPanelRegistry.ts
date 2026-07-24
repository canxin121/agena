import { computed, type Ref } from 'vue'

export type SectionPanelRegistry<TTab extends string, TPanel> = Record<TTab, TPanel>

export function useSectionPanelRegistry<TTab extends string, TPanel>(input: {
  activeTab: Ref<TTab>
  panels: SectionPanelRegistry<TTab, TPanel>
}) {
  const currentPanel = computed(() => input.panels[input.activeTab.value])

  return {
    currentPanel,
    panels: input.panels,
  }
}
