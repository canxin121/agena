export const COMPOSER_TOOLBAR_WRAP_MAX_WIDTH = 430

export type ComposerToolbarLayout = {
  wrapChips: boolean
  stackActionsRow: boolean
  allowHorizontalScroll: boolean
  splitChipRows: boolean
}

const SINGLE_ROW_LAYOUT: ComposerToolbarLayout = {
  wrapChips: false,
  stackActionsRow: false,
  allowHorizontalScroll: true,
  splitChipRows: false,
}

const TWO_ROW_LAYOUT: ComposerToolbarLayout = {
  wrapChips: true,
  stackActionsRow: false,
  allowHorizontalScroll: false,
  splitChipRows: true,
}

export function shouldWrapComposerToolbar(isCompactTouch: boolean, viewportWidth: number): boolean {
  if (!isCompactTouch) return false
  if (!Number.isFinite(viewportWidth)) return false
  return Math.floor(viewportWidth) <= COMPOSER_TOOLBAR_WRAP_MAX_WIDTH
}

export function resolveComposerToolbarLayout(isCompactTouch: boolean, viewportWidth: number): ComposerToolbarLayout {
  return shouldWrapComposerToolbar(isCompactTouch, viewportWidth) ? TWO_ROW_LAYOUT : SINGLE_ROW_LAYOUT
}
