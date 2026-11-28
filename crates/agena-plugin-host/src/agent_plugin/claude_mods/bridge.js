(() => {
    const hooks = [];
    const frames = new Map();
    const has = (value, key) => Object.hasOwn(value, key);
    const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
    const strings = value => Array.isArray(value) && value.every(item => typeof item === 'string');

    function signature(value) {
        if (value instanceof RegExp) return ['regex', value.source, value.flags];
        if (Array.isArray(value)) return ['list', ...value.map(signature)];
        if (object(value)) return ['object', ...Object.keys(value).sort().map(key => [key, signature(value[key])])];
        return value;
    }

    function matches(pattern, value) {
        if (pattern instanceof RegExp) {
            pattern.lastIndex = 0;
            return typeof value === 'string' && pattern.test(value);
        }
        if (Array.isArray(pattern)) return pattern.some(item => matches(item, value));
        if (object(pattern)) {
            if (Array.isArray(value)) return value.some(item => matches(pattern, item));
            return object(value) && Object.keys(pattern).every(key => matches(pattern[key], value[key]));
        }
        return pattern === value;
    }

    function validateAnswer(event, value) {
        if (!object(value)) throw new Error(`${event} must answer with an object`);
        for (const key of ['block', 'stopReason', 'deny', 'ask', 'drop', 'text', 'description', 'summary', 'help', 'searchHint', 'whenToUse', 'reason', 'initialUserMessage', 'sessionTitle', 'permissionDecisionReason']) {
            if (value[key] !== undefined && typeof value[key] !== 'string') throw new Error(`Invalid ${key} on ${event}`);
        }
        for (const key of ['additionalContext', 'context', 'watchPaths']) {
            if (value[key] !== undefined && !strings(value[key])) throw new Error(`Invalid ${key} on ${event}`);
        }
        if (value.updatedInput !== undefined && !object(value.updatedInput)) throw new Error('updatedInput must be an object');
        if (value.isEnd !== undefined && typeof value.isEnd !== 'boolean') throw new Error(`Invalid isEnd on ${event}`);
        if (value.content !== undefined && !Array.isArray(value.content)) throw new Error(`Invalid content on ${event}`);
        if ((event === 'tool.call' || event === 'tool.run')
            && !has(value, 'result') && !Array.isArray(value.content) && typeof value.deny !== 'string') {
            throw new Error(`${event} must return a result, content, or a refusal`);
        }
        if (event === 'tool.offer' && typeof value.offer !== 'boolean') {
            throw new Error('tool.offer must return a boolean offer decision');
        }
        if (event === 'tool.check' && (!['allow', 'ask', 'deny'].includes(value.decision)
            || (value.reason !== undefined && typeof value.reason !== 'string'))) {
            throw new Error('tool.check must return an allow, ask, or deny decision');
        }
        if (event === 'prompt.submit' && (typeof value.text === 'string') === (typeof value.drop === 'string')) {
            throw new Error('prompt.submit must return the forwarded text or a drop reason');
        }
        return value;
    }

    function register(module, source, options) {
        if (typeof module.register !== 'function') throw new Error('Expected an exported register(on, options)');
        const start = hooks.length;
        const entries = new Set();
        let open = true;
        const on = (event, matcher, handler) => {
            if (!open) throw new Error('on is only available during register');
            if (typeof matcher === 'function') {
                handler = matcher;
                matcher = undefined;
            }
            if (typeof event !== 'string' || !/^(\*|[a-z][\w.]*\.(\*|[A-Za-z][\w.]*))$/.test(event)
                || typeof handler !== 'function' || (matcher !== undefined && !object(matcher))) {
                throw new Error('Invalid on(event, matcher, handler) registration');
            }
            const key = JSON.stringify([event, signature(matcher)]);
            if (entries.has(key)) throw new Error(`Duplicate registration for ${event}`);
            if (hooks.length >= 256) throw new Error('Too many Claude mod hooks');
            entries.add(key);
            const entry = { index: hooks.length, source, event, matcher, handler, recover: undefined };
            hooks.push(entry);
            return Object.freeze({ catch(recover) {
                if (!open || typeof recover !== 'function' || entry.recover) throw new Error('Invalid hook error handler');
                entry.recover = recover;
            } });
        };
        try {
            const outcome = module.register(on, options);
            if (outcome != null && typeof outcome.then === 'function') throw new Error('register must be synchronous');
            return hooks.slice(start).map(entry => ({
                event: entry.event,
                tool: typeof entry.matcher?.tool === 'string' ? entry.matcher.tool : null,
            }));
        } catch (error) {
            hooks.length = start;
            throw error;
        } finally {
            open = false;
        }
    }

    async function dispatch(frame, initial, state) {
        const event = frame.event;
        const enqueue = (kind, params, wantsReply) => {
            if (!state.open) throw new Error('Claude mod callback is no longer active');
            if (state.sequence >= frame.maxRequests) throw new Error('Claude mod exceeded its host call limit');
            const id = ++state.sequence;
            state.requests.push({ ...params, id, kind });
            if (wantsReply) return new Promise(resolve => state.pending.set(id, resolve));
        };
        const call = async (kind, params) => {
            const reply = await enqueue(kind, params, true);
            if (reply.error !== undefined) throw new Error(reply.error);
            return reply.value;
        };
        const parseGetOptions = options => {
            if (options === undefined) return false;
            if (!object(options) || (options.version !== undefined && typeof options.version !== 'boolean')) {
                throw new Error('Invalid store.get options');
            }
            return Boolean(options.version);
        };
        const parseVersionCondition = (options, allowZero) => {
            if (options === undefined) return undefined;
            if (!object(options)) throw new Error('Invalid store options');
            if (options.ifVersion === undefined) return undefined;
            if (allowZero && options.ifVersion === null) return 0;
            const min = allowZero ? 0 : 1;
            if (!Number.isSafeInteger(options.ifVersion) || options.ifVersion < min) {
                throw new Error('Invalid ifVersion store option');
            }
            return options.ifVersion;
        };
        const say = (text, options) => enqueue('log', { text: String(text), level: options?.level }, false);
        const promptSubmit = event === 'prompt.submit';
        let promptSettled = false;
        let mediaReads = Promise.resolve();
        const api = Object.freeze({
            plugin: Object.freeze({ name: frame.plugin, root: frame.pluginRoot }),
            session: Object.freeze({
                id: async () => frame.sessionId,
                cwd: async () => frame.cwd,
                root: async () => frame.workspace,
                model: async () => frame.model,
            }),
            env: Object.freeze({ get: async name => has(frame.env, name) ? frame.env[name] : undefined }),
            ui: Object.freeze({ log: say }),
            command: Object.freeze({ register: async definition => call('register_command', { definition }) }),
            tool: Object.freeze({
                register: async definition => {
                    if (!object(definition) || typeof definition.name !== 'string' || typeof definition.description !== 'string') {
                        throw new Error('tool.register requires a name and description');
                    }
                    return call('register_tool', { definition });
                },
            }),
            prompt: Object.freeze({ media: async (input) => {
                if (!object(input) || typeof input.id !== 'string' || !input.id
                    || (input.offset !== undefined && (!Number.isSafeInteger(input.offset) || input.offset < 0))) {
                    return { deny: 'Invalid prompt attachment ID or byte offset' };
                }
                const {id, offset = 0} = input;
                const result = mediaReads.then(() => promptSubmit && !promptSettled
                    ? call('prompt_media', { attachment_id: id, offset })
                    : { deny: 'This hook has no open prompt attachment' });
                mediaReads = result.catch(() => {});
                return result;
            } }),
            http: Object.freeze({ fetch: async (url, init) => {
                if (typeof url !== 'string' || (init !== undefined && !object(init))) {
                    throw new Error('http.fetch requires a URL and optional request options');
                }
                return call('http_fetch', { url, init });
            } }),
            process: Object.freeze({ run: async (argv, options) => {
                if (!strings(argv) || argv.length === 0 || (options !== undefined && !object(options))) {
                    throw new Error('process.run requires argv and an optional options object');
                }
                if ((options?.cwd !== undefined && typeof options.cwd !== 'string')
                    || (options?.stdin !== undefined && typeof options.stdin !== 'string')
                    || (options?.env !== undefined && (!object(options.env)
                        || !Object.values(options.env).every(value => typeof value === 'string')))
                    || (options?.timeoutMs !== undefined
                        && (!Number.isSafeInteger(options.timeoutMs) || options.timeoutMs < 0))) {
                    throw new Error('Invalid process.run options');
                }
                return call('process_run', { argv, options });
            } }),
            store: Object.freeze({
                get: async (key, options) => {
                    const version = parseGetOptions(options);
                    const reply = await call('store_get', { key, version });
                    if (!reply.found) return undefined;
                    return version ? { value: reply.value, version: reply.version } : reply.value;
                },
                set: async (key, value, options) => {
                    const ifVersion = parseVersionCondition(options, true);
                    const params = ifVersion === undefined ? { key, value } : { key, value, if_version: ifVersion };
                    return call('store_set', params);
                },
                delete: async (key, options) => {
                    const ifVersion = parseVersionCondition(options, false);
                    const params = ifVersion === undefined ? { key } : { key, if_version: ifVersion };
                    return call('store_delete', params);
                },
                keys: async () => call('store_keys'),
            }),
        });
        const decorate = (next, extra = {}) => Object.assign(next, {
            event,
            origin: Object.freeze({ plugin: 'engine', tier: 'core' }),
            signal: Object.freeze({ aborted: false, throwIfAborted() {} }),
            budget: { ms: frame.budget, get remainingMs() { return Math.max(0, state.deadline - Date.now()); } },
            to: async () => { throw new Error('Hook tiers are managed by the host'); },
            ...extra,
        });
        const selected = hooks.filter(entry => entry.index !== frame.skipHook && (
            entry.event === event || entry.event === '*'
            || (entry.event.endsWith('.*') && event.startsWith(entry.event.slice(0, -1)))
        ));
        const pinnedCheck = event === 'tool.check' ? JSON.stringify(initial) : undefined;
        const pinnedInput = frame.pinInput ? JSON.stringify(initial) : undefined;

        async function step(index, e) {
            const entry = selected[index];
            if (!entry) return call('next', { event: e });
            if (entry.matcher && !matches(entry.matcher, e)) return step(index + 1, e);
            const prompt = promptSubmit ? JSON.stringify(e) : undefined;
            const fallback = () => {
                if (pinnedCheck !== undefined) return JSON.parse(pinnedCheck);
                if (pinnedInput !== undefined) return JSON.parse(pinnedInput);
                if (prompt !== undefined) return JSON.parse(prompt);
                return e;
            };
            const promptNext = changed => {
                const original = JSON.parse(prompt);
                for (const key of ['origin', 'wait', 'turnId', 'command', 'isMeta']) {
                    if (has(changed, key) && JSON.stringify(signature(changed[key])) !== JSON.stringify(signature(original[key]))) {
                        throw new Error(`prompt.submit cannot change ${key}`);
                    }
                }
                if (typeof changed.text !== 'string' || changed.text.length > 256 * 1024) {
                    throw new Error('prompt.submit requires text within the prompt limit');
                }
                if (changed.context !== undefined && !strings(changed.context)) {
                    throw new Error('prompt.submit context must be a list of strings');
                }
                if (original.command !== undefined && changed.text !== original.text) {
                    say('prompt.submit text rewrite ignored for a slash command', { level: 'warn' });
                } else {
                    original.text = changed.text;
                }
                original.context = changed.context;
                return original;
            };
            let following;
            const next = decorate(async changed => {
                if (following === undefined) {
                    if (!object(changed)) throw new Error('next requires an event object');
                    if (pinnedCheck !== undefined) {
                        const check = JSON.parse(pinnedCheck);
                        for (const key of Object.keys(check)) {
                            if (has(changed, key) && JSON.stringify(signature(changed[key])) !== JSON.stringify(signature(check[key]))) {
                                throw new Error(`tool.check cannot change ${key}`);
                            }
                        }
                        changed = check;
                    }
                    if (pinnedInput !== undefined) {
                        const original = JSON.parse(pinnedInput);
                        if (JSON.stringify(signature(changed)) !== JSON.stringify(signature(original))) {
                            throw new Error(`${event} cannot change an approved tool input`);
                        }
                        changed = original;
                    }
                    if (promptSubmit) changed = promptNext(changed);
                    following = step(index + 1, changed);
                }
                const result = await following;
                if (promptSubmit) promptSettled = true;
                // A plugin can annotate a copy of the receipt. The result
                // returned to the rest of the chain still reflects the commit.
                return promptSubmit ? JSON.parse(JSON.stringify(result)) : result;
            });
            async function answer(value) {
                if (value === undefined) return following ?? step(index + 1, fallback());
                const checked = validateAnswer(event, value);
                if (promptSubmit && following !== undefined) {
                    const entered = await following;
                    if (has(checked, 'drop') && !has(entered, 'drop')) {
                        throw new Error('prompt.submit cannot drop a prompt after next was answered');
                    }
                    return entered;
                }
                return checked;
            }
            const handlerApi = Object.freeze({
                ...api,
                tool: Object.freeze({
                    ...api.tool,
                    list: async () => call('tool_operation', { hook: entry.index, request: { operation:'list' } }),
                    check: async request => {
                        if (!object(request) || typeof request.tool !== 'string' || !object(request.input)) {
                            throw new Error('tool.check requires a tool name and input object');
                        }
                        const {tool, input} = request;
                        return call('tool_operation', { hook: entry.index, request: { operation:'check', tool, input } });
                    },
                    call: async request => {
                        if (!object(request) || typeof request.tool !== 'string') {
                            throw new Error('tool.call requires a tool name and its arguments');
                        }
                        const {tool, tool_use_id, agentId, requestMeta, consent, ...input} = request;
                        return call('tool_operation', { hook: entry.index, request: { operation:'call', tool, input } });
                    },
                }),
                prompt: Object.freeze({
                    ...api.prompt,
                    submit: async input => {
                        if (!object(input) || typeof input.text !== 'string' || !input.text.trim()
                            || (input.wait !== undefined && typeof input.wait !== 'boolean')
                            || (input.asUser !== undefined && typeof input.asUser !== 'boolean')) {
                            return { drop: 'prompt.submit requires text and optional boolean wait and asUser settings' };
                        }
                        const {text, wait = true, asUser = false} = input;
                        return call('submit_prompt', { hook: entry.index, request: {text, wait, asUser} });
                    },
                }),
            });
            try {
                return await answer(await entry.handler(handlerApi, e, next));
            } catch (error) {
                enqueue('hook_error', { source: entry.source, text: String(error) }, false);
                if (entry.recover) {
                    const recoverNext = decorate(next, { called: following !== undefined, error: { kind: 'throw', message: String(error) } });
                    try {
                        const value = await entry.recover(handlerApi, e, recoverNext);
                        if (value !== undefined) return await answer(value);
                    } catch (recoveryError) {
                        enqueue('hook_error', { source: entry.source, text: String(recoveryError) }, false);
                    }
                }
                return following ?? step(index + 1, fallback());
            }
        }
        return step(0, initial);
    }

    function begin(frame, initial) {
        const state = { sequence: 0, open: true, requests: [], pending: new Map(), outcome: {}, deadline: frame.deadline };
        frames.set(frame.id, state);
        dispatch(frame, initial, state).then(
            answer => { state.outcome = { answer }; },
            error => { state.outcome = { error: String(error) }; },
        );
    }

    function resume(id, replies, deadline) {
        const state = frames.get(id);
        state.deadline = deadline;
        for (const reply of replies) {
            const resolve = state.pending.get(reply.id);
            state.pending.delete(reply.id);
            resolve?.(reply);
        }
    }

    function poll(id) {
        const state = frames.get(id);
        return { ...state.outcome, requests: state.requests.splice(0) };
    }

    function close(id) {
        const state = frames.get(id);
        if (!state) return;
        state.open = false;
        state.pending.clear();
        state.requests.length = 0;
        frames.delete(id);
    }

    return Object.freeze({ register, begin, resume, poll, close });
})()
