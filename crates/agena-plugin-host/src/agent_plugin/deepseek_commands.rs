//! Run package-local Cordis modules that contribute commands or MCP servers.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use agena_plugin_sdk::{
    CommandHostEffect, CommandInvokeInput, CommandResult, HostClient, PluginError, PluginErrorKind,
    host_api::{
        CordisLlmRequest, CordisSubagentControl, CordisWorkflowJobRequest,
        DeepSeekMcpManagerRequest,
    },
};
use futures_util::stream::{FuturesUnordered, StreamExt};
use rquickjs::{
    CatchResultExt, Context, Ctx, Function, Module, Object, Promise,
    function::Func,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use serde_yaml::Value as Yaml;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{
    CordisDependency, CordisService, bridge::AgentBridge, deepseek, deepseek_js, manifest_path,
    read_package_bytes, workflow,
};

mod packages;
mod controllers;
mod jobs;
mod prompt;
mod retained;
mod skills;
pub(super) mod storage;
mod subagents;
mod tools;
pub use tools::decode_content_blocks;
pub(super) mod events;
pub(super) use skills::Skill as CordisSkill;
pub(super) use prompt::assemble as assemble_system_prompt;

const MAX_SCRIPT_BYTES: u64 = 256 * 1024;
const MAX_MODULE_IMPORTS: usize = 32;
const MAX_PACKAGE_COMMANDS: usize = 256;
const MAX_PACKAGE_TOOLS: usize = 128;
const MAX_ARGUMENT_BYTES: usize = 16 * 1024;
const MAX_PROMPT_BYTES: usize = 128 * 1024;
const MAX_RESULT_CHARS: usize = 8_192;
const MAX_HOST_OPERATIONS: usize = 32;
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(1);
static WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);
const MCP_MANAGER: &str = include_str!("deepseek_commands/mcp.js");
const NODE_BUILTINS: &str = include_str!("deepseek_commands/node_builtins.js");
const WORKFLOW_ENGINE: &str = include_str!("deepseek_commands/workflow.js");
const WORKFLOW_TOOL_MODULE: &str = include_str!("deepseek_commands/tool_workflow.js");
const JOBS_TOOL_MODULE: &str = include_str!("deepseek_commands/tool_jobs.js");
const SUBAGENT_TOOL_MODULE: &str = include_str!("deepseek_commands/tool_subagent.js");
const SUBAGENT_CONTROL_MODULE: &str = include_str!("deepseek_commands/tool_subagent_control.js");
pub(super) const WORKFLOW_TOOL: &str = "@deepseek-ai/dsh-tool-workflow";
pub(super) const WORKFLOW_PTC: &str = "@deepseek-ai/dsh-workflow-ptc";
pub(super) const SUBAGENT: &str = "@deepseek-ai/dsh-subagent";
pub(super) const SUBAGENT_SPAWN: &str = "@deepseek-ai/dsh-subagent-spawn-in-process";
pub(super) const SUBAGENT_FORK: &str = "@deepseek-ai/dsh-subagent-fork-in-process";
pub(super) const SUBAGENT_TOOL: &str = "@deepseek-ai/dsh-tool-subagent";
pub(super) const SUBAGENT_CONTROL_TOOL: &str = "@deepseek-ai/dsh-tool-subagent-control";
pub(super) const JOBS_TOOL: &str = "@deepseek-ai/dsh-tool-jobs";
pub(super) const JOBS_LOCAL: &str = "@deepseek-ai/dsh-jobs-local";

const MESSAGE_HELPERS: &str = r#"
const randomUUID = globalThis.agenaCordisRandomUuid;
delete globalThis.agenaCordisRandomUuid;
function detached(value) {
    if (value === null || typeof value !== 'object') return value;
    if (Array.isArray(value)) return Object.freeze(value.map(detached));
    return Object.freeze(Object.fromEntries(
        Object.entries(value).map(([key, value]) => [key, detached(value)])));
}
export function createUserMessage(input) {
    return detached({ ...input, role: 'user', id: randomUUID() });
}
"#;

const COMMAND_IDENTITIES: &str = r#"
export const CommandDefinitionId = id => id;
export const CommandId = id => id;
"#;

const NODE_URL: &str = r#"
const binding = globalThis.agenaCordisNodeUrl;
delete globalThis.agenaCordisNodeUrl;
export const URL = binding.URL;
export const URLSearchParams = globalThis.URLSearchParams;
export const fileURLToPath = binding.fileURLToPath;
export const pathToFileURL = binding.pathToFileURL;
export default Object.freeze({ URL, URLSearchParams, fileURLToPath, pathToFileURL });
"#;

const NODE_PATH: &str = r#"
const path = globalThis.agenaCordisNodePath;
delete globalThis.agenaCordisNodePath;
export const sep = path.sep;
export const delimiter = path.delimiter;
export const normalize = path.normalize;
export const join = path.join;
export const resolve = path.resolve;
export const isAbsolute = path.isAbsolute;
export const dirname = path.dirname;
export const basename = path.basename;
export const extname = path.extname;
export const relative = path.relative;
export const parse = path.parse;
export const format = path.format;
export const posix = path.posix;
export default path;
"#;

const NODE_BUFFER: &str = r#"
export const Buffer = globalThis.Buffer;
export default Object.freeze({ Buffer });
"#;

const NODE_CRYPTO: &str = r#"
const cryptoUuid = globalThis.agenaCordisCryptoRandomUuid;
const cryptoRandomBytes = globalThis.agenaCordisCryptoRandomBytes;
const cryptoDigest = globalThis.agenaCordisCryptoDigest;
delete globalThis.agenaCordisCryptoRandomUuid;
delete globalThis.agenaCordisCryptoRandomBytes;
delete globalThis.agenaCordisCryptoDigest;

export function randomUUID() {
    return cryptoUuid();
}

export function randomBytes(size) {
    if (!Number.isSafeInteger(size) || size < 0 || size > 65536) {
        throw new RangeError('Cordis crypto.randomBytes size must be 0..=65536');
    }
    return Buffer.from(cryptoRandomBytes(size), 'base64');
}

export function createHash(algorithm) {
    const alg = String(algorithm).toLowerCase().replace(/-/g, '');
    if (alg !== 'sha256' && alg !== 'sha384' && alg !== 'sha512') {
        throw new TypeError(`Unsupported hash algorithm: ${algorithm}`);
    }
    const chunks = [];
    let total = 0;
    let finished = false;
    return {
        update(data, inputEncoding) {
            if (finished) throw new Error('Digest already called');
            const bytes = typeof data === 'string'
                ? Buffer.from(data, inputEncoding ?? 'utf8')
                : ArrayBuffer.isView(data)
                    ? Buffer.from(data)
                    : (() => { throw new TypeError('Hash.update requires a string or TypedArray'); })();
            total += bytes.length;
            if (total > 8 * 1024 * 1024) throw new RangeError('Hash input exceeds 8 MiB limit');
            chunks.push(bytes);
            return this;
        },
        digest(encoding) {
            if (finished) throw new Error('Digest already called');
            finished = true;
            const joined = chunks.length === 1 ? chunks[0] : Buffer.concat(chunks, total);
            const base64 = cryptoDigest(alg, joined.toString('base64'));
            const out = Buffer.from(base64, 'base64');
            return encoding === undefined ? out : out.toString(encoding);
        },
    };
}

export function timingSafeEqual(a, b) {
    if (!ArrayBuffer.isView(a) || !ArrayBuffer.isView(b)) {
        throw new TypeError('timingSafeEqual requires TypedArray arguments');
    }
    if (a.byteLength !== b.byteLength) {
        throw new RangeError('Input buffers must have the same byte length');
    }
    const left = new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
    const right = new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
    let diff = 0;
    for (let i = 0; i < left.length; i++) diff |= left[i] ^ right[i];
    return diff === 0;
}

export default Object.freeze({ randomUUID, randomBytes, createHash, timingSafeEqual });
"#;

const NODE_FS_PROMISES: &str = r#"
const readText = globalThis.agenaCordisReadPackageText;
const readBase64 = globalThis.agenaCordisReadPackageBase64;
delete globalThis.agenaCordisReadPackageText;
delete globalThis.agenaCordisReadPackageBase64;
export async function readFile(file, options) {
    const encoding = typeof options === 'string' ? options : options?.encoding;
    options?.signal?.throwIfAborted();
    if (encoding === undefined || encoding === null) {
        return Buffer.from(readBase64(String(file)), 'base64');
    }
    if (encoding === 'utf8' || encoding === 'utf-8') {
        return readText(String(file));
    }
    if (encoding === 'base64' || encoding === 'base64url' || encoding === 'hex' || encoding === 'latin1' || encoding === 'binary' || encoding === 'ascii') {
        return Buffer.from(readBase64(String(file)), 'base64').toString(encoding);
    }
    throw new TypeError(`Unsupported Cordis readFile encoding: ${encoding}`);
}
export default Object.freeze({ readFile });
"#;

const NODE_EVENTS: &str = r#"
export const EventEmitter = globalThis.agenaCordisNodeEvents;
export default EventEmitter;
"#;

const NODE_UTIL: &str = r#"
const util = globalThis.agenaCordisNodeUtil;
export const { format, inspect, promisify, TextEncoder, TextDecoder, types, isDeepStrictEqual } = util;
export default util;
"#;

const NODE_ASSERT: &str = r#"
const assert = globalThis.agenaCordisNodeAssert;
export const {
    ok, equal, notEqual, strictEqual, notStrictEqual,
    deepEqual, deepStrictEqual, notDeepStrictEqual,
    throws, rejects, match, fail, AssertionError, strict,
} = assert;
export default assert;
"#;

const NODE_QUERYSTRING: &str = r#"
const qs = globalThis.agenaCordisNodeQuerystring;
export const { parse, stringify, escape, unescape } = qs;
export const decode = parse;
export const encode = stringify;
export default qs;
"#;

const NODE_OS: &str = r#"
const os = globalThis.agenaCordisNodeOs;
export const { EOL, platform, arch, endianness, constants } = os;
export default os;
"#;

fn builtin_module(name: &str) -> Option<&'static str> {
    match name {
        "@deepseek-ai/dsh-llm" => Some(MESSAGE_HELPERS),
        "@deepseek-ai/dsh-commands" | "@deepseek-ai/dsh-commands/brand" => Some(COMMAND_IDENTITIES),
        "@deepseek-ai/dsh-tools" => Some(tools::MODULE),
        "@deepseek-ai/dsh-workflow" | "@deepseek-ai/dsh-workflow/types" => {
            Some("export const WorkflowRunId = id => id;")
        }
        "@deepseek-ai/dsh-jobs" | "@deepseek-ai/dsh-jobs/brand" | "@deepseek-ai/dsh-jobs/types" => {
            Some("export const JobId = id => id;")
        }
        storage::HUB => Some(storage::HUB_MODULE),
        storage::DOMAIN => Some(storage::DOMAIN_MODULE),
        "@deepseek-ai/dsh-skill" => Some(skills::EXPORTS),
        SUBAGENT => Some(subagents::EXPORTS),
        "node:url" | "url" => Some(NODE_URL),
        "node:path" | "path" | "node:path/posix" | "path/posix" => Some(NODE_PATH),
        "node:buffer" | "buffer" => Some(NODE_BUFFER),
        "node:crypto" | "crypto" => Some(NODE_CRYPTO),
        "node:fs/promises" | "fs/promises" => Some(NODE_FS_PROMISES),
        "node:events" | "events" => Some(NODE_EVENTS),
        "node:util" | "util" => Some(NODE_UTIL),
        "node:assert" | "assert" | "node:assert/strict" | "assert/strict" => Some(NODE_ASSERT),
        "node:querystring" | "querystring" => Some(NODE_QUERYSTRING),
        "node:os" | "os" => Some(NODE_OS),
        "node:timers" | "timers" => Some(jobs::TIMERS),
        "node:timers/promises" | "timers/promises" => Some(jobs::TIMERS_PROMISES),
        _ => None,
    }
}

