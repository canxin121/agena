import { describe, expect, test } from 'bun:test'

import {
  activeNotifications,
  createLocalNotification,
  isNotificationActive,
  mergeServerList,
  pushLocalNotification,
  selectBanners,
  selectToasts,
  sortNotifications,
  upsertNotification,
} from './notificationsModel'
import type { NotificationResource } from './types'

function sampleNotification(overrides: Partial<NotificationResource> = {}): NotificationResource {
  return {
    id: 'n1',
    kind: { kind: 'notice', code: 'test' },
    severity: 'info',
    scope: 'global',
    surface: 'banner',
    source: 'runtime',
    summary: 'hello',
    detail: null,
    control: 'dismiss',
    actions: [],
    priority: 0,
    dedup_key: null,
    created_at_ms: 1_000,
    expires_at_ms: null,
    dismissed: false,
    ...overrides,
  }
}

describe('notificationsModel', () => {
  test('sortNotifications orders newest first, priority breaking ties', () => {
    const list = [
      sampleNotification({ id: 'a', created_at_ms: 100, priority: 0 }),
      sampleNotification({ id: 'b', created_at_ms: 300, priority: 0 }),
      sampleNotification({ id: 'c', created_at_ms: 200, priority: 5 }),
      sampleNotification({ id: 'd', created_at_ms: 200, priority: 1 }),
    ]
    expect(sortNotifications(list).map((n) => n.id)).toEqual(['b', 'c', 'd', 'a'])
  })

  test('isNotificationActive honours dismissed and expires_at_ms', () => {
    const now = 5_000
    expect(isNotificationActive(sampleNotification({ dismissed: false }), now)).toBe(true)
    expect(isNotificationActive(sampleNotification({ dismissed: true }), now)).toBe(false)
    expect(isNotificationActive(sampleNotification({ expires_at_ms: 4_000 }), now)).toBe(false)
    expect(isNotificationActive(sampleNotification({ expires_at_ms: 6_000 }), now)).toBe(true)
  })

  test('activeNotifications filters dismissed and expired entries', () => {
    const list = [
      sampleNotification({ id: 'a', dismissed: true }),
      sampleNotification({ id: 'b', expires_at_ms: 100 }),
      sampleNotification({ id: 'c' }),
    ]
    expect(activeNotifications(list, 5_000).map((n) => n.id)).toEqual(['c'])
  })

  test('upsertNotification replaces by id and preserves order', () => {
    const list = [sampleNotification({ id: 'a' }), sampleNotification({ id: 'b' })]
    const updated = upsertNotification(list, sampleNotification({ id: 'b', summary: 'changed' }))
    expect(updated.length).toBe(2)
    expect(updated[1].summary).toBe('changed')
  })

  test('mergeServerList keeps entries that arrived during a fetch', () => {
    const fetched = [sampleNotification({ id: 'a' })]
    const current = [sampleNotification({ id: 'b' })]
    const merged = mergeServerList(current, fetched)
    expect(merged.map((n) => n.id).sort()).toEqual(['a', 'b'])
  })

  test('selectBanners prefers higher priority then newest', () => {
    const list = [
      sampleNotification({ id: 'old-low', priority: 1, created_at_ms: 100 }),
      sampleNotification({ id: 'new-high', priority: 5, created_at_ms: 200 }),
      sampleNotification({ id: 'new-mid', priority: 3, created_at_ms: 300 }),
      sampleNotification({ id: 'toast', surface: 'toast', priority: 9, created_at_ms: 400 }),
    ]
    expect(selectBanners(list).map((n) => n.id)).toEqual(['new-high', 'new-mid', 'old-low'])
  })

  test('selectToasts only returns active toasts newest first', () => {
    const list = [
      sampleNotification({ id: 't1', surface: 'toast', created_at_ms: 100, dismissed: true }),
      sampleNotification({ id: 't2', surface: 'toast', created_at_ms: 400 }),
      sampleNotification({ id: 't3', surface: 'toast', created_at_ms: 200 }),
    ]
    expect(selectToasts(list).map((n) => n.id)).toEqual(['t2', 't3'])
  })

  test('createLocalNotification is frontend-source with expiry when ttlMs given', () => {
    const local = createLocalNotification(
      { severity: 'success', surface: 'toast', summary: 'done', ttlMs: 4_000 },
      'local-1',
      1_000,
    )
    expect(local.source).toBe('frontend')
    expect(local.id).toBe('local-1')
    expect(local.expires_at_ms).toBe(5_000)
    expect(local.control).toBe('dismiss')
  })

  test('pushLocalNotification replaces the single banner slot', () => {
    const first = pushLocalNotification([], { severity: 'error', surface: 'banner', summary: 'first' }, 'l1', 1_000)
    expect(first.map((n) => n.summary)).toEqual(['first'])
    const second = pushLocalNotification(first, { severity: 'info', surface: 'banner', summary: 'second' }, 'l2', 2_000)
    expect(second.map((n) => n.summary)).toEqual(['second'])
  })

  test('pushLocalNotification prunes expired toasts', () => {
    const withToast = pushLocalNotification(
      [],
      { severity: 'info', surface: 'toast', summary: 't', ttlMs: 100 },
      'l1',
      1_000,
    )
    const pruned = pushLocalNotification(
      withToast,
      { severity: 'info', surface: 'toast', summary: 't2', ttlMs: 100 },
      'l2',
      2_000,
    )
    expect(pruned.map((n) => n.id)).toEqual(['l2'])
  })
})
