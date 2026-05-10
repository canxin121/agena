import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/RuntimeSkillsPageContent.vue'

describe('RuntimeSkillsPageContent', () => {
  test('wires shared runtime catalog actions into the skills shell', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('RuntimeSkillsPanel')).toBe(true)
    expect(source.includes(':open-runtime-config-root="props.skills.openRuntimeConfigRoot"')).toBe(true)
    expect(source.includes(':open-plugin-logs-workspace-path="props.skills.openPluginLogsWorkspacePath"')).toBe(true)
    expect(source.includes(':open-runtime-entry-in-chat="props.skills.openRuntimeEntryInChat"')).toBe(true)
    expect(source.includes(':open-runtime-entry-source="props.skills.openRuntimeEntrySource"')).toBe(true)
  })
})