const MODULE_SERVICES: &str = r#"
(async (module, config, manager, {prompt, skills, jobs, subagents}, storage, workflow, report) => {
    const plugin = typeof module.apply === 'function' ? module : module.default;
    const declaration = plugin?.inject ?? module.inject;
    if (!plugin || (typeof plugin !== 'function' && typeof plugin.apply !== 'function')) {
        throw new Error('expected a Cordis module with an apply function');
    }
    if (declaration != null && typeof declaration !== 'object') {
        throw new Error('expected an array or map of Cordis service dependencies');
    }
    const available = new Set(['commands', 'mcpManager', 'tools', 'systemPrompt', 'skills', 'llm',
        ...(workflow ? ['workflowEngine'] : []),
        ...(subagents ? ['subagents'] : []),
        ...(jobs ? ['jobs'] : []),
        'storage', 'storageDomain', ...storage.backends.map(name => 'storage.backend.' + name)]);
    const declared = Array.isArray(declaration)
        ? declaration.map(name => [name, null]) : Object.entries(declaration ?? {});
    if (declared.length > 64 || declared.some(([name]) =>
        typeof name !== 'string' || !/^[A-Za-z0-9_.-]{1,128}$/.test(name))) {
        throw new Error('invalid Cordis service declaration');
    }
    const services = Array.from(new Map(declared), ([name, config]) => ({name,
        status: !available.has(name) ? 'unavailable'
            : config != null ? 'unsupported_intercept' : 'available',
    }));
    report(JSON.stringify(services));
    const blocked = services.filter(({status}) => status !== 'available');
    if (blocked.length) throw new Error(`Cordis service dependencies: ${blocked.map(({name, status}) =>
        status === 'unavailable' ? `${name} is unavailable` : `${name} uses unsupported intercept configuration`
    ).join(', ')}`);
    const inject = services.map(({name}) => name);

    const registered = new Map();
    const register = definition => {
        const { name, description, input, handler } = definition;
        if (typeof name !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/.test(name)
            || typeof description !== 'string' || typeof handler !== 'function') {
            throw new Error('invalid Cordis command declaration');
        }
        if (input?.attachments !== undefined && typeof input.attachments !== 'boolean') {
            throw new Error('Cordis command attachments flag must be a boolean');
        }
        if (registered.size >= 256 || registered.has(name)) throw new Error('too many or duplicate Cordis commands');
        const command = { name, description, hint: input?.hint, attachments: input?.attachments === true, handler };
        registered.set(name, command);
        return () => registered.get(name) === command && registered.delete(name);
    };
    let applying = true;
    const tools = new Map();
    const registerTool = (definition, options) => {
        const { name, description, parameters, execute, output, timeoutMs } = definition;
        if (!applying || typeof name !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/.test(name)
            || name === 'run_code' || typeof description !== 'string' || !description.trim()
            || description.length > 8192 || typeof execute !== 'function'
            || parameters?.type !== 'object' || typeof output?.render !== 'function'
            || (!output.schema || typeof output.schema !== 'object')) {
            throw new Error('invalid Cordis tool declaration');
        }
        if (timeoutMs !== undefined && !(Number.isFinite(timeoutMs) && timeoutMs > 0)) {
            throw new Error('invalid Cordis tool timeout');
        }
        if (options?.when !== undefined && typeof options.when !== 'function') {
            throw new TypeError('Cordis tool options.when must be a function');
        }
        if (definition.deferLoading !== undefined && typeof definition.deferLoading !== 'boolean') {
            throw new TypeError('Cordis tool deferLoading must be a boolean');
        }
        if (tools.size >= 128 || tools.has(name)) throw new Error('too many or duplicate Cordis tools');
        const tool = {
            definition, name, description, parameters, execute,
            render: output.render, schema: output.schema, timeoutMs,
            when: options?.when ?? null,
            deferLoading: definition.deferLoading === true,
        };
        tools.set(name, tool);
        return () => tools.get(name) === tool && tools.delete(name);
    };
    const servers = new Map();
    const registerServer = (name, entry) => {
        if (!applying) throw new Error('MCP servers can be registered during apply');
        if (typeof name !== 'string' || !/^[A-Za-z0-9_-]{1,32}$/.test(name)
            || !entry || typeof entry !== 'object' || Array.isArray(entry)) {
            throw new Error('invalid Cordis MCP server declaration');
        }
        if (servers.has(name)) throw new Error('duplicate Cordis MCP server declaration');
        const server = { ...entry, enabled: true };
        servers.set(name, server);
        return () => {
            if (!applying) throw new Error('MCP registrations follow the package lifetime');
            if (servers.get(name) === server) servers.delete(name);
        };
    };
    const context = {};
    if (inject.includes('commands')) context.commands = Object.freeze({ register });
    if (inject.includes('tools')) {
        context.tools = Object.freeze({
            register: registerTool,
            get: name => tools.get(name)?.definition,
            definitions: () => Array.from(tools.values(), tool => tool.definition),
            schemas: () => Array.from(tools.values(), ({name, description, parameters}) => ({name, description, parameters})),
        });
    }
    if (inject.includes('systemPrompt')) context.systemPrompt = prompt.service;
    if (inject.includes('skills')) context.skills = skills.service;
    if (inject.includes('storage')) context.storage = storage.service;
    if (inject.includes('storageDomain')) context.storageDomain = storage.domain;
    for (const name of storage.backends) {
        const service = 'storage.backend.' + name;
        if (inject.includes(service)) context[service] = storage.service.backend.get(name);
    }
    if (inject.includes('workflowEngine')) context.workflowEngine = workflow.service;
    if (inject.includes('subagents')) context.subagents = subagents.service;
    if (inject.includes('jobs')) context.jobs = jobs.service;
    if (inject.includes('mcpManager')) {
        context.mcpManager = manager.service(registerServer);
    }
    if (inject.includes('llm')) {
        const extractMessageText = content => {
            if (typeof content === 'string') return content;
            if (Array.isArray(content)) {
                return content
                    .filter(block => block?.type === 'text' && typeof block.text === 'string')
                    .map(block => block.text)
                    .join('\n');
            }
            throw new TypeError('Cordis llm message content must be a string or text blocks');
        };
        const callLlm = async (requestOrMessages, extraOptions) => {
            let input;
            if (typeof requestOrMessages === 'string') {
                input = { ...(extraOptions ?? {}), messages: [{ role: 'user', content: requestOrMessages }] };
            } else if (Array.isArray(requestOrMessages)) {
                input = { ...(extraOptions ?? {}), messages: requestOrMessages };
            } else if (requestOrMessages && typeof requestOrMessages === 'object') {
                input = { ...(extraOptions ?? {}), ...requestOrMessages };
            } else {
                throw new TypeError('Cordis llm request must be an object, message array, or prompt string');
            }
            const rawMessages = Array.isArray(input.messages)
                ? input.messages
                : input.prompt !== undefined
                    ? [{ role: 'user', content: input.prompt }]
                    : null;
            if (!rawMessages || !rawMessages.length) {
                throw new TypeError('Cordis llm request requires non-empty messages or prompt');
            }
            const messages = rawMessages.map(msg => {
                if (!msg || typeof msg !== 'object') throw new TypeError('Cordis llm message must be an object');
                const role = typeof msg.role === 'string' ? msg.role : 'user';
                const text = extractMessageText(msg.content ?? msg.text);
                return { role, text };
            });
            const payload = {
                messages,
                ...(input.system !== undefined && { system: String(input.system) }),
                ...(input.provider !== undefined && { provider: String(input.provider) }),
                ...(input.model !== undefined && { model: String(input.model) }),
                ...(input.maxTokens !== undefined && { maxTokens: input.maxTokens }),
                ...(input.max_tokens !== undefined && input.maxTokens === undefined && { maxTokens: input.max_tokens }),
                ...(input.temperature !== undefined && { temperature: input.temperature }),
                ...(Array.isArray(input.stopSequences) && { stopSequences: input.stopSequences.map(String) }),
                ...(Array.isArray(input.stop) && !input.stopSequences && { stopSequences: input.stop.map(String) }),
            };
            const signal = input.signal === undefined
                ? manager.signal
                : AbortSignal.any([manager.signal, input.signal]);
            const reply = await manager.host('llm', payload, signal);
            return Object.freeze({
                text: reply.text,
                content: Object.freeze([Object.freeze({ type: 'text', text: reply.text })]),
                stopReason: reply.stopReason,
                provider: reply.provider,
                model: reply.model,
                ...(reply.usage && { usage: Object.freeze(reply.usage) }),
            });
        };
        context.llm = Object.freeze({
            complete: callLlm,
            generate: callLlm,
            chat: callLlm,
        });
    }
    context.on = manager.on;
    context.once = manager.once;
    context.events = Object.freeze({ on: context.on, once: context.once });
    context.get = name => inject.includes(name) ? context[name] : undefined;
    context.root = context;
    Object.freeze(context);
    if (typeof plugin !== 'function') {
        await plugin.apply(context, config);
    } else if (plugin.prototype && typeof plugin.prototype.next !== 'function') {
        await new plugin(context, config);
    } else {
        await plugin(context, config);
    }
    applying = false;

    return {
        catalog: {
            commands: Array.from(registered.values(), ({ name, description, hint, attachments }) =>
                ({ name, description, hint, attachments })),
            servers: Array.from(servers),
            uses_mcp_manager: context.mcpManager !== undefined,
            uses_system_prompt: context.systemPrompt !== undefined,
            observes_session_events: manager.observesSessionEvents(),
            services: inject,
            tools: Array.from(tools.values(), ({name, description, parameters, schema, timeoutMs, when, deferLoading}) =>
                ({name, description, parameters, schema, timeoutMs, hasWhen: when !== null, deferLoading})),
        },
        assemblePrompt: () => prompt.assemble(storage.prepare),
        refreshStorage: storage.refresh,
        storageStartup: storage.startup,
        skillSnapshot: options => skills.snapshot({...options, signal: new AbortController().signal}),
        subagentSettled: settlement => subagents?.settle(settlement),
        async evaluateToolWhen(request) {
            const tool = tools.get(request.name);
            if (!tool) return false;
            if (!tool.when) return true;
            manager.activate();
            try {
                await storage.prepare();
                const whenCtx = Object.freeze({
                    sessionId: String(request.sessionId),
                    toolName: tool.name,
                    deferLoading: tool.deferLoading,
                });
                return Boolean(await tool.when(whenCtx));
            } catch {
                return false;
            } finally {
                manager.deactivate();
            }
        },
        async invokeTool(invocation, scope) {
            const tool = tools.get(invocation.name);
            if (!tool) throw new Error('Cordis tool was not registered');
            const freeze = value => {
                if (value !== null && typeof value === 'object') {
                    for (const child of Object.values(value)) freeze(child);
                    Object.freeze(value);
                }
                return value;
            };
            const args = freeze(invocation.args);
            manager.activate();
            try {
                await storage.prepare();
                if (tool.when) {
                    const whenCtx = Object.freeze({
                        sessionId: String(invocation.sessionId),
                        toolName: tool.name,
                        deferLoading: tool.deferLoading,
                    });
                    if (!(await tool.when(whenCtx))) {
                        throw new Error(`Cordis tool ${tool.name} is not active in the current step`);
                    }
                }
                const agent = context.workflowEngine ? workflow.enter(invocation.sessionId, scope?.status)
                    : context.jobs || context.subagents ? Object.freeze({
                        id: String(invocation.sessionId), status: scope?.status,
                    }) : undefined;
                if (context.jobs) jobs.enter(agent.id, invocation.id);
                if (context.subagents) subagents.enter(agent, scope);
                const result = await tool.execute(args, Object.freeze({
                    name: invocation.name, callId: invocation.id, rootCallId: invocation.id,
                    arguments: args, token: Symbol(), signal: manager.signal, agent,
                }));
                await subagents?.flush();
                const value = freeze(JSON.parse(JSON.stringify(result, (_, item) => {
                    if (item === undefined || typeof item === 'symbol' || typeof item === 'function'
                        || (typeof item === 'number' && !Number.isFinite(item))) {
                        throw new Error('Cordis tool output must be a JSON value');
                    }
                    return item;
                })));
                const blocks = tool.render(args, value);
                if (!Array.isArray(blocks) || blocks.length > 128) {
                    throw new Error('Cordis tools must render at most 128 content blocks');
                }
                const texts = [];
                const media = [];
                const parseDataUrl = raw => {
                    if (typeof raw !== 'string' || !raw.startsWith('data:')) return null;
                    const comma = raw.indexOf(',');
                    if (comma < 0) return null;
                    const header = raw.slice(5, comma);
                    if (!header.endsWith(';base64')) return null;
                    const mediaType = header.slice(0, -';base64'.length).trim();
                    const data = raw.slice(comma + 1);
                    return mediaType && data ? { mediaType, data } : null;
                };
                for (const block of blocks) {
                    if (block?.type === 'text' && typeof block.text === 'string') {
                        texts.push(block.text);
                        continue;
                    }
                    if (block?.type === 'image' || block?.type === 'audio'
                        || block?.type === 'document' || block?.type === 'file') {
                        let mediaType = block.mediaType ?? block.mimeType ?? block.media_type;
                        let data = block.data;
                        const urlCandidate = typeof block.url === 'string'
                            ? block.url
                            : typeof block.image_url === 'string'
                                ? block.image_url
                                : typeof block.image_url?.url === 'string'
                                    ? block.image_url.url
                                    : null;
                        if (urlCandidate) {
                            const parsed = parseDataUrl(urlCandidate);
                            if (!parsed) throw new Error('Cordis tool media URL must be a base64 data URL');
                            mediaType = mediaType ?? parsed.mediaType;
                            data = parsed.data;
                        } else if (block.source && typeof block.source === 'object') {
                            if (block.source.type !== 'base64') {
                                throw new Error('Cordis tool media source must use base64 encoding');
                            }
                            mediaType = block.source.mediaType ?? block.source.media_type ?? mediaType;
                            data = block.source.data;
                        }
                        if (ArrayBuffer.isView(data)) {
                            data = Buffer.from(data).toString('base64');
                        }
                        if (typeof mediaType !== 'string' || !mediaType.trim()
                            || typeof data !== 'string' || !data.trim()) {
                            throw new Error('Cordis tool media blocks require mediaType and base64 data');
                        }
                        if (block.filename !== undefined && typeof block.filename !== 'string') {
                            throw new Error('Cordis tool media filename must be a string');
                        }
                        if (media.length >= 8) {
                            throw new Error('Cordis tool output contains more than eight media blocks');
                        }
                        media.push({
                            kind: block.type,
                            mediaType: mediaType.trim(),
                            data,
                            ...(block.filename ? { filename: block.filename } : {}),
                        });
                        continue;
                    }
                    throw new Error('Cordis tools must render text or supported media content');
                }
                return { value, text: texts.join('\n'), media };
            } finally {
                jobs?.leave();
                workflow?.leave();
                subagents?.leave();
                manager.deactivate();
            }
        },
        async invoke(invocation, scope) {
    const command = registered.get(invocation.name);
    if (!command) throw new Error('Cordis command was not registered');
    if (invocation.attachments !== null && !command.attachments) {
        throw new Error('Cordis command does not accept attachments');
    }
    const attachments = Object.freeze((invocation.attachments ?? []).map(block => {
        Object.freeze(block.attachment);
        return Object.freeze(block);
    }));
    let content;
    const followup = message => {
        if (content !== undefined || message?.role !== 'user' || message?.source?.kind !== 'user'
            || !Array.isArray(message.content) || message.content.length > 256) {
            throw new Error('Cordis commands can submit one user followup with at most 256 content blocks');
        }
        let forwarded = 0;
        content = message.content.map(block => {
            if (block?.type === 'text' && typeof block.text === 'string') {
                return { type: 'text', text: block.text };
            }
            const ref = block?.attachment;
            const index = ref ? attachments.findIndex(candidate => {
                const expected = candidate.attachment;
                return candidate.type === block.type && expected.attachmentId === ref.attachmentId
                    && expected.name === ref.name && expected.bytes === ref.bytes
                    && expected.mediaType === ref.mediaType && expected.width === ref.width
                    && expected.height === ref.height;
            }) : -1;
            if (index < 0) throw new Error('Cordis followup attachment was not supplied to this invocation');
            if (++forwarded > 8) throw new Error('Cordis followup has more than eight attachments');
            return { type: 'attachment', index };
        });
    };
    manager.activate();
    try {
        await storage.prepare();
        const owner = invocation.sessionId == null ? undefined : String(invocation.sessionId);
        const agent = Object.freeze({ followup, id: owner, status: scope?.status });
        if (context.jobs && owner !== undefined) jobs.enter(owner);
        if (context.subagents && owner !== undefined) subagents.enter(agent, scope);
        const result = await command.handler(Object.freeze({
            commandId: invocation.id,
            rawInput: invocation.args,
            attachments,
            agent,
            signal: manager.signal,
        }));
        await subagents?.flush();
        return { kind: result?.kind, text: result?.text, followup: content };
    } finally {
        jobs?.leave();
        subagents?.leave();
        manager.deactivate();
    }
        },
    };
})
"#;

