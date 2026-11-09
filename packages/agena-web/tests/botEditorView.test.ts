import { describe, expect, test } from 'bun:test'
import en from '../src/i18n/messages/en-US'
import zh from '../src/i18n/messages/zh-CN'
import es from '../src/i18n/messages/es-ES'
import fr from '../src/i18n/messages/fr-FR'
import hi from '../src/i18n/messages/hi-IN'
import ar from '../src/i18n/messages/ar-SA'
import pt from '../src/i18n/messages/pt-BR'

describe('Bot editor and sidebar copy', () => {
  test('all locales provide instructions-only labels and an empty preview', () => {
    for (const locale of [en, zh, es, fr, hi, ar, pt]) {
      const profiles = locale.bot.profiles
      expect(profiles.instructions).toBeTruthy()
      expect(profiles.instructionsNotSet).toBeTruthy()
      expect(profiles.instructionsCount).toContain('{count}')
      expect(profiles.instructionsCount).toContain('{max}')
      expect('tones' in profiles).toBe(false)
    }
    expect(zh.bot.profiles.instructionsNotSet).toBe('未设置附加系统指令')
  })
})
