import { afterEach, describe, expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { readViewportRect, useViewportObserver } from '../src/composables/useViewportObserver'

class Viewport extends EventTarget {
  width = 320.5
  height = 480.5
  offsetLeft = 12.5
  offsetTop = 24.5
}

class ViewportWindow extends EventTarget {
  innerWidth = 1024
  innerHeight = 768
  visualViewport: Viewport | undefined = new Viewport()
}

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
const scopes: ReturnType<typeof effectScope>[] = []

function installWindow(owner: ViewportWindow | undefined) {
  Object.defineProperty(globalThis, 'window', { value: owner, configurable: true, writable: true })
}

function observe(active: ReturnType<typeof ref<boolean>>, changed: () => void) {
  const scope = effectScope()
  scopes.push(scope)
  scope.run(() => useViewportObserver(() => active.value, changed))
  return scope
}

afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow)
  else Reflect.deleteProperty(globalThis, 'window')
})

describe('viewport observation lifecycle', () => {
  test('subscribes only while active and releases every event on close and scope disposal', async () => {
    const owner = new ViewportWindow()
    installWindow(owner)
    const active = ref(false)
    let changes = 0
    const scope = observe(active, () => { changes += 1 })
    const fire = () => {
      owner.dispatchEvent(new Event('resize'))
      owner.dispatchEvent(new Event('orientationchange'))
      owner.visualViewport?.dispatchEvent(new Event('resize'))
      owner.visualViewport?.dispatchEvent(new Event('scroll'))
    }
    fire()
    expect(changes).toBe(0)
    active.value = true
    await nextTick()
    // Observation itself must not trigger component layout or focus work.
    expect(changes).toBe(0)
    fire()
    expect(changes).toBe(4)
    active.value = false
    await nextTick()
    fire()
    expect(changes).toBe(4)
    active.value = true
    await nextTick()
    fire()
    expect(changes).toBe(8)
    scope.stop()
    fire()
    expect(changes).toBe(8)
  })

  test('cleanup removes listeners from the viewport that was actually observed', async () => {
    const owner = new ViewportWindow()
    installWindow(owner)
    const original = owner.visualViewport!
    const active = ref(true)
    let changes = 0
    observe(active, () => { changes += 1 })
    original.dispatchEvent(new Event('resize'))
    expect(changes).toBe(1)
    const replacement = new Viewport()
    owner.visualViewport = replacement
    active.value = false
    await nextTick()
    original.dispatchEvent(new Event('resize'))
    replacement.dispatchEvent(new Event('resize'))
    expect(changes).toBe(1)
    active.value = true
    await nextTick()
    original.dispatchEvent(new Event('scroll'))
    replacement.dispatchEvent(new Event('scroll'))
    expect(changes).toBe(2)
  })

  test('works without visualViewport and without a browser window', () => {
    const owner = new ViewportWindow()
    owner.visualViewport = undefined
    installWindow(owner)
    let changes = 0
    const scope = observe(ref(true), () => { changes += 1 })
    owner.dispatchEvent(new Event('resize'))
    expect(changes).toBe(1)
    expect(readViewportRect()).toEqual({ width: 1024, height: 768, left: 0, top: 0 })
    scope.stop()
    installWindow(undefined)
    expect(readViewportRect()).toEqual({ width: 0, height: 0, left: 0, top: 0 })
    expect(() => observe(ref(true), () => {})).not.toThrow()
  })

  test('preserves fractional visual geometry and the existing invalid-value fallbacks', () => {
    const owner = new ViewportWindow()
    installWindow(owner)
    expect(readViewportRect()).toEqual({ width: 320.5, height: 480.5, left: 12.5, top: 24.5 })
    Object.assign(owner.visualViewport!, { width: 0, height: NaN, offsetLeft: -10, offsetTop: Infinity })
    expect(readViewportRect()).toEqual({ width: 1024, height: 768, left: 0, top: 0 })
  })
})