#[derive(Deserialize)]
pub(super) struct Definition {
    pub name: String,
    pub description: String,
    pub hint: Option<String>,
    attachments: bool,
}

#[derive(Deserialize)]
struct ModuleCatalog {
    commands: Vec<Definition>,
    servers: Vec<(String, Yaml)>,
    uses_mcp_manager: bool,
    uses_system_prompt: bool,
    observes_session_events: bool,
    services: Vec<String>,
    tools: Vec<tools::Registration>,
}

#[derive(Default)]
pub(super) struct Contributions {
    pub commands: Vec<super::markdown::ImportedCommand>,
    pub servers: Vec<(String, Vec<(String, Yaml)>)>,
    pub programs: Vec<Arc<Program>>,
    pub dependencies: Vec<CordisDependency>,
    pub skill_count: usize,
}

#[derive(Clone)]
pub(super) struct Command {
    program: Arc<Program>,
    name: String,
    attachments: bool,
}

pub(super) struct Program {
    root: PathBuf,
    source: Vec<u8>,
    url: Url,
    config: Vec<u8>,
    uses_mcp_manager: bool,
    uses_system_prompt: bool,
    uses_storage: bool,
    uses_workflow: bool,
    uses_jobs: bool,
    uses_skills: bool,
    uses_subagents: bool,
    uses_llm: bool,
    storage: storage::Config,
    workflow_config: Option<workflow::CordisConfig>,
    jobs_config: Option<Arc<workflow::CordisJobsConfig>>,
    job_controllers: Arc<controllers::JobControllers>,
    session_events: events::SessionEvents,
    entry_index: usize,
    observes_session_events: bool,
    tools: Vec<tools::Registration>,
    sessions: retained::Sessions,
}

#[derive(Deserialize)]
struct Reply {
    kind: ReplyKind,
    text: Option<String>,
    followup: Option<Vec<CordisFollowupBlock>>,
}

/// Followup content produced by the Cordis adapter. Attachment indexes
/// refer to the invocation's admitted attachments, in submission order.
#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CordisFollowupBlock {
    Text { text: String },
    Attachment { index: usize },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplyKind {
    Success,
    Error,
}

fn package_export<'a>(package: &str, name: &'a str) -> Option<&'a str> {
    if package.is_empty() {
        return None;
    }
    let suffix = name.strip_prefix(package)?;
    if suffix.is_empty() {
        Some("")
    } else {
        suffix.strip_prefix('/')
    }
}

pub(super) fn is_package_module(manifest: &Map<String, Value>, name: &str) -> bool {
    name.starts_with("file:")
        || package_export(packages::name(manifest), name).is_some()
        || packages::dependency(manifest, name, false).is_some()
}

