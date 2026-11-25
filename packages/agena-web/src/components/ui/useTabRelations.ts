import { useId } from 'vue'

/** Give each tab and its panel stable, reciprocal ID references within this instance. */
export function useTabRelations() {
  const group = useId()
  const tabId = (key: string) => `${group}-tab-${key}`
  const panelId = (key: string) => `${group}-panel-${key}`

  return {
    tab: (key: string) => ({ id: tabId(key), 'aria-controls': panelId(key) }),
    panel: (key: string) => ({
      id: panelId(key),
      role: 'tabpanel' as const,
      'aria-labelledby': tabId(key),
      tabindex: 0,
    }),
  }
}
