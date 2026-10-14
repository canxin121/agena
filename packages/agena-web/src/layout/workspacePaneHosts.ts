import { inject, provide, shallowReactive, type InjectionKey } from 'vue'

type PaneHosts = Map<string, HTMLElement>
const key: InjectionKey<PaneHosts> = Symbol('workspace-pane-hosts')

export function provideWorkspacePaneHosts() {
  const hosts = shallowReactive(new Map<string, HTMLElement>())
  provide(key, hosts)
  return hosts
}

export function useWorkspacePaneHosts() {
  return inject(key, null)
}