pub(super) fn discover(
    root: &Path,
    manifest: &Map<String, Value>,
    rows: &[(String, Yaml)],
    watch: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
) -> Contributions {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut contributions = Contributions::default();
    let session_events = events::SessionEvents::default();
    let job_controllers = Arc::new(controllers::JobControllers::default());
    let mut server_count = 0;
    let mut tool_count = 0;
    let mut skill_names = BTreeSet::new();
    let storage = storage::Config::discover(rows, diagnostics);
    let workflow_config = match workflow::CordisConfig::discover(rows) {
        Ok(config) => Some(config),
        Err(error) => {
            diagnostics.push(format!("DeepSeek Harness workflow: {error}"));
            None
        }
    };
    let jobs_config = match workflow::CordisJobsConfig::discover(rows) {
        Ok(config) => Some(Arc::new(config)),
        Err(error) => {
            diagnostics.push(format!("DeepSeek Harness jobs: {error}"));
            None
        }
    };
    let has_controller = rows.iter().any(|(_, row)| deepseek::module_name(row) == Some(JOBS_TOOL));
    let mut needs_controller = rows.iter().any(|(_, row)| match deepseek::module_name(row) {
        Some(WORKFLOW_TOOL) => row.get("config").and_then(|config| config.get("enableRunInBackground"))
            .and_then(Yaml::as_bool) != Some(false),
        Some(JOBS_LOCAL) => true,
        _ => false,
    });
    let implicit_controller = (!has_controller && jobs_config.is_some()).then(|| {
        let row = Yaml::Mapping(serde_yaml::Mapping::from_iter([(
            Yaml::String("name".into()), Yaml::String(JOBS_TOOL.into()),
        )]));
        (JOBS_TOOL.to_owned(), row)
    });
    for (entry_index, (id, row)) in rows.iter().chain(implicit_controller.iter()).enumerate() {
        if entry_index == rows.len() && (!needs_controller || job_controllers.has_modules()) {
            break;
        }
        let Some(name) = deepseek::module_name(row).filter(|name| {
            matches!(*name, WORKFLOW_TOOL | JOBS_TOOL | SUBAGENT_TOOL | SUBAGENT_CONTROL_TOOL)
                || (!deepseek::is_native_module(name) && is_package_module(manifest, name))
        }) else {
            continue;
        };
        if Instant::now() >= deadline {
            diagnostics.push("DeepSeek Harness module discovery timed out".into());
            break;
        }
        let loaded = (|| {
            let native_tool = match name {
                WORKFLOW_TOOL => Some(("tool-workflow", WORKFLOW_TOOL_MODULE)),
                JOBS_TOOL => Some(("tool-jobs", JOBS_TOOL_MODULE)),
                SUBAGENT_TOOL => Some(("tool-subagent", SUBAGENT_TOOL_MODULE)),
                SUBAGENT_CONTROL_TOOL => Some(("tool-subagent-control", SUBAGENT_CONTROL_MODULE)),
                _ => None,
            };
            let (source, url, source_name) = if let Some((module, code)) = native_tool {
                (
                    code.as_bytes().to_vec(),
                    Url::parse(&format!("agena-cordis:{module}")).expect("valid Cordis module URL"),
                    name.to_owned(),
                )
            } else {
                let path = if name.starts_with("file:") {
                    resolve_module(root, manifest, packages::name(manifest), name)?
                } else {
                    packages::resolve(root, root, manifest, name, false, watch)?
                };
                watch.insert(path.clone());
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "module is outside its package")?;
                let source = read_package_bytes(root, relative, MAX_SCRIPT_BYTES)?
                    .ok_or("Cordis module is missing")?;
                (
                    source,
                    Url::from_file_path(&path).map_err(|()| "invalid Cordis module path")?,
                    relative.to_string_lossy().into_owned(),
                )
            };
            let mut config = row.get("config").filter(|value| !value.is_null())
                .map(serde_json::to_value).transpose().map_err(|error| error.to_string())?
                .unwrap_or_else(|| json!({}));
            if name == SUBAGENT_TOOL && id == "tool-subagent-fork"
                && let Some(config) = config.as_object_mut()
            {
                config.entry("provider").or_insert_with(|| json!("fork"));
                config.entry("toolName").or_insert_with(|| json!("subagent_fork"));
            }
            let mut program = Program {
                root: root.to_path_buf(),
                source,
                url,
                config: serde_json::to_vec(&config).map_err(|error| error.to_string())?,
                uses_mcp_manager: false,
                uses_system_prompt: false,
                uses_storage: false,
                uses_workflow: false,
                uses_jobs: false,
                uses_skills: false,
                uses_subagents: false,
                uses_llm: false,
                storage: storage.clone(),
                workflow_config: workflow_config.clone(),
                jobs_config: jobs_config.clone(),
                job_controllers: job_controllers.clone(),
                session_events: session_events.clone(),
                entry_index,
                observes_session_events: false,
                tools: Vec::new(),
                sessions: Default::default(),
            };
            let mut services = Vec::new();
            let vm = program.load(
                &RuntimeSnapshot::default(),
                deadline,
                None,
                Some(ModuleDiscovery {
                    watch,
                    services: &mut services,
                }),
            );
            contributions
                .dependencies
                .extend(services.into_iter().map(|service| CordisDependency {
                    entry: id.clone(),
                    module: source_name.clone(),
                    service,
                }));
            let vm = vm?;
            let catalog = vm.catalog()?;
            program.uses_skills = catalog.services.iter().any(|service| service == "skills");
            if program.uses_skills {
                let observed = vm.skill_snapshot(root, root, deadline, None, None);
                watch.extend(vm.package_files.lock().unwrap_or_else(|error| error.into_inner()).iter().cloned());
                let skill_diagnostics = match observed {
                    Ok(observed) => {
                        skill_names.extend(observed.skills.into_iter().map(|skill| skill.name));
                        observed.diagnostics
                    }
                    Err(error) => vec![error],
                };
                diagnostics.extend(skill_diagnostics.into_iter().map(|message| {
                    format!("DeepSeek Harness skill provider {name:?} (entry {id:?}): {message}")
                }));
            }
            if contributions.commands.len() + catalog.commands.len() > MAX_PACKAGE_COMMANDS {
                return Err("DeepSeek Harness bundle exceeds 256 commands".into());
            }
            if server_count + catalog.servers.len() > super::MAX_COMPONENT_ENTRIES {
                return Err("DeepSeek Harness bundle has too many MCP servers".into());
            }
            if tool_count + catalog.tools.len() > MAX_PACKAGE_TOOLS {
                return Err("DeepSeek Harness bundle exceeds 128 tools".into());
            }
            if !catalog.servers.is_empty() {
                server_count += catalog.servers.len();
                contributions.servers.push((id.clone(), catalog.servers));
            }
            tool_count += catalog.tools.len();
            program.uses_mcp_manager = catalog.uses_mcp_manager;
            program.uses_system_prompt = catalog.uses_system_prompt;
            program.observes_session_events = catalog.observes_session_events;
            program.uses_storage = catalog.services.iter().any(|service| {
                matches!(service.as_str(), "storage" | "storageDomain")
                    || service.starts_with("storage.backend.")
            });
            program.uses_workflow = catalog
                .services
                .iter()
                .any(|service| service == "workflowEngine");
            program.uses_jobs = catalog.services.iter().any(|service| service == "jobs");
            program.uses_subagents = catalog.services.iter().any(|service| service == "subagents");
            program.uses_llm = catalog.services.iter().any(|service| service == "llm");
            needs_controller |= program.uses_jobs;
            program.tools = catalog.tools;
            if matches!(name, SUBAGENT_TOOL | SUBAGENT_CONTROL_TOOL) {
                for tool in &mut program.tools {
                    tool.agent_coordination = true;
                }
            }
            let program = Arc::new(program);
            if name == JOBS_TOOL {
                job_controllers.register_native();
            } else if program.uses_jobs && vm.jobs.has_controller() {
                job_controllers.register_module(&program);
            }
            if program.keeps_session() {
                contributions.programs.push(program.clone());
            }
            contributions
                .commands
                .extend(catalog.commands.into_iter().map(|definition| {
                    let command = Command {
                        program: program.clone(),
                        name: definition.name.clone(),
                        attachments: definition.attachments,
                    };
                    super::markdown::ImportedCommand::deepseek(
                        source_name.clone(),
                        definition,
                        command,
                    )
                }));
            Ok::<_, String>(())
        })();
        if let Err(error) = loaded {
            diagnostics.push(format!(
                "DeepSeek Harness module {name:?} (entry {id:?}): {error}"
            ));
        }
    }
    contributions.skill_count = skill_names.len();
    contributions
}

fn resolve_module(
    root: &Path,
    manifest: &Map<String, Value>,
    package: &str,
    name: &str,
) -> Result<PathBuf, String> {
    let path = if name.starts_with("file:") {
        let path = Url::parse(name)
            .ok()
            .and_then(|url| url.to_file_path().ok())
            .ok_or("invalid local Cordis module URL")?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "Cordis module is outside its package")?;
        super::package_path(root, relative)?
    } else {
        let suffix =
            package_export(package, name).ok_or("Cordis module belongs to another package")?;
        let target = match manifest.get("exports") {
            Some(exports) => {
                let key = if suffix.is_empty() {
                    ".".to_owned()
                } else {
                    format!("./{suffix}")
                };
                let entry = exports.get(&key).or_else(|| {
                    (suffix.is_empty()
                        && !exports
                            .as_object()
                            .is_some_and(|object| object.keys().any(|key| key.starts_with('.'))))
                    .then_some(exports)
                });
                entry
                    .and_then(export_target)
                    .ok_or("no import export for the Cordis module")?
            }
            None if suffix.is_empty() => manifest
                .get("main")
                .and_then(Value::as_str)
                .unwrap_or("./index.js"),
            None => return super::package_path(root, Path::new(suffix)).and_then(check_extension),
        };
        if manifest.contains_key("exports") {
            manifest_path(root, target)?
        } else {
            super::package_path(root, Path::new(target))?
        }
    };
    check_extension(path)
}

fn check_extension(path: PathBuf) -> Result<PathBuf, String> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "js" | "mjs"))
    {
        return Err("Cordis command modules require a local .js or .mjs ES module".into());
    }
    Ok(path)
}

fn export_target(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| {
        if let Some(values) = value.as_array() {
            return values.iter().find_map(export_target);
        }
        ["import", "node", "default"]
            .iter()
            .find_map(|key| export_target(value.get(key)?))
    })
}

struct CordisModuleResolver {
    root: PathBuf,
    manifests: Arc<Mutex<BTreeSet<PathBuf>>>,
}

impl Resolver for CordisModuleResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if attributes.is_none() && builtin_module(name).is_some() {
            return Ok(name.to_owned());
        }
        if attributes.is_some() || name.contains(['?', '#', '\\']) {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "invalid Cordis import",
            ));
        }
        let resolve = || -> Result<Url, String> {
            let base = Url::parse(base).map_err(|error| error.to_string())?;
            if name.starts_with("./") || name.starts_with("../") {
                return base.join(name).map_err(|error| error.to_string());
            }
            let importer = base.to_file_path().map_err(|()| "invalid Cordis importer")?;
            let mut manifests = self.manifests.lock().unwrap_or_else(|error| error.into_inner());
            let path = packages::resolve_import(&self.root, &importer, name, &mut manifests)?;
            Url::from_file_path(path).map_err(|()| "invalid Cordis dependency path".into())
        };
        resolve()
            .map(Into::into)
            .map_err(|error| rquickjs::Error::new_resolving_message(base, name, error))
    }
}

struct PackageModules {
    root: PathBuf,
    paths: Rc<RefCell<BTreeSet<PathBuf>>>,
}

impl Loader for PackageModules {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        if let Some(source) = builtin_module(name) {
            return Module::declare(ctx.clone(), name, source);
        }
        let load = || -> Result<Vec<u8>, String> {
            let path = check_extension(
                Url::parse(name)
                    .ok()
                    .and_then(|url| url.to_file_path().ok())
                    .ok_or("invalid Cordis import URL")?,
            )?;
            let relative = path
                .strip_prefix(&self.root)
                .map_err(|_| "Cordis import is outside its package")?;
            {
                let mut paths = self.paths.borrow_mut();
                if paths.len() >= MAX_MODULE_IMPORTS {
                    return Err("too many Cordis module imports".into());
                }
                paths.insert(path.clone());
            }
            read_package_bytes(&self.root, relative, MAX_SCRIPT_BYTES)?
                .ok_or_else(|| "Cordis import does not exist".into())
        };
        let source = load().map_err(|error| rquickjs::Error::new_loading_message(name, error))?;
        let module = Module::declare(ctx.clone(), name, source)?;
        module.meta()?.set("url", name)?;
        Ok(module)
    }
}

const MODULE_GLOBAL: &str = "__agenaCordisModule";
const MANAGER_GLOBAL: &str = "__agenaCordisManager";

struct ScriptBudget {
    deadline: Instant,
    cancellation: Option<CancellationToken>,
}

struct Vm {
    context: Context,
    budget: Arc<Mutex<Option<ScriptBudget>>>,
    package_files: Arc<Mutex<BTreeSet<PathBuf>>>,
    jobs: Arc<jobs::VmJobs>,
    keeps_jobs: bool,
    keeps_subagents: bool,
}

struct ModuleDiscovery<'a> {
    watch: &'a mut BTreeSet<PathBuf>,
    services: &'a mut Vec<CordisService>,
}

#[derive(Default)]
struct RuntimeSnapshot {
    mcp: Value,
    storage: Value,
    subagent_scope: Value,
}

impl Program {
    pub(in crate::agent_plugin) fn tools(&self) -> &[tools::Registration] {
        &self.tools
    }

    fn keeps_session(&self) -> bool {
        self.uses_mcp_manager
            || self.uses_system_prompt
            || self.uses_storage
            || self.uses_skills
            || self.uses_jobs
            || self.uses_subagents
            || self.uses_llm
            || self.observes_session_events
            || !self.tools.is_empty()
    }

    pub(in crate::agent_plugin) fn uses_system_prompt(&self) -> bool {
        self.uses_system_prompt
    }

