import { isRef } from 'vue'

type RectangleSource = { getBoundingClientRect: () => DOMRect | undefined }
type AnchorComponent = Partial<RectangleSource> & { triggerEl?: unknown; $el?: unknown }
export type OverlayAnchor = HTMLElement | AnchorComponent | null

function unwrap(value: unknown): unknown {
  for (let depth = 0; depth < 4 && isRef(value); depth++) value = value.value
  return value
}

function element(value: unknown): HTMLElement | null {
  return typeof HTMLElement !== 'undefined' && value instanceof HTMLElement ? value : null
}

export function resolveOverlayAnchorElement(value: unknown): HTMLElement | null {
  const anchor = unwrap(value)
  if (!anchor || typeof anchor !== 'object') return null
  const component = anchor as AnchorComponent
  const trigger = unwrap(component.triggerEl)
  return (
    element(anchor) ||
    element(trigger) ||
    element(unwrap((trigger as AnchorComponent | null | undefined)?.$el)) ||
    element(unwrap(component.$el))
  )
}

export function resolveOverlayAnchor(value: unknown): RectangleSource | null {
  const node = resolveOverlayAnchorElement(value)
  if (node) return node
  const anchor = unwrap(value) as AnchorComponent | null | undefined
  if (typeof anchor?.getBoundingClientRect === 'function') return anchor as RectangleSource
  const trigger = unwrap(anchor?.triggerEl) as AnchorComponent | null | undefined
  return typeof trigger?.getBoundingClientRect === 'function' ? (trigger as RectangleSource) : null
}
