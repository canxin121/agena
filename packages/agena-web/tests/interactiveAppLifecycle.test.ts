import { describe, expect, test } from 'bun:test'
import { effectScope } from 'vue'
import { useAppLifecycle } from '../src/features/interactive/useAppLifecycle'

describe('interactive app startup cancellation', () => {
  test('Stop aborts a pending Run and ignores its late finalizer', () => {
    const scope = effectScope()
    const lifecycle = scope.run(useAppLifecycle)!
    try {
      const run = lifecycle.begin('run')!
      expect(lifecycle.begin('run')).toBeUndefined()
      const stop = lifecycle.begin('stop')!
      expect(run.signal.aborted).toBe(true)
      expect(run.isCurrent()).toBe(false)
      run.finish()
      expect(lifecycle.phase.value).toBe('stop')
      expect(stop.isCurrent()).toBe(true)
      expect(lifecycle.begin('run')).toBeUndefined()
      stop.finish()
      expect(lifecycle.phase.value).toBe('')
      expect(lifecycle.begin('run')).toBeDefined()
    } finally {
      scope.stop()
    }
  })

  test('scope changes and unmount abort pending lifecycle requests without cancelling completed runs', () => {
    const scope = effectScope()
    const lifecycle = scope.run(useAppLifecycle)!
    const first = lifecycle.begin('run')!
    lifecycle.cancel()
    expect(first.signal.aborted).toBe(true)
    const finished = lifecycle.begin('run')!
    finished.finish()
    const pending = lifecycle.begin('run')!
    scope.stop()
    expect(pending.signal.aborted).toBe(true)
    expect(finished.signal.aborted).toBe(false)
    expect(lifecycle.phase.value).toBe('')
  })
})