    pub(super) async fn provided_skills(
        self: &Arc<Self>,
        cwd: &Path,
        deadline: Instant,
        session: Option<i64>,
        host: &dyn HostClient,
    ) -> agena_plugin_sdk::Result<skills::Snapshot> {
        if !self.uses_skills {
            return Ok(skills::Snapshot {
                skills: Vec::new(),
                diagnostics: Vec::new(),
            });
        }
        if let Some(session) = session {
            return self
                .start_session(session, host)
                .await?
                .provided_skills(self, cwd, deadline, host)
                .await;
        }

        let storage = self.storage_snapshot(Some(host)).await?;
        let program = Arc::clone(self);
        let cwd = cwd.to_owned();
        let cancel = CancellationToken::new();
        let _stop = cancel.clone().drop_guard();
        WORKERS
            .run(move || {
                let snapshot = RuntimeSnapshot {
                    storage,
                    ..Default::default()
                };
                let vm = program.load(&snapshot, deadline, Some(cancel.clone()), None)?;
                vm.skill_snapshot(&program.root, &cwd, deadline, Some(cancel), None)
            })
            .await
            .map_err(|error| PluginError::internal_error(&error))?
            .map_err(PluginError::internal)
    }

    async fn storage_snapshot(
        &self,
        host: Option<&dyn HostClient>,
    ) -> agena_plugin_sdk::Result<Value> {
        if !self.uses_storage {
            return Ok(Value::Null);
        }
        let host =
            host.ok_or_else(|| PluginError::internal("Cordis storage host is unavailable"))?;
        Ok(self.storage.snapshot(host).await)
    }

    fn load(
        &self,
        initial: &RuntimeSnapshot,
        deadline: Instant,
        cancellation: Option<CancellationToken>,
        discovery: Option<ModuleDiscovery<'_>>,
    ) -> Result<Vm, String> {
        let budget = Arc::new(Mutex::new(None::<ScriptBudget>));
        let interrupt = Arc::clone(&budget);
        let runtime = deepseek_js::create_runtime(move || {
            interrupt
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .as_ref()
                .is_some_and(|active| {
                    Instant::now() >= active.deadline
                        || active
                            .cancellation
                            .as_ref()
                            .is_some_and(CancellationToken::is_cancelled)
                })
        })?;
        let paths = Rc::new(RefCell::new(BTreeSet::new()));
        let manifests = Arc::new(Mutex::new(BTreeSet::new()));
        runtime.set_loader(
            CordisModuleResolver { root: self.root.clone(), manifests: manifests.clone() },
            PackageModules {
                root: self.root.clone(),
                paths: paths.clone(),
            },
        );
        let vm = Vm {
            context: Context::full(&runtime).map_err(|error| error.to_string())?,
            jobs: jobs::VmJobs::new(budget.clone(), self.job_controllers.clone()),
            budget,
            package_files: manifests.clone(),
            keeps_jobs: self.uses_jobs,
            keeps_subagents: self.uses_subagents,
        };
        let services = Rc::new(RefCell::new(Vec::new()));
        let result = vm.with_budget(deadline, cancellation, || {
            vm.context.with(|ctx| {
                let error = |error: rquickjs::CaughtError<'_>| error.to_string();
                deepseek_js::install_module_url(&ctx)?;
                ctx.eval::<(), _>(NODE_BUILTINS).catch(&ctx).map_err(error)?;
                let root = self.root.clone();
                let files = manifests.clone();
                ctx.globals().set(
                    "agenaCordisReadPackageText",
                    Func::from(move |input: String| {
                        let mut files = files.lock().unwrap_or_else(|error| error.into_inner());
                        packages::read_text(&root, &input, &mut files)
                    }),
                ).catch(&ctx).map_err(error)?;
                let root = self.root.clone();
                let files = manifests.clone();
                ctx.globals().set(
                    "agenaCordisReadPackageBase64",
                    Func::from(move |input: String| {
                        let mut files = files.lock().unwrap_or_else(|error| error.into_inner());
                        packages::read_base64(&root, &input, &mut files)
                    }),
                ).catch(&ctx).map_err(error)?;
                ctx.globals()
                    .set(
                        "agenaCordisRandomUuid",
                        Func::from(|| uuid::Uuid::new_v4().to_string()),
                    )
                    .catch(&ctx)
                    .map_err(error)?;
                ctx.globals()
                    .set(
                        "agenaCordisCryptoRandomUuid",
                        Func::from(|| uuid::Uuid::new_v4().to_string()),
                    )
                    .catch(&ctx)
                    .map_err(error)?;
                ctx.globals()
                    .set(
                        "agenaCordisCryptoRandomBytes",
                        Func::from(|size: usize| -> rquickjs::Result<String> {
                            use base64::Engine as _;
                            if size > 65_536 {
                                return Err(rquickjs::Error::new_from_js_message(
                                    "randomBytes",
                                    "size",
                                    "size exceeds 65536 bytes",
                                ));
                            }
                            let mut bytes = Vec::with_capacity(size);
                            while bytes.len() < size {
                                let chunk = *uuid::Uuid::new_v4().as_bytes();
                                let remaining = size - bytes.len();
                                bytes.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
                            }
                            Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
                        }),
                    )
                    .catch(&ctx)
                    .map_err(error)?;
                ctx.globals()
                    .set(
                        "agenaCordisCryptoDigest",
                        Func::from(|algorithm: String, input_base64: String| -> rquickjs::Result<String> {
                            use base64::Engine as _;
                            use sha2::{Digest, Sha256, Sha384, Sha512};
                            let bytes = base64::engine::general_purpose::STANDARD
                                .decode(input_base64.as_bytes())
                                .map_err(|_| {
                                    rquickjs::Error::new_from_js_message(
                                        "crypto",
                                        "base64",
                                        "invalid hash input",
                                    )
                                })?;
                            let digest = match algorithm.as_str() {
                                "sha256" => Sha256::digest(&bytes).to_vec(),
                                "sha384" => Sha384::digest(&bytes).to_vec(),
                                "sha512" => Sha512::digest(&bytes).to_vec(),
                                _ => {
                                    return Err(rquickjs::Error::new_from_js_message(
                                        "crypto",
                                        "algorithm",
                                        "unsupported hash algorithm",
                                    ));
                                }
                            };
                            Ok(base64::engine::general_purpose::STANDARD.encode(digest))
                        }),
                    )
                    .catch(&ctx)
                    .map_err(error)?;
                let create: Function = ctx.eval(MODULE_SERVICES).catch(&ctx).map_err(error)?;
                let mcp_snapshot = ctx
                    .json_parse(initial.mcp.to_string())
                    .catch(&ctx)
                    .map_err(error)?;
                let new_manager: Function = ctx.eval(MCP_MANAGER).catch(&ctx).map_err(error)?;
                let job_runtime = vm.jobs.clone();
                let foreground_effects = Func::from(move || job_runtime.foreground_effects());
                let manager: Object = new_manager
                    .call((mcp_snapshot, foreground_effects))
                    .catch(&ctx)
                    .map_err(error)?;
                let jobs = self.jobs_config.as_ref().map(|config|
                    vm.jobs.install(&ctx, manager.clone(), config)
                ).transpose().catch(&ctx).map_err(error)?;
                let create_skills: Function = ctx.eval(skills::SERVICE).catch(&ctx).map_err(error)?;
                let skills: Object = create_skills.call((manager.clone(),)).catch(&ctx).map_err(error)?;
                let new_prompt: Function = ctx.eval(prompt::MODULE).catch(&ctx).map_err(error)?;
                let prompt: Object = new_prompt
                    .call((manager.clone(),))
                    .catch(&ctx)
                    .map_err(error)?;
                let registries = Object::new(ctx.clone()).catch(&ctx).map_err(error)?;
                registries.set("prompt", prompt).catch(&ctx).map_err(error)?;
                registries.set("skills", skills).catch(&ctx).map_err(error)?;
                registries.set("jobs", jobs).catch(&ctx).map_err(error)?;
                if let Some(config) = &self.workflow_config {
                    let create: Function = ctx.eval(subagents::SERVICE).catch(&ctx).map_err(error)?;
                    let providers = ctx.json_parse(subagents::providers(config).to_string())
                        .catch(&ctx).map_err(error)?;
                    let service: Object = create.call((
                        manager.clone(), Func::from(|| uuid::Uuid::new_v4().to_string()),
                        providers, config.subagent_max_depth, config.subagent_max_active,
                    )).catch(&ctx).map_err(error)?;
                    registries.set("subagents", service).catch(&ctx).map_err(error)?;
                }
                let open: Function = ctx.eval(storage::MODULE).catch(&ctx).map_err(error)?;
                let snapshot = ctx
                    .json_parse(initial.storage.to_string())
                    .catch(&ctx)
                    .map_err(error)?;
                let storage_binding = ctx
                    .json_parse(self.storage.binding().to_string())
                    .catch(&ctx)
                    .map_err(error)?;
                let storage: Object = open
                    .call((manager.clone(), snapshot, storage_binding))
                    .catch(&ctx)
                    .map_err(error)?;
                let workflow = if let Some(config) = &self.workflow_config {
                    let create: Function = ctx.eval(WORKFLOW_ENGINE).catch(&ctx).map_err(error)?;
                    let id = Func::from(|| uuid::Uuid::new_v4().to_string());
                    let job_policy = self.jobs_config.as_deref().map(|jobs| json!({
                        "waitTimeoutMs": jobs.wait_timeout_ms,
                        "maxWaitTimeoutMs": jobs.max_wait_timeout_ms.min(600_000),
                    })).unwrap_or(Value::Null);
                    let job_policy = ctx.json_parse(job_policy.to_string()).catch(&ctx).map_err(error)?;
                    let providers = ctx
                        .json_parse(json!(config.provider_names()).to_string())
                        .catch(&ctx)
                        .map_err(error)?;
                    Some(
                        create
                            .call::<_, Object>((
                                manager.clone(),
                                id,
                                config.max_total_agents,
                                job_policy,
                                providers,
                            ))
                            .catch(&ctx)
                            .map_err(error)?,
                    )
                } else {
                    None
                };
                let module =
                    Module::declare(ctx.clone(), self.url.as_str(), self.source.as_slice())
                        .catch(&ctx)
                        .map_err(error)?;
                module
                    .meta()
                    .and_then(|meta| meta.set("url", self.url.as_str()))
                    .catch(&ctx)
                    .map_err(error)?;
                let (module, ready) = module.eval().catch(&ctx).map_err(error)?;
                ready.finish::<()>().catch(&ctx).map_err(error)?;
                let config = ctx
                    .json_parse(self.config.as_slice())
                    .catch(&ctx)
                    .map_err(error)?;
                let module = module.namespace().catch(&ctx).map_err(error)?;
                let observed = services.clone();
                let report = Func::from(move |wire: String| {
                    let dependencies = serde_json::from_str(&wire).map_err(|error| {
                        rquickjs::Error::new_from_js_message(
                            "Cordis dependencies",
                            "JSON",
                            error.to_string(),
                        )
                    })?;
                    *observed.borrow_mut() = dependencies;
                    Ok::<_, rquickjs::Error>(())
                });
                let ready: Promise = create
                    .call((
                        module,
                        config,
                        manager.clone(),
                        registries,
                        storage,
                        workflow,
                        report,
                    ))
                    .catch(&ctx)
                    .map_err(error)?;
                let module: Object = ready.finish().catch(&ctx).map_err(error)?;
                ctx.globals()
                    .set(MANAGER_GLOBAL, manager)
                    .catch(&ctx)
                    .map_err(error)?;
                ctx.globals()
                    .set(MODULE_GLOBAL, module)
                    .catch(&ctx)
                    .map_err(error)
            })
        });
        if let Some(discovery) = discovery {
            discovery
                .watch
                .extend(std::mem::take(&mut *paths.borrow_mut()));
            let mut manifests = manifests.lock().unwrap_or_else(|error| error.into_inner());
            discovery.watch.extend(std::mem::take(&mut *manifests));
            *discovery.services = std::mem::take(&mut *services.borrow_mut());
        }
        result?;
        Ok(vm)
    }
}

