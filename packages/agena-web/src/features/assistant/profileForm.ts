import type { AssistantPreferences, AssistantPatch } from '@/generated/agenaState'
export function minuteText(value: number): string {
  return `${String(Math.floor(value / 60)).padStart(2, '0')}:${String(value % 60).padStart(2, '0')}`
}
export function parseMinute(value: string): number | null {
  if (!/^\d{2}:\d{2}$/.test(value)) return null
  const [hour, minute] = value.split(':').map(Number)
  return hour! < 24 && minute! < 60 ? hour! * 60 + minute! : null
}
export function validPreferences(p: AssistantPreferences): boolean {
  return (
    Boolean(p.name?.trim() && p.language?.trim() && p.timezone?.trim()) &&
    new TextEncoder().encode(p.instructions || '').length <= 4096 &&
    p.daily_limit >= 1 &&
    p.daily_limit <= 24 &&
    p.cooldown_seconds >= 60 &&
    p.cooldown_seconds <= 86400 &&
    p.quiet_start_minute >= 0 &&
    p.quiet_start_minute < 1440 &&
    p.quiet_end_minute >= 0 &&
    p.quiet_end_minute < 1440 &&
    (!p.quiet_enabled || p.quiet_start_minute !== p.quiet_end_minute)
  )
}
export function profileChanged(before: AssistantPreferences, after: AssistantPreferences): boolean {
  return JSON.stringify(before) !== JSON.stringify(after)
}
/** Send only deliberate changes, keeping the draft's original revision until
 * the user explicitly reconciles a conflict with a refreshed profile. */
export function preferencePatch(before: AssistantPreferences, after: AssistantPreferences): AssistantPatch {
  return Object.fromEntries(
    Object.entries(after).filter(([key, value]) => before[key as keyof AssistantPreferences] !== value),
  ) as AssistantPatch
}
export function rebasePreferences(
  before: AssistantPreferences,
  draft: AssistantPreferences,
  latest: AssistantPreferences,
): AssistantPreferences {
  return { ...latest, ...preferencePatch(before, draft) } as AssistantPreferences
}
