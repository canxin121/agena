import { describe, expect, test } from 'bun:test'
import { copyPreviewArgs, parsePreviewArgs } from '../src/features/workspacePreview/model/previewArgs'

describe('preview argv', () => {
  test('a command can have no arguments', () => {
    expect(parsePreviewArgs('')).toEqual([])
    expect(parsePreviewArgs('  \n  ')).toEqual([])
    expect(parsePreviewArgs('[]')).toEqual([])
    expect(copyPreviewArgs([])).toEqual([])
  })

  test('structured argv preserves empty values, significant whitespace, Unicode and Windows paths', () => {
    const args = ['', '  My app  ', '中文路径/hello world', 'C:\\Users\\My Name\\app.js', 'a\nb']
    expect(parsePreviewArgs(JSON.stringify(args))).toEqual(args)
    const copied = copyPreviewArgs(args)
    expect(copied).toEqual(args)
    expect(copied).not.toBe(args)
  })

  test('quoted and escaped whitespace forms one argument', () => {
    expect(parsePreviewArgs('server.js --name "My app" --root \'folder with spaces\'')).toEqual([
      'server.js',
      '--name',
      'My app',
      '--root',
      'folder with spaces',
    ])
    expect(parsePreviewArgs('hello\\ world --name="My app"')).toEqual(['hello world', '--name=My app'])
    expect(parsePreviewArgs('"" \'\' "  "')).toEqual(['', '', '  '])
  })

  test('quoted backslashes and quote escapes survive without evaluation', () => {
    expect(parsePreviewArgs(String.raw`'C:\Users\My Name' "C:\Users\My Name" "say \"hi\""`)).toEqual([
      'C:\\Users\\My Name',
      'C:\\Users\\My Name',
      'say "hi"',
    ])
    expect(parsePreviewArgs('$HOME "$(touch /tmp/not-executed)" `whoami` ; &&')).toEqual([
      '$HOME',
      '$(touch /tmp/not-executed)',
      '`whoami`',
      ';',
      '&&',
    ])
  })

  test('ambiguous input produces actionable validation errors', () => {
    for (const text of ['"unfinished', "'unfinished", 'trailing\\', '["broken",]', '[1]', '[null]', '[{}]']) {
      expect(() => parsePreviewArgs(text)).toThrow('Invalid arguments')
    }
    expect(() => copyPreviewArgs(['ok', 1])).toThrow('array of strings')
  })
})