impl Vm {
    fn with_budget<T>(
        &self,
        deadline: Instant,
        cancellation: Option<CancellationToken>,
        run: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        *self
            .budget
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(ScriptBudget {
            deadline,
            cancellation,
        });
        let result = run();
        self.budget
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        result
    }

    fn catalog(&self) -> Result<ModuleCatalog, String> {
        self.with_budget(Instant::now() + SCRIPT_TIMEOUT, None, || {
            self.context.with(|ctx| {
                let error = |error: rquickjs::CaughtError<'_>| error.to_string();
                let module: Object = ctx
                    .globals()
                    .get(MODULE_GLOBAL)
                    .catch(&ctx)
                    .map_err(error)?;
                decode_js(&ctx, module.get("catalog").catch(&ctx).map_err(error)?)
            })
        })
    }

    fn skill_snapshot(
        &self,
        root: &Path,
        cwd: &Path,
        deadline: Instant,
        cancellation: Option<CancellationToken>,
        storage: Option<&Value>,
    ) -> Result<skills::Snapshot, String> {
        let mut observed = self.with_budget(deadline, cancellation, || {
            self.context.with(|ctx| {
                let error = |error: rquickjs::CaughtError<'_>| error.to_string();
                let module: Object = ctx.globals().get(MODULE_GLOBAL).catch(&ctx).map_err(error)?;
                if let Some(storage) = storage {
                    Self::refresh_storage(&ctx, &module, storage)?;
                }
                let snapshot: Function = module.get("skillSnapshot").catch(&ctx).map_err(error)?;
                let options = Object::new(ctx.clone()).catch(&ctx).map_err(error)?;
                options.set("cwd", cwd.to_string_lossy().as_ref()).catch(&ctx).map_err(error)?;
                let reply: Promise = snapshot.call((options,)).catch(&ctx).map_err(error)?;
                let value = reply.finish().catch(&ctx).map_err(error)?;
                decode_js::<skills::Snapshot>(&ctx, value)
            })
        })?;
        let mut watch = self.package_files.lock().unwrap_or_else(|error| error.into_inner());
        observed.skills.retain_mut(|skill| match skill.validate(root, &mut watch) {
            Ok(()) => true,
            Err(error) => {
                observed.diagnostics.push(format!("{}: {error}", skill.name));
                false
            }
        });
        Ok(observed)
    }

    fn storage_startup(&self) -> Result<Vec<storage::RecordMutation>, String> {
        self.with_budget(Instant::now() + SCRIPT_TIMEOUT, None, || {
            self.context.with(|ctx| {
                let error = |error: rquickjs::CaughtError<'_>| error.to_string();
                let module: Object = ctx
                    .globals()
                    .get(MODULE_GLOBAL)
                    .catch(&ctx)
                    .map_err(error)?;
                let startup: Function = module.get("storageStartup").catch(&ctx).map_err(error)?;
                decode_js(&ctx, startup.call(()).catch(&ctx).map_err(error)?)
            })
        })
    }

    fn run<T: serde::de::DeserializeOwned>(
        &self,
        method: &'static str,
        invocation: &[u8],
        initial: &RuntimeSnapshot,
        cancellation: CancellationToken,
        mut exchange: Option<WorkerHostBridge>,
    ) -> Result<Result<T, String>, String> {
        let job_foreground = exchange.as_mut().and_then(|bridge| bridge.job_foreground.take());
        let subagents = exchange.as_ref().and_then(|bridge| bridge.subagents.clone());
        let pumps_jobs = job_foreground.is_some();
        let _job_foreground = self.jobs.enter(job_foreground);
        self.with_budget(
            Instant::now() + SCRIPT_TIMEOUT,
            Some(cancellation.clone()),
            || {
                self.context.with(|ctx| {
                    let error = |error: rquickjs::CaughtError<'_>| error.to_string();
                    let manager: Object = ctx
                        .globals()
                        .get(MANAGER_GLOBAL)
                        .catch(&ctx)
                        .map_err(error)?;
                    let module: Object = ctx
                        .globals()
                        .get(MODULE_GLOBAL)
                        .catch(&ctx)
                        .map_err(error)?;
                    Self::update_manager(&ctx, &manager, &initial.mcp)?;
                    Self::refresh_storage(&ctx, &module, &initial.storage)?;
                    let input = ctx.json_parse(invocation).catch(&ctx).map_err(error)?;
                    let invoke: Function = module.get(method).catch(&ctx).map_err(error)?;
                    let scope = ctx.json_parse(initial.subagent_scope.to_string()).catch(&ctx).map_err(error)?;
                    let result: Promise = invoke.call((input, scope)).catch(&ctx).map_err(error)?;
                    loop {
                        if let Some(runs) = &subagents {
                            Self::settle_subagents(&ctx, &module, runs)?;
                        }
                        if let Some(result) = result.result() {
                            return match result.catch(&ctx) {
                                Ok(value) => decode_js(&ctx, value).map(Ok),
                                Err(failure) if (self.keeps_jobs || self.keeps_subagents) && !self.out_of_time(&cancellation) => {
                                    Ok(Err(error(failure)))
                                }
                                Err(failure) => Err(error(failure)),
                            };
                        }
                        if self.out_of_time(&cancellation) {
                            return Err("Cordis JavaScript timed out or was cancelled".into());
                        }
                        if ctx.execute_pending_job() {
                            continue;
                        }
                        let exchange = exchange
                            .as_mut()
                            .ok_or("Cordis command is waiting on unavailable host work")?;
                        let jobs = pumps_jobs.then(|| self.job_schedule(&ctx)).transpose()?;
                        let next: Function = manager.get("requests").catch(&ctx).map_err(error)?;
                        let allow_job_wait = jobs.as_ref().is_some_and(|schedule| schedule.live);
                        let wait_subagents = subagents.as_ref().is_some_and(|runs| runs.has_pending());
                        let host_pending: Function = manager.get("hasPendingRequests").catch(&ctx).map_err(error)?;
                        let host_pending: bool = host_pending.call(()).catch(&ctx).map_err(error)?;
                        let requests: Vec<HostOperation> =
                            decode_js(&ctx, next.call((allow_job_wait || wait_subagents,)).catch(&ctx).map_err(error)?)?;
                        let started = Instant::now();
                        if !requests.is_empty() || jobs.is_some() || wait_subagents {
                            exchange
                                .requests
                                .blocking_send(WorkerBatch { requests, jobs, wait_subagents, host_pending })
                                .map_err(|_| "Cordis host callback has ended")?;
                        }
                        let reply = exchange
                            .replies
                            .blocking_recv()
                            .ok_or("Cordis host callback has ended")?;
                        if let Some(active) = self
                            .budget
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                            .as_mut()
                        {
                            active.deadline += started.elapsed();
                        }
                        match reply {
                            WorkerReply::Response(bytes) => {
                                let resume: Function = manager.get("resume").catch(&ctx).map_err(error)?;
                                let reply = ctx.json_parse(bytes).catch(&ctx).map_err(error)?;
                                resume.call::<_, ()>((reply,)).catch(&ctx).map_err(error)?;
                            }
                            WorkerReply::Workflow(event, _slot) => {
                                Self::emit_session_event(&ctx, &manager, &event)?;
                            }
                            WorkerReply::Jobs(_slot) => {
                                self.advance_parked_jobs(&ctx, &cancellation)?;
                            }
                            WorkerReply::Subagents => {}
                        }
                    }
                })
            },
        )
    }

    fn settle_subagents<'js>(
        ctx: &Ctx<'js>,
        module: &Object<'js>,
        runs: &subagents::Runs,
    ) -> Result<(), String> {
        let completed = runs.take_completed();
        if completed.is_empty() {
            return Ok(());
        }
        let error = |error: rquickjs::CaughtError<'_>| error.to_string();
        let settle: Function = module.get("subagentSettled").catch(ctx).map_err(error)?;
        for outcome in completed {
            let wire = serde_json::to_vec(&outcome).map_err(|error| error.to_string())?;
            let value = ctx.json_parse(wire).catch(ctx).map_err(error)?;
            settle.call::<_, ()>((value,)).catch(ctx).map_err(error)?;
        }
        Ok(())
    }

    fn advance_subagents(
        &self,
        runs: &subagents::Runs,
        cancellation: CancellationToken,
    ) -> Result<(), String> {
        self.with_budget(Instant::now() + SCRIPT_TIMEOUT, Some(cancellation.clone()), || {
            self.context.with(|ctx| {
                let module = ctx.globals().get(MODULE_GLOBAL).catch(&ctx).map_err(|error| error.to_string())?;
                Self::settle_subagents(&ctx, &module, runs)?;
                while !self.out_of_time(&cancellation) && ctx.execute_pending_job() {}
                Ok(())
            })
        })
    }

    fn update_status(
        &self,
        snapshot: &Value,
        cancellation: CancellationToken,
    ) -> Result<(), String> {
        self.with_budget(
            Instant::now() + SCRIPT_TIMEOUT,
            Some(cancellation.clone()),
            || {
                self.context.with(|ctx| {
                    let manager: Object = ctx
                        .globals()
                        .get(MANAGER_GLOBAL)
                        .catch(&ctx)
                        .map_err(|error| error.to_string())?;
                    Self::update_manager(&ctx, &manager, snapshot)?;
                    while !self.out_of_time(&cancellation) && ctx.execute_pending_job() {}
                    Ok(())
                })
            },
        )
    }

    fn session_event(
        &self,
        event: &events::Event,
        cancellation: CancellationToken,
    ) -> Result<(), String> {
        self.with_budget(
            Instant::now() + SCRIPT_TIMEOUT,
            Some(cancellation.clone()),
            || self.context.with(|ctx| {
                let error = |error: rquickjs::CaughtError<'_>| error.to_string();
                let manager: Object = ctx.globals().get(MANAGER_GLOBAL).catch(&ctx).map_err(error)?;
                Self::emit_session_event(&ctx, &manager, event)?;
                while !self.out_of_time(&cancellation) && ctx.execute_pending_job() {}
                Ok(())
            }),
        )
    }

    fn emit_session_event<'js>(
        ctx: &Ctx<'js>,
        manager: &Object<'js>,
        event: &events::Event,
    ) -> Result<(), String> {
        let error = |error: rquickjs::CaughtError<'_>| error.to_string();
        let emit: Function = manager.get("sessionEvent").catch(ctx).map_err(error)?;
        let event = ctx.json_parse(event.wire().to_string()).catch(ctx).map_err(error)?;
        emit.call::<_, ()>((event,)).catch(ctx).map_err(error)
    }

    fn update_manager<'js>(
        ctx: &Ctx<'js>,
        manager: &Object<'js>,
        snapshot: &Value,
    ) -> Result<(), String> {
        let error = |error: rquickjs::CaughtError<'_>| error.to_string();
        let update: Function = manager.get("update").catch(ctx).map_err(error)?;
        let snapshot = ctx
            .json_parse(snapshot.to_string())
            .catch(ctx)
            .map_err(error)?;
        update.call::<_, ()>((snapshot,)).catch(ctx).map_err(error)
    }

    fn refresh_storage<'js>(
        ctx: &Ctx<'js>,
        module: &Object<'js>,
        snapshot: &Value,
    ) -> Result<(), String> {
        let error = |error: rquickjs::CaughtError<'_>| error.to_string();
        let refresh: Function = module.get("refreshStorage").catch(ctx).map_err(error)?;
        let snapshot = ctx.json_parse(snapshot.to_string()).catch(ctx).map_err(error)?;
        refresh.call::<_, ()>((snapshot,)).catch(ctx).map_err(error)
    }

    fn out_of_time(&self, cancellation: &CancellationToken) -> bool {
        cancellation.is_cancelled()
            || self
                .budget
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .as_ref()
                .is_some_and(|budget| Instant::now() >= budget.deadline)
    }
}

