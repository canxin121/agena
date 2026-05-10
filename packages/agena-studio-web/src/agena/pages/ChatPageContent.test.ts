import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/ChatPageContent.vue'

describe('ChatPageContent', () => {
  test('renders chat content panels from page content state', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('ChatSidebarPanel')).toBe(true)
    expect(source.includes('ChatActiveSessionPanel')).toBe(true)
    expect(source.includes('ChatMessagesPanel')).toBe(true)
    expect(source.includes('ChatTimelinePanel')).toBe(true)
    expect(source.includes('ChatPendingPermissionsPanel')).toBe(true)
    expect(source.includes('ChatPendingUserInputPanel')).toBe(true)
    expect(source.includes('ChatComposerPanel')).toBe(true)
    expect(source.includes('inspect-message')).toBe(true)
    expect(source.includes('inspected-message')).toBe(true)
    expect(source.includes('props.state.sidebar.selectedSessionId.value')).toBe(true)
    expect(source.includes('props.state.composer.value')).toBe(true)
  })
})
