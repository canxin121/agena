import { hasInjectionContext, inject, type ComputedRef, type InjectionKey } from 'vue'
import type { RouteLocationNormalizedLoaded, RouteLocationRaw } from 'vue-router'

export type WorkspacePaneContext = {
  windowId: ComputedRef<string>
  isFocused: ComputedRef<boolean>
  route: ComputedRef<RouteLocationNormalizedLoaded>
  navigate: (to: RouteLocationRaw, replace?: boolean) => Promise<unknown>
}

export const workspacePaneContextKey: InjectionKey<WorkspacePaneContext> = Symbol('agena-workspace-pane')

export function useWorkspacePaneContext(): WorkspacePaneContext | null {
  if (!hasInjectionContext()) return null
  return inject(workspacePaneContextKey, null)
}