fn decode_js<'js, T: serde::de::DeserializeOwned>(
    ctx: &Ctx<'js>,
    value: rquickjs::Value<'js>,
) -> Result<T, String> {
    let json = ctx
        .json_stringify(value)
        .catch(ctx)
        .map_err(|error| error.to_string())?
        .ok_or("Cordis result is not JSON")?
        .to_string()
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&json).map_err(|error| error.to_string())
}

struct WorkerHostBridge {
    requests: mpsc::Sender<WorkerBatch>,
    replies: mpsc::Receiver<WorkerReply>,
    job_foreground: Option<jobs::Foreground>,
    subagents: Option<Arc<subagents::Runs>>,
}

struct WorkerBatch {
    requests: Vec<HostOperation>,
    jobs: Option<jobs::Schedule>,
    wait_subagents: bool,
    host_pending: bool,
}

enum WorkerReply {
    Response(Vec<u8>),
    Workflow(events::Event, OwnedSemaphorePermit),
    Jobs(OwnedSemaphorePermit),
    Subagents,
}

#[derive(Clone)]
struct WorkerMessages {
    sender: mpsc::Sender<WorkerReply>,
    workflow_slots: Arc<Semaphore>,
    job_slot: Arc<Semaphore>,
}

impl WorkerMessages {
    fn jobs(&self) {
        if let Ok(slot) = self.job_slot.clone().try_acquire_owned() {
            let _ = self.sender.try_send(WorkerReply::Jobs(slot));
        }
    }

    fn workflow(&self, event: events::Event) {
        // Limit event frames independently so a burst cannot consume room
        // reserved for host replies that resolve pending JS promises.
        if let Ok(slot) = self.workflow_slots.clone().try_acquire_owned() {
            let _ = self.sender.try_send(WorkerReply::Workflow(event, slot));
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HostOperation {
    Request {
        id: u32,
        #[serde(flatten)]
        request: HostRequest,
        cancel_after_ms: Option<u64>,
    },
    Cancel {
        id: u32,
        reason: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "service", content = "request", rename_all = "snake_case")]
enum HostRequest {
    Mcp(DeepSeekMcpManagerRequest),
    Storage(storage::Mutation),
    StorageRecords(storage::RecordMutation),
    Workflow(workflow::CordisRequest),
    WorkflowBackground(workflow::CordisBackgroundRequest),
    WorkflowJob(CordisWorkflowJobRequest),
    SubagentStart(Box<subagents::Request>),
    SubagentDispose(subagents::Dispose),
    SubagentControl(subagents::ControlRequest),
    Llm(CordisLlmRequest),
}

pub(super) struct WorkflowCaller {
    pub(super) bridge: AgentBridge,
    pub(super) host: Arc<dyn HostClient>,
    pub(super) session: i64,
    pub(super) call: i64,
}

async fn answer_mcp_request(
    host: &dyn HostClient,
    id: u32,
    request: DeepSeekMcpManagerRequest,
    cancellation: CancellationToken,
    cancel_after_ms: Option<u64>,
) -> Result<(u32, Vec<u8>), String> {
    let name = match &request {
        DeepSeekMcpManagerRequest::List | DeepSeekMcpManagerRequest::Watch { .. } => None,
        DeepSeekMcpManagerRequest::Probe { name, .. }
        | DeepSeekMcpManagerRequest::Restart { name } => Some(name.clone()),
    };
    let call = host.deepseek_mcp_manager(request, cancellation.clone());
    tokio::pin!(call);
    let result = if let Some(delay) = cancel_after_ms {
        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(delay)) => {
                cancellation.cancel();
                call.await
            }
            result = &mut call => result,
        }
    } else {
        call.await
    };
    let result = match result {
        Ok(value) => json!({ "id": id, "value": value }),
        Err(error) => {
            let detail = error.failure.user.fallback.as_str();
            let message = match name.as_deref() {
                Some(name) => format!("{name}: {detail}"),
                None => detail.to_owned(),
            };
            json!({ "id": id, "error": message })
        }
    };
    let mut reply = serde_json::to_vec(&result).map_err(|error| error.to_string())?;
    if reply.len() > 512 * 1024 {
        reply = serde_json::to_vec(
            &json!({ "id": id, "error": "Cordis MCP reply exceeded its byte limit" }),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok((id, reply))
}

async fn answer_host_request(
    program: &Program,
    host: &dyn HostClient,
    session: Option<i64>,
    id: u32,
    request: HostRequest,
    cancellation: CancellationToken,
    cancel_after_ms: Option<u64>,
    caller: Option<&WorkflowCaller>,
    events: Option<events::RunEvents>,
    subagents: Option<Arc<subagents::Runs>>,
) -> Result<(u32, Vec<u8>), String> {
    let result = match request {
        HostRequest::Mcp(request) if program.uses_mcp_manager => {
            return answer_mcp_request(host, id, request, cancellation, cancel_after_ms).await;
        }
        HostRequest::Storage(mutation) if program.uses_storage => {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(PluginError::from_kind(PluginErrorKind::Cancelled, "Cordis storage operation cancelled")),
                result = program.storage.commit(host, mutation) => result,
            }
        }
        HostRequest::StorageRecords(mutation) if program.uses_storage => {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(PluginError::from_kind(PluginErrorKind::Cancelled, "Cordis storage operation cancelled")),
                result = program.storage.commit_records(host, mutation) => result,
            }
        }
        HostRequest::Workflow(request) if program.uses_workflow => {
            let (Some(caller), Some(events)) = (caller, events) else {
                return serde_json::to_vec(&json!({"id": id, "error": "Cordis workflow requires a live project tool call"}))
                    .map(|reply| (id, reply)).map_err(|error| error.to_string());
            };
            let config = program
                .workflow_config
                .as_ref()
                .ok_or("Cordis workflow configuration is unavailable")?;
            let work = workflow::execute_cordis(
                request,
                config,
                caller,
                cancellation.clone(),
                events,
            );
            tokio::pin!(work);
            if let Some(delay) = cancel_after_ms {
                tokio::select! {
                    biased;
                    _ = tokio::time::sleep(Duration::from_millis(delay)) => { cancellation.cancel(); work.await }
                    result = &mut work => result,
                }
            } else {
                work.await
            }
        }
        HostRequest::WorkflowBackground(request) if program.uses_workflow => {
            match (caller, &program.workflow_config, &program.jobs_config) {
                (Some(caller), Some(config), Some(jobs)) if program.job_controllers.serves(caller.session) => {
                    let events = program.session_events.run(
                        caller.session, request.workflow.id, &request.workflow.meta,
                    );
                    let start = workflow::start_cordis(
                        request, config, jobs, caller.bridge.clone(), caller.host.as_ref(),
                        caller.call, cancellation.clone(), events, program.job_controllers.has_native(),
                    );
                    tokio::pin!(start);
                    if let Some(delay) = cancel_after_ms {
                        tokio::select! {
                            biased;
                            _ = tokio::time::sleep(Duration::from_millis(delay)) => {
                                cancellation.cancel();
                                start.await
                            }
                            result = &mut start => result,
                        }
                    } else {
                        start.await
                    }
                }
                _ => Err(PluginError::invalid_params("Cordis background workflow requires a live tool and job controller")),
            }
        }
        HostRequest::WorkflowJob(request) if program.uses_workflow && program.jobs_config.is_some() && caller.is_some() => {
            host.cordis_workflow_job(request, cancellation).await
        }
        HostRequest::SubagentStart(request) if program.uses_subagents => {
            if let (Some(runs), Some(config)) = (subagents, program.workflow_config.as_ref()) {
                let events = session
                    .or_else(|| {
                        agena_plugin_sdk::host_api::current_host_callback_context()
                            .and_then(|context| context.session_id)
                    })
                    .map(|session| program.session_events.subagent(session, program.entry_index));
                let start = runs.start(host, *request, config, cancellation.clone(), events);
                tokio::pin!(start);
                if let Some(delay) = cancel_after_ms {
                    tokio::select! {
                        biased;
                        _ = tokio::time::sleep(Duration::from_millis(delay)) => {
                            cancellation.cancel();
                            start.await
                        }
                        result = &mut start => result,
                    }
                } else {
                    start.await
                }
            } else {
                Err(subagents::unavailable())
            }
        }
        HostRequest::SubagentDispose(request) if program.uses_subagents => {
            match subagents {
                Some(runs) => runs.dispose(request.run_id).await,
                None => Err(subagents::unavailable()),
            }
        }
        HostRequest::SubagentControl(request) if program.uses_subagents => {
            let control = request.into_host();
            match control {
                Ok(control) => {
                    let operation = host.cordis_subagent_control(control, cancellation.clone());
                    tokio::pin!(operation);
                    if let Some(delay) = cancel_after_ms {
                        tokio::select! {
                            biased;
                            _ = tokio::time::sleep(Duration::from_millis(delay)) => {
                                cancellation.cancel();
                                operation.await
                            }
                            result = &mut operation => result,
                        }
                    } else {
                        operation.await
                    }
                }
                Err(error) => Err(error),
            }
        }
        HostRequest::Llm(request) if program.uses_llm => {
            let operation = host.cordis_llm_complete(request, cancellation.clone());
            tokio::pin!(operation);
            let result = if let Some(delay) = cancel_after_ms {
                tokio::select! {
                    biased;
                    _ = tokio::time::sleep(Duration::from_millis(delay)) => {
                        cancellation.cancel();
                        operation.await
                    }
                    result = &mut operation => result,
                }
            } else {
                operation.await
            };
            result.and_then(|response| {
                serde_json::to_value(response).map_err(|error| PluginError::internal_error(&error))
            })
        }
        _ => Err(PluginError::invalid_params(
            "Cordis host service was not injected",
        )),
    };
    let reply = match result {
        Ok(value) => json!({"id": id, "value": value}),
        Err(error) => json!({"id": id, "error": error.failure.user.fallback}),
    };
    serde_json::to_vec(&reply)
        .map(|reply| (id, reply))
        .map_err(|error| error.to_string())
}

impl Program {
    async fn invoke<T: serde::de::DeserializeOwned + Send + 'static>(
        self: &Arc<Self>,
        method: &'static str,
        session: Option<i64>,
        host: Option<&dyn HostClient>,
        invocation: Vec<u8>,
        caller: Option<WorkflowCaller>,
        timeout: Option<Duration>,
    ) -> agena_plugin_sdk::Result<T> {
        let cancel = agena_plugin_sdk::current_cancellation_token()
            .unwrap_or_default()
            .child_token();
        let _stop = cancel.clone().drop_guard();
        let command_jobs = method == "invoke" && self.uses_jobs
            && agena_plugin_sdk::host_api::current_host_callback_context()
                .is_some_and(|context| context.plugin_command.is_some());
        let can_start_jobs = method == "invokeTool" || command_jobs;
        if (self.uses_jobs || self.uses_workflow) && can_start_jobs
            && let (Some(host), Some(session)) = (host, session)
        {
            self.job_controllers.start_modules(session, host).await?;
        }
        let manager = if let Some(host) = host.filter(|_| self.uses_mcp_manager) {
            match host
                .deepseek_mcp_manager(DeepSeekMcpManagerRequest::List, cancel.child_token())
                .await
            {
                Ok(snapshot) => snapshot,
                Err(error) => json!({ "error": error.failure.user.fallback }),
            }
        } else {
            Value::Null
        };
        let retained =
            if let (Some(host), Some(session)) = (host.filter(|_| self.keeps_session()), session) {
                match self.start_session(session, host).await {
                    Ok(retained) => Some(retained),
                    Err(_) if self.uses_mcp_manager && !self.uses_storage && !self.uses_skills && !self.uses_jobs && !self.uses_subagents && !self.uses_llm => None,
                    Err(error) => return Err(error),
                }
            } else {
                None
            };
        let plugin_cancel = cancel.clone();
        let program = self.clone();
        let subagents = retained.as_ref().filter(|_| self.uses_subagents)
            .map(|retained| retained.subagents.clone());
        let (requests, mut receive_requests) = mpsc::channel(1);
        let (start_job, mut admit_jobs) = mpsc::channel(1);
        let (send_replies, replies) =
            mpsc::channel(MAX_HOST_OPERATIONS + events::EVENT_CAPACITY + 2);
        let messages = WorkerMessages {
            sender: send_replies,
            workflow_slots: Arc::new(Semaphore::new(events::EVENT_CAPACITY)),
            job_slot: Arc::new(Semaphore::new(1)),
        };
        let pumps_jobs = self.uses_jobs && retained.is_some() && host.is_some();
        let job_config = self.jobs_config.as_deref().filter(|_| pumps_jobs && can_start_jobs);
        let job_wake = Arc::new(tokio::sync::Notify::new());
        let job_foreground = pumps_jobs.then(|| jobs::Foreground {
            starts: job_config.is_some().then_some(start_job),
            wake: job_wake.clone(),
        });
        let uses_host = (self.uses_mcp_manager
            || self.uses_storage
            || self.uses_workflow
            || self.uses_subagents
            || self.uses_llm
            || pumps_jobs)
            && host.is_some();
        let worker_subagents = subagents.clone();
        let worker = async move {
            let exchange = uses_host.then_some(WorkerHostBridge {
                requests,
                replies,
                job_foreground,
                subagents: worker_subagents,
            });
            let snapshot = async {
                let subagent_scope = if let Some(host) = host.filter(|_| program.uses_subagents && session.is_some()) {
                    match host.cordis_subagent_control(CordisSubagentControl::Scope, plugin_cancel.child_token()).await {
                        Ok(scope) => scope,
                        Err(error) => json!({"error":error.failure.user.fallback}),
                    }
                } else {
                    Value::Null
                };
                Ok(RuntimeSnapshot {
                    mcp: manager,
                    storage: program.storage_snapshot(host).await?,
                    subagent_scope,
                })
            };
            if let Some(retained) = retained {
                return retained
                    .run(method, invocation, snapshot, plugin_cancel.clone(), exchange)
                    .await;
            }
            let snapshot = snapshot.await?;
            WORKERS
                .run(move || {
                    program
                        .load(
                            &snapshot,
                            Instant::now() + SCRIPT_TIMEOUT,
                            Some(plugin_cancel.clone()),
                            None,
                        )?
                        .run(method, &invocation, &snapshot, plugin_cancel, exchange)
                        .and_then(|result| result)
                })
                .await
                .map_err(|error| PluginError::internal_error(&error))?
                .map_err(PluginError::internal)
        };
        tokio::pin!(worker);
        let mut operations = FuturesUnordered::new();
        let mut job_admissions = FuturesUnordered::new();
        let mut tokens = BTreeMap::new();
        let mut jobs_parked = false;
        let mut job_deadline: Option<tokio::time::Instant> = None;
        let mut subagents_parked = false;
        let mut host_pending = false;
        let mut started = 0;
        let expires = async {
            match timeout {
                Some(timeout) => tokio::time::sleep(timeout).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(expires);
        let result = async { loop {
            let yield_tool_slot = !host_pending && !jobs_parked && operations.is_empty() && job_admissions.is_empty();
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    return Err(PluginError::from_kind(PluginErrorKind::Cancelled, "Cordis invocation cancelled"));
                }
                _ = &mut expires => {
                    return Err(PluginError::from_kind(PluginErrorKind::Timeout, "Cordis tool timed out"));
                }
                reply = &mut worker => return reply,
                Some(admission) = admit_jobs.recv(), if job_config.is_some() && !admit_jobs.is_closed() => {
                    started += 1;
                    if started > MAX_HOST_OPERATIONS {
                        return Err(PluginError::invalid_params("Cordis host operation limit reached"));
                    }
                    let host = host.expect("Cordis job admission has a live host");
                    let config = job_config.expect("Cordis job admission is configured");
                    job_admissions.push(admission.answer(host, config, self.job_controllers.has_native(), cancel.child_token()));
                }
                _ = job_admissions.next(), if !job_admissions.is_empty() => {}
                Some(batch) = receive_requests.recv(), if !receive_requests.is_closed() => {
                    jobs_parked = batch.jobs.is_some();
                    subagents_parked = batch.wait_subagents;
                    host_pending = batch.host_pending;
                    job_deadline = batch.jobs.and_then(|schedule| schedule.delay_ms)
                        .map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms));
                    for operation in batch.requests {
                        match operation {
                            HostOperation::Request { id, request, cancel_after_ms } => {
                                started += 1;
                                if started > MAX_HOST_OPERATIONS || tokens.len() >= 8 || tokens.contains_key(&id) {
                                    return Err(PluginError::invalid_params("Cordis host operation limit reached"));
                                }
                                let token = cancel.child_token();
                                let events = match (&request, caller.as_ref()) {
                                    (HostRequest::Workflow(workflow), Some(caller)) if self.uses_workflow => {
                                        Some(self.session_events.foreground(
                                            caller.session, workflow.id, &workflow.meta,
                                            self.entry_index, messages.clone(),
                                        ))
                                    }
                                    _ => None,
                                };
                                tokens.insert(id, (token.clone(), events.clone()));
                                let host = host.ok_or_else(|| PluginError::internal("Cordis host is unavailable"))?;
                                operations.push(answer_host_request(self, host, session, id, request, token, cancel_after_ms, caller.as_ref(), events, subagents.clone()));
                            }
                            HostOperation::Cancel { id, reason } => {
                                if let Some((token, events)) = tokens.get(&id) {
                                    if !token.is_cancelled()
                                        && let (Some(events), Some(reason)) = (events, reason)
                                    {
                                        events.record_cancellation(&reason);
                                    }
                                    token.cancel();
                                }
                            }
                        }
                    }
                }
                reply = operations.next(), if !operations.is_empty() => {
                    if let Some(reply) = reply {
                        let (id, bytes) = reply.map_err(PluginError::internal)?;
                        tokens.remove(&id);
                        let _ = messages.sender.try_send(WorkerReply::Response(bytes));
                    }
                }
                _ = wait_for_job_tick(&job_wake, job_deadline), if jobs_parked => {
                    jobs_parked = false;
                    messages.jobs();
                }
                result = wait_for_subagents(
                    host, subagents.clone(), yield_tool_slot,
                ), if subagents_parked => {
                    result?;
                    subagents_parked = false;
                    let _ = messages.sender.try_send(WorkerReply::Subagents);
                }
            }
        }}.await;
        cancel.cancel();
        if job_config.is_some() || (caller.is_some() && self.uses_workflow) || self.uses_subagents {
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(
                    async { while operations.next().await.is_some() {} },
                    async { while job_admissions.next().await.is_some() {} },
                );
            })
            .await;
        }
        result
    }
}

