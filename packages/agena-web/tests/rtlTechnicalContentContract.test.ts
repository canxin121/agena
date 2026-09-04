import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const codeEditor = readFileSync(new URL('../src/components/MonacoCodeEditor.vue', import.meta.url), 'utf8')
const diffEditor = readFileSync(new URL('../src/components/MonacoDiffEditor.vue', import.meta.url), 'utf8')
const terminalPage = readFileSync(new URL('../src/pages/TerminalPage.vue', import.meta.url), 'utf8')
const terminalDock = readFileSync(
  new URL('../src/features/terminal/components/TerminalDockPanel.vue', import.meta.url),
  'utf8',
)
const style = readFileSync(new URL('../src/style.css', import.meta.url), 'utf8')

test('code editors and terminals stay left-to-right inside an RTL application shell', () => {
  assert.match(codeEditor, /class="monaco-host"[\s\S]*dir="ltr"/)
  assert.match(diffEditor, /class="monaco-diff-host"[\s\S]*dir="ltr"/)
  assert.match(terminalPage, /ref="el" dir="ltr"/)
  assert.match(terminalDock, /ref="el" dir="ltr"/)
})

test('markdown code blocks keep source-code direction in RTL locales', () => {
  assert.match(style, /\.prose :where\(pre\)[\s\S]*direction: ltr;[\s\S]*text-align: left;/)
  assert.match(style, /\.oc-md-pre[\s\S]*direction: ltr;[\s\S]*text-align: left;/)
})
