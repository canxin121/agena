// Deterministic browser fixture using the production components and SSE
// reducer. No server/client-specific storage or alternate renderer is involved.
import { createApp, h, reactive, ref } from 'vue'
import { createPinia } from 'pinia'
import { Terminal } from '@xterm/xterm'
import { i18n } from '../../src/i18n'
import OperationPart from '../../src/components/chat/AgenaOperationPart.vue'
import Markdown from '../../src/components/Markdown.vue'
import CodeBlock from '../../src/components/ui/CodeBlock.vue'
import '../../src/style.css'
import '@fontsource/ibm-plex-sans/400.css'
import '@fontsource/ibm-plex-mono/400.css'

const resources = new Map()
const streams = new Map()
const metrics = []
const text = ref('# Streamed answer\n\nStable first paragraph.\n\nLive tail')
const code = ref('small')
const fixture = window.partFixture = { streams, metrics, text, code, copiedText: '', terminals: new Map() }
Object.defineProperty(navigator, 'clipboard', { value: { writeText: async (value) => { fixture.copiedText = value } } })
const nativeWrite = Terminal.prototype.write
const nativeOpen = Terminal.prototype.open
Terminal.prototype.open = function (element) {
  fixture.terminals.set(element.closest('[data-fixture]')?.dataset.fixture, this)
  return nativeOpen.call(this, element)
}
Terminal.prototype.write = function (value, callback) {
  return nativeWrite.call(this, value, () => {
    callback?.()
    const matches = [...String(value).matchAll(/latency:(\d+):(\d+\.\d+)/g)]
    if (matches.length) requestAnimationFrame(() => requestAnimationFrame(() => {
      const now = performance.now()
      for (const match of matches) metrics.push({ sequence: Number(match[1]), ms: now - Number(match[2]) })
    }))
  })
}

const encoder = new TextEncoder()
const epoch = '00000000-0000-0000-0000-000000000001'
function resource(id, kind) {
  if (!resources.has(id)) resources.set(id, { resource_id: id, kind, owner_session_id:1, part_id:2, state:'active', cursor:{epoch,sequence:0}, committed_cursor:{epoch,sequence:0}, total_bytes:0, dropped_bytes:0, retained_ranges:[], chunks:[] })
  return resources.get(id)
}
const nativeFetch = window.fetch.bind(window)
window.fetch = async (url, options) => {
  const parsed = new URL(String(url), location.href)
  const match = /\/content\/([^/]+)(\/stream)?$/.exec(parsed.pathname)
  if (!match) {
    if (parsed.pathname.startsWith('/api/')) return new Response('{}', {headers:{'Content-Type':'application/json'}})
    return nativeFetch(url, options)
  }
  const id = match[1]
  if (match[2]) {
    const body = new ReadableStream({ start(controller) { streams.set(id, controller) }, cancel() { streams.delete(id) } })
    options?.signal?.addEventListener('abort', () => { try { streams.get(id)?.close() } catch {} streams.delete(id) }, {once:true})
    return new Response(body, {headers:{'Content-Type':'text/event-stream'}})
  }
  const value = resources.get(id)
  const chunks = value.chunks.filter((chunk) => chunk.cursor.sequence > Number(parsed.searchParams.get('after') || 0))
  return Response.json({resource:{...value,chunks:undefined},chunks,next_cursor:value.cursor,has_more:false,gap:value.dropped_bytes>0})
}

function part(id, kind, command) {
  resource(id, kind)
  const reference = {resource_id:id,kind}
  return reactive({ key:id,id:'2',kind:'operation',status:'in_progress',role:'assistant',title:command,summary:'',copyText:'',toggleable:true,defaultExpanded:true,
    source:{id:'2',agenaKind:'tool_call',partState:'in_progress',revision:1,updatedAt:1,agenaSections:[{section:'presentation',revision:1}],
      agenaContent:{name:'shell.exec',call_id:1,state:'in_progress',input:{command,workdir:'/workspace/agena'},lifecycle:{start_ms:1},resources:[reference]},
      agenaPresentation:{blocks:[{type:'command',id:'command',command,cwd:'/workspace/agena',exit_code:null},{type:'content',id:'output',resource:reference}]}} })
}
const shell = part('shell', 'log', 'bun run build')
const pty = part('pty', 'terminal', 'interactive terminal')
const expanded = reactive({shell:true,pty:true})
fixture.expanded = expanded
fixture.emit = (id, payload, state='active') => {
  const value = resource(id, id === 'pty' ? 'terminal' : 'log')
  const chunk = {cursor:{epoch,sequence:value.cursor.sequence+1},captured_at_ms:Date.now(),payload}
  value.chunks.push(chunk)
  value.cursor = chunk.cursor
  value.committed_cursor = chunk.cursor
  value.state = state
  value.total_bytes += 'text' in payload ? encoder.encode(payload.text).length : 0
  value.retained_ranges = [{first:1,last:chunk.cursor.sequence}]
  const page = {resource:{...value,chunks:undefined},chunks:[chunk],next_cursor:chunk.cursor,has_more:false,gap:false}
  streams.get(id)?.enqueue(encoder.encode(`event: content\ndata: ${JSON.stringify(page)}\n\n`))
  if (state !== 'active') {
    const operation = id === 'pty' ? pty : shell
    operation.status = state === 'complete' ? 'completed' : 'failed'
    operation.source.partState = operation.status
    operation.source.agenaContent.state = operation.status
    operation.source.agenaContent.lifecycle.end_ms = 1201
    try { streams.get(id)?.close() } catch {}
  }
}
fixture.markdown = (value) => { text.value = value }
fixture.setCode = (value) => { code.value = value }
fixture.theme = (dark) => document.documentElement.classList.toggle('dark', dark)

createApp({render:() => h('main', {class:'mx-auto max-w-4xl space-y-6 p-6',style:'font-family:var(--font-sans)'}, [
  h('h1',{class:'text-xl font-semibold'},'Part streaming'),
  h('section',{'data-fixture':'shell'},[h(OperationPart,{part:shell,expanded:expanded.shell,collapseSignal:0,sessionId:'1',onToggle:()=>expanded.shell=!expanded.shell})]),
  h('section',{'data-fixture':'pty'},[h(OperationPart,{part:pty,expanded:expanded.pty,collapseSignal:0,sessionId:'1',onToggle:()=>expanded.pty=!expanded.pty})]),
  h('section',{'data-fixture':'markdown'},[h(Markdown,{content:text.value,stream:true})]),
  h('section',{'data-fixture':'code'},[h(CodeBlock,{code:code.value,lang:'text',compact:true})]),
])}).use(createPinia()).use(i18n).mount('#app')