async fn wait_for_subagents(
    host: Option<&dyn HostClient>,
    runs: Option<Arc<subagents::Runs>>,
    yield_tool_slot: bool,
) -> agena_plugin_sdk::Result<()> {
    let (Some(host), Some(runs)) = (host, runs) else {
        return Err(subagents::unavailable());
    };
    if yield_tool_slot {
        host.wait_for_cordis_subagent(Box::pin(async move { runs.ready().await })).await
    } else {
        runs.ready().await;
        Ok(())
    }
}

async fn wait_for_job_tick(wake: &tokio::sync::Notify, deadline: Option<tokio::time::Instant>) {
    if let Some(deadline) = deadline {
        tokio::select! {
            _ = wake.notified() => {},
            _ = tokio::time::sleep_until(deadline) => {},
        }
    } else {
        wake.notified().await;
    }
}

impl Command {
    pub(super) fn accepts_attachments(&self) -> bool {
        self.attachments
    }

    pub(super) fn uses_mcp_manager(&self) -> bool {
        self.program.uses_mcp_manager
    }

    pub(super) async fn invoke(
        &self,
        input: CommandInvokeInput,
        host: Option<&dyn HostClient>,
    ) -> agena_plugin_sdk::Result<CommandResult> {
        let args = input
            .input
            .get("args")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if args.len() > MAX_ARGUMENT_BYTES {
            return Err(PluginError::invalid_params(
                "command arguments exceed 16 KiB",
            ));
        }
        let composer = input.cordis_attachments.is_some();
        let invocation = serde_json::to_vec(&json!({
            "name": self.name, "id": uuid::Uuid::new_v4(), "args": args,
            "sessionId": input.session_id,
            "attachments": input.cordis_attachments,
        }))
        .map_err(|error| PluginError::internal_error(&error))?;
        let reply: Reply = self
            .program
            .invoke("invoke", input.session_id, host, invocation, None, None)
            .await?;
        let text = reply.text.unwrap_or_default();
        let summary: String = text.chars().take(MAX_RESULT_CHARS).collect();
        if matches!(reply.kind, ReplyKind::Error) {
            return Ok(CommandResult::failed(summary));
        }
        let mut result = CommandResult::succeeded(summary);
        let Some(mut followup) = reply.followup else {
            return Ok(result);
        };
        if followup.iter().all(
            |block| matches!(block, CordisFollowupBlock::Text { text } if text.trim().is_empty()),
        ) {
            return Ok(result);
        }
        if !input.prompt_context.is_empty() {
            followup.push(CordisFollowupBlock::Text {
                text: format!("\n{}", input.prompt_context),
            });
        }
        let prompt_bytes: usize = followup
            .iter()
            .map(|block| match block {
                CordisFollowupBlock::Text { text } => text.len() + 1,
                CordisFollowupBlock::Attachment { .. } => 0,
            })
            .sum();
        if prompt_bytes > MAX_PROMPT_BYTES {
            return Err(PluginError::invalid_params(
                "Cordis command prompt exceeds 128 KiB",
            ));
        }
        if composer {
            let output = serde_json::to_value(followup)
                .map_err(|error| PluginError::internal_error(&error))?;
            result = result.with_output(output);
        } else {
            let prompt = followup
                .into_iter()
                .map(|block| match block {
                    CordisFollowupBlock::Text { text } => Ok(text),
                    CordisFollowupBlock::Attachment { .. } => Err(PluginError::invalid_params(
                        "Unexpected Cordis followup attachment",
                    )),
                })
                .collect::<agena_plugin_sdk::Result<Vec<_>>>()?
                .join("\n");
            result = result.with_effect(CommandHostEffect::InsertPrompt { prompt });
        }
        Ok(result)
    }
}
