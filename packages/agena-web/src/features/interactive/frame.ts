import type { AppManifest } from './model'

export const APP_CHANNEL = 'agena.interactive.v1'

/** No host cookies, storage, API credentials, or same-origin access are exposed.
 * CSP is placed before generated content, which cannot relax an earlier policy. */
export function appFrameDocument(
  manifest: AppManifest,
  token: string,
  state: unknown,
  locale: string,
  theme: string,
): string {
  const config = JSON.stringify({ token, state, locale, theme }).replace(/</g, '\\u003c')
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'">
<script>(()=>{
const config=${config}, channel=${JSON.stringify(APP_CHANNEL)}, pending=new Map();let sequence=0,widgetState=config.state,stateQueue=Promise.resolve(),environment={locale:config.locale,theme:config.theme};
const send=(type,payload)=>parent.postMessage({channel,token:config.token,type,...payload},'*');
const request=(method,payload={})=>new Promise((resolve,reject)=>{const id=String(++sequence);const timer=setTimeout(()=>{pending.delete(id);reject(new Error('Application request timed out'))},25000);pending.set(id,{resolve,reject,timer});send('request',{id,method,payload})});
addEventListener('message',event=>{const d=event.data;if(event.source!==parent||!d||d.channel!==channel||d.token!==config.token)return;if(d.type==='environment'){environment={locale:d.locale,theme:d.theme};document.documentElement.dataset.theme=d.theme;document.documentElement.style.colorScheme=d.theme;dispatchEvent(new CustomEvent('agena:environment',{detail:environment}));return}if(d.type!=='response')return;const p=pending.get(d.id);if(!p)return;pending.delete(d.id);clearTimeout(p.timer);d.error?p.reject(new Error(d.error)):p.resolve(d.result)});
const setState=async value=>{const serialized=JSON.stringify(value??null);if(new TextEncoder().encode(serialized).byteLength>65536)throw new Error('Widget state exceeds 64 KiB');const payload=JSON.parse(serialized),next=stateQueue.then(()=>request('state',{value:payload})).then(saved=>{widgetState=saved;return saved});stateQueue=next.catch(()=>{});return next};
Object.defineProperty(window,'agena',{value:Object.freeze({get locale(){return environment.locale},get theme(){return environment.theme},ready:Promise.resolve(config.state),getState:()=>widgetState,setState,fetch:async(service,path,init={})=>{const r=await request('fetch',{service,path,method:init.method||'GET',headers:Object.fromEntries(new Headers(init.headers||{})),body:init.body});return new Response([204,205,304].includes(r.status)?null:r.body,{status:r.status,headers:r.headers})}}),writable:false});
document.documentElement.dataset.theme=config.theme;document.documentElement.style.colorScheme=config.theme;
addEventListener('error',e=>send('error',{message:String(e.message||'Application error').slice(0,1000)}));addEventListener('unhandledrejection',e=>send('error',{message:String(e.reason?.message||e.reason||'Application error').slice(0,1000)}));
addEventListener('DOMContentLoaded',()=>{let last=0;new ResizeObserver(()=>{const height=Math.max(160,Math.min(1200,Math.max(Math.ceil(document.body.getBoundingClientRect().height),document.body.scrollHeight)));if(height!==last){last=height;send('resize',{height})}}).observe(document.body);send('ready',{})});
})();<\/script></head><body style="margin:0">${manifest.html}</body></html>`
}
