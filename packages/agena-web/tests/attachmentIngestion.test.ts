import assert from 'node:assert/strict'
import test from 'node:test'
import { createAttachmentIngestor, filesFromClipboard, MAX_ATTACHMENT_BYTES, type StagedAttachment } from '../src/pages/chat/attachmentIngestion'

const file = (name: string, text: string) => new File([text], name, {type:'text/plain'})
const url = async (file: File) => 'data:text/plain;base64,' + Buffer.from(await file.arrayBuffer()).toString('base64')
function harness(read: (file: File, signal: AbortSignal) => Promise<string> = url) {
  let files: StagedAttachment[] = []
  let busy = 0
  const errors: string[] = []
  const ingestor = createAttachmentIngestor({get:()=>files,set:(value)=>{files=value},read,onError:({kind})=>errors.push(kind),onBusy:(value)=>{busy=value}})
  return {ingestor,get files(){return files},get busy(){return busy},errors}
}
test('simultaneous paste/drop queues are ordered and cannot overbook slots', async()=>{
  const h=harness()
  const first=h.ingestor.stage(Array.from({length:5},(_,i)=>file(`a${i}.txt`,String(i))))
  const second=h.ingestor.stage(Array.from({length:5},(_,i)=>file(`b${i}.txt`,String(i))))
  assert.equal(h.busy,2)
  await Promise.all([first,second])
  assert.equal(h.files.length,8);assert.equal(h.busy,0);assert.ok(h.errors.includes('count'))
  assert.deepEqual(h.files.slice(0,5).map(f=>f.filename),['a0.txt','a1.txt','a2.txt','a3.txt','a4.txt'])
  assert.ok(h.files.every(f=>f.delivery==='model_input'))
})
test('same filename and size do not suppress distinct screenshot contents',async()=>{
  const h=harness()
  await h.ingestor.stage([file('same.png','AAA'),file('same.png','BBB'),file('same.png','AAA')])
  assert.equal(h.files.length,2)
})
test('clear aborts earlier read and does not block or corrupt a newer draft',async()=>{
  let finishOld!: (data:string)=>void;let oldSignal: AbortSignal|undefined
  const h=harness((f,signal)=>f.name==='old.txt'?(oldSignal=signal,new Promise(resolve=>{finishOld=resolve})):url(f))
  const old=h.ingestor.stage([file('old.txt','OLD')]);await Promise.resolve()
  assert.equal(h.busy,1);h.ingestor.clear();assert.equal(h.busy,0);assert.equal(oldSignal?.aborted,true)
  await h.ingestor.stage([file('new.txt','NEW')]);assert.equal(h.files[0]?.filename,'new.txt')
  finishOld('data:text/plain;base64,T0xE');await old
  assert.equal(h.files.length,1);assert.equal(h.files[0]?.filename,'new.txt');assert.equal(h.busy,0)
})
test('a failed read returns no accepted file so mixed long-paste text can be restored',async()=>{
  const h=harness(async f=>{if(f.name==='bad.txt')throw new Error('read failed');return url(f)})
  const good=file('good.txt','text'),bad=file('bad.txt','long text')
  const accepted=await h.ingestor.stage([bad,good]);assert.deepEqual(accepted,[good]);assert.deepEqual(h.errors,['read']);assert.equal(h.busy,0)
})
test('individual and total byte limits reject before reading',async()=>{
  let reads=0;const h=harness(async f=>{reads++;return url(f)})
  const huge=file('huge.txt','x');Object.defineProperty(huge,'size',{value:MAX_ATTACHMENT_BYTES+1})
  await h.ingestor.stage([huge]);assert.equal(reads,0);assert.deepEqual(h.errors,['size'])
  const sized=Array.from({length:3},(_,i)=>{const f=file(`${i}.txt`,String(i));Object.defineProperty(f,'size',{value:MAX_ATTACHMENT_BYTES});return f})
  await h.ingestor.stage(sized);assert.equal(reads,2);assert.equal(h.files.length,2);assert.ok(h.errors.includes('total'))
})
test('clipboard files are not duplicated when supplied in both browser collections',()=>{
  const image=file('clipboard.png','image')
  const data={items:[{kind:'string',getAsFile:()=>null},{kind:'file',getAsFile:()=>image}],files:[image]} as unknown as Pick<DataTransfer,'items'|'files'>
  assert.deepEqual(filesFromClipboard(data),[image]);assert.deepEqual(filesFromClipboard(null),[])
  assert.deepEqual(filesFromClipboard({items:[],files:[image]} as unknown as Pick<DataTransfer,'items'|'files'>),[image])
})

test('failed send recovery never overwrites a new draft or another session', async()=>{
  const {createFailedAttachmentDraftSlot}=await import('../src/pages/chat/failedAttachmentDrafts')
  const slot=createFailedAttachmentDraftSlot()
  slot.save({sessionId:'a',text:'failed text',files:[{id:'x',filename:'image.png',size:4,mime:'image/png',url:'data:image/png;base64,AAAA',delivery:'model_input'}]})
  assert.equal(slot.take('b','',[]),null);assert.equal(slot.take('a','new text',[]),null)
  assert.throws(()=>slot.save({sessionId:'a',text:'later',files:[]}))
  assert.equal(slot.peek()?.text,'failed text')
  const recovered=slot.take('a','',[]);assert.equal(recovered?.files[0]?.delivery,'model_input');assert.equal(recovered?.text,'failed text');assert.equal(slot.peek(),null)
})

test('large clipboard text is bounded by UTF-8 bytes before staging',async()=>{
  const {pasteTextWithinBudget,MAX_PASTE_TEXT_BYTES}=await import('../src/pages/chat/attachmentIngestion')
  assert.equal(pasteTextWithinBudget('a'.repeat(MAX_PASTE_TEXT_BYTES)),true)
  assert.equal(pasteTextWithinBudget('a'.repeat(MAX_PASTE_TEXT_BYTES+1)),false)
  assert.equal(pasteTextWithinBudget('中'.repeat(400000)),false)
  assert.equal(pasteTextWithinBudget('😀'.repeat(200000)),true)
})
