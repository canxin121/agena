// Built-in review requests carry stable English protocol values. Translate
// their display text at render time; selections and replies keep the originals.
const reviewTextKeys: Record<string, string> = {
  'Approve New Plan': 'creationTitle',
  'Review Plan Status Change': 'statusTitle',
  Decision: 'decision',
  Approve: 'approve',
  'Approve with autorun on': 'approveAutorunOn',
  'Approve with autorun off': 'approveAutorunOff',
  'Approve requested status': 'approveRequested',
  'Approve requested status with auto-continue off': 'approveRequestedPause',
  'Keep in planning': 'keepPlanning',
  Reject: 'reject',
  'Cancel plan': 'cancelPlan',
  'Approve the plan, move it to active, and keep autorun on.': 'approveAutorunOnDescription',
  'Approve the plan, move it to active, and keep autorun off.': 'approveAutorunOffDescription',
  'Approve the plan and move it to blocked.': 'approveBlockedDescription',
  'Approve the plan and mark it completed.': 'approveCompletedDescription',
  'Approve the plan and return it to planning.': 'approvePlanningDescription',
  'Approve the plan and cancel it.': 'approveCancelledDescription',
  'Return to planning so the plan can be edited further.': 'keepPlanningDescription',
  'Reject the current plan and mark the review as rejected.': 'rejectDescription',
  'Cancel the plan entirely and stop work on it.': 'cancelPlanDescription',
}

type Translate = (key: string, values?: Record<string, string>) => string

export function interactionText(kind: string, value: string, translate: Translate): string {
  if (kind.trim().toLowerCase() !== 'review') return value
  const key = Object.hasOwn(reviewTextKeys, value) ? reviewTextKeys[value] : undefined
  if (key) return translate(`chat.planReview.${key}`)
  const question =
    /^Choose whether this plan should move to (planning|active|blocked|completed|cancelled), or type feedback for the agent to revise the plan\.$/.exec(
      value,
    )
  if (question) {
    return translate('chat.planReview.question', { phase: translate(`chat.planReview.phases.${question[1]}`) })
  }
  return value
}
