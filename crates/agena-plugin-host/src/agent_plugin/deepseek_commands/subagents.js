((manager, uuid, configuredProviders, defaultDepth, maxActiveSubagents) => {
    const providers = new Map(configuredProviders.map(provider => [
        provider.name, Object.freeze({...provider, capabilities: Object.freeze(provider.capabilities)}),
    ]));
    const customProviders = new Map();
    const active = new Map();
    const startedThisCall = new Set();
    let parent;
    let scope;
    let root;
    const interrupts = [];

    function requireCaller(caller) {
        if (parent === undefined || caller !== parent) {
            throw new Error('Cordis subagents require the exact live parent Agent');
        }
    }

    function requireScope() {
        if (!scope?.rootId) throw new Error(scope?.error ?? 'Cordis agent tree is unavailable');
    }

    function control(request, signal) {
        requireScope();
        if (signal === undefined) return manager.host('subagent_control', request, manager.signal);
        return manager.host('subagent_control', request, AbortSignal.any([manager.signal, signal]));
    }

    function parseDataUrl(raw) {
        if (typeof raw !== 'string' || !raw.startsWith('data:')) return null;
        const comma = raw.indexOf(',');
        if (comma < 0) return null;
        const header = raw.slice(5, comma);
        if (!header.endsWith(';base64')) return null;
        const mediaType = header.slice(0, -';base64'.length).trim();
        const data = raw.slice(comma + 1);
        return mediaType && data ? { mediaType, data } : null;
    }

    function normalizeContentBlocks(content, label) {
        if (!Array.isArray(content) || !content.length || content.length > 256) {
            throw new TypeError(`${label} requires 1–256 content blocks`);
        }
        const blocks = [];
        for (const block of content) {
            if (!block || typeof block !== 'object') {
                throw new TypeError(`${label} content blocks must be objects`);
            }
            if (block.type === 'text') {
                if (typeof block.text !== 'string') {
                    throw new TypeError(`${label} text blocks require a string`);
                }
                blocks.push({ type: 'text', text: block.text });
                continue;
            }
            if (block.type === 'image' || block.type === 'audio'
                || block.type === 'document' || block.type === 'file') {
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
                    if (!parsed) throw new TypeError(`${label} media URL must be a base64 data URL`);
                    mediaType = mediaType ?? parsed.mediaType;
                    data = parsed.data;
                } else if (block.source && typeof block.source === 'object') {
                    if (block.source.type !== 'base64') {
                        throw new TypeError(`${label} media source must use base64 encoding`);
                    }
                    mediaType = block.source.mediaType ?? block.source.media_type ?? mediaType;
                    data = block.source.data;
                }
                if (ArrayBuffer.isView(data)) {
                    data = Buffer.from(data).toString('base64');
                }
                if (typeof mediaType !== 'string' || !mediaType.trim()
                    || typeof data !== 'string' || !data.trim()) {
                    throw new TypeError(`${label} media blocks require mediaType and base64 data`);
                }
                const filename = block.filename ?? block.name ?? block.title;
                if (filename !== undefined && typeof filename !== 'string') {
                    throw new TypeError(`${label} media filename must be a string`);
                }
                blocks.push({
                    type: block.type,
                    mediaType: mediaType.trim(),
                    data,
                    ...(filename ? { filename } : {}),
                });
                continue;
            }
            throw new TypeError(`${label} contains unsupported content block type ${String(block.type)}`);
        }
        return blocks;
    }

    function freeze(value) {
        if (value !== null && typeof value === 'object') {
            for (const child of Object.values(value)) freeze(child);
            Object.freeze(value);
        }
        return value;
    }

    function validateDepth(depth) {
        if (depth !== undefined && (!Number.isSafeInteger(depth) || depth < 0)) {
            throw new TypeError('maxDepth must be a non-negative safe integer');
        }
        return depth;
    }

    function validateChildId(childId) {
        if (childId === undefined) return undefined;
        if (typeof childId !== 'string' || !childId.length || childId.length > 128
            || childId === 'lead' || !/^[A-Za-z0-9._:-]+$/.test(childId)) {
            throw new TypeError('Invalid Cordis childId');
        }
        return childId;
    }

    function settle(record) {
        if (!record.info || !record.completion) return;
        const { runId, value, error } = record.completion;
        active.delete(runId);
        startedThisCall.delete(runId);
        const stopReason = value?.stopReason ?? 'error';
        manager.emit('subagent/end', freeze({
            ...record.info, stopReason,
            ...(value?.output?.length && {lastAssistantMessage: value.output}),
        }));
        if (error === undefined) record.resolve(freeze(value));
        else record.reject(new Error(error));
    }

    const service = Object.freeze({
        get maxActiveSubagents() { return maxActiveSubagents; },
        capacityOf(caller) {
            if (root === undefined || caller !== root) requireCaller(caller);
            requireScope();
            return {running: scope.activeChildren + startedThisCall.size, limit: maxActiveSubagents};
        },
        getProvider(name) { return providers.get(name); },
        listProviders() { return Array.from(providers.values()); },
        registerProvider(provider) {
            if (!provider || typeof provider !== 'object') {
                throw new TypeError('Cordis subagent provider must be an object');
            }
            const { name, startActivation } = provider;
            if (typeof name !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/.test(name)) {
                throw new TypeError('Invalid Cordis subagent provider name');
            }
            if (typeof startActivation !== 'function') {
                throw new TypeError('Cordis subagent provider requires a startActivation function');
            }
            if (providers.has(name)) {
                throw new Error(`Cordis subagent provider ${name} is already registered`);
            }
            const descriptor = Object.freeze({
                name,
                inheritsParentContext: Boolean(provider.inheritsParentContext),
                capabilities: Object.freeze({ ...(provider.capabilities ?? {}) }),
            });
            providers.set(name, descriptor);
            customProviders.set(name, provider);
            return () => {
                if (customProviders.get(name) === provider) {
                    customProviders.delete(name);
                    providers.delete(name);
                }
            };
        },
        rootOf(caller) {
            if (root === undefined || caller !== root) requireCaller(caller);
            requireScope();
            return root;
        },
        namesOf(caller) {
            if (root !== undefined && caller === root && caller !== parent) return {self: 'lead'};
            requireCaller(caller);
            requireScope();
            return {self: scope.self, ...(scope.parent != null && {parent: scope.parent})};
        },
        listDescendants(parentId, signal) {
            return control({operation: 'list', parentId}, signal);
        },
        resolveName(caller, name, signal) {
            requireCaller(caller);
            return control({operation: 'resolve', name}, signal);
        },
        sendMessage(caller, targetId, content, options = {}) {
            requireCaller(caller);
            const blocks = normalizeContentBlocks(content, 'Cordis subagent message');
            return control({operation: 'message', targetId, blocks}, options?.signal);
        },
        interrupt(targetId, authority) {
            if (authority?.kind !== 'ancestor') throw new Error('Cordis plugins must interrupt as an ancestor Agent');
            requireCaller(authority.agent);
            const request = control({operation: 'interrupt', targetId});
            request.catch(() => {});
            interrupts.push(request);
        },
        modelRoutes(caller, signal) {
            requireCaller(caller);
            return control({operation: 'model_routes'}, signal);
        },
        waitForInput(caller, timeoutMs, signal) {
            requireCaller(caller);
            return control({operation: 'wait', timeoutMs}, signal);
        },
        async waitForTreeWork(caller) {
            if (caller !== parent || caller !== root || root === undefined) return false;
            return control({operation: 'wait_for_tree_work'});
        },
        resolveMaxDepth(configured) {
            return configured === 'provider-managed' ? undefined
                : validateDepth(configured === undefined ? defaultDepth : configured);
        },
        async startActivation(request) {
            requireCaller(request?.parent);
            const {
                provider, prompt, label, name, childId, cwd,
                agentPreset, agentOptions, outputSchema, maxDepth, signal, delivery,
            } = request;
            if (!providers.has(provider)) throw new Error(`Cordis subagent provider ${String(provider)} is unavailable`);
            const validatedChildId = validateChildId(childId);
            if (delivery !== 'parent' && delivery !== 'caller') throw new TypeError('Invalid Cordis result delivery');
            if (typeof label !== 'string' || !label.trim() || label.length > 1024) {
                throw new TypeError('Invalid Cordis child label');
            }
            if (name !== undefined && (typeof name !== 'string' || !/^[a-z][a-z0-9-]{0,31}$/.test(name) || name === 'lead')) {
                throw new TypeError('Invalid Cordis child name');
            }
            if (cwd !== undefined && (typeof cwd !== 'string' || !cwd.length || cwd.length > 4096)) {
                throw new TypeError('Invalid Cordis child cwd');
            }
            const normalizedPrompt = normalizeContentBlocks(prompt, 'Cordis subagent prompt');
            validateDepth(maxDepth);
            // This combination validates the brand of the signal before any
            // host request, and cancels only the startup operation.
            if (signal === undefined) throw new TypeError('Cordis subagent startup requires an AbortSignal');
            const startup = AbortSignal.any([manager.signal, signal]);
            startup.throwIfAborted();
            const runId = uuid();
            const parentSessionId = scope?.selfId ?? parent.id;
            const custom = customProviders.get(provider);
            if (custom !== undefined) {
                const handle = await custom.startActivation({
                    ...request,
                    prompt: normalizedPrompt,
                    ...(validatedChildId !== undefined && { childId: validatedChildId }),
                    runId,
                    signal: startup,
                });
                if (!handle || typeof handle !== 'object' || typeof handle.result?.then !== 'function') {
                    throw new TypeError('Custom Cordis subagent provider must return an activation handle with a result Promise');
                }
                const id = String(handle.childId ?? validatedChildId ?? runId);
                const messageId = handle.messageId !== undefined ? String(handle.messageId) : undefined;
                const visibleName = typeof handle.name === 'string' && handle.name ? handle.name : (name ?? id);
                const info = Object.freeze({ runId, provider, parentSessionId, id, local: false });
                startedThisCall.add(runId);
                manager.emit('subagent/start', info);
                const settledResult = Promise.resolve(handle.result).then(
                    raw => {
                        startedThisCall.delete(runId);
                        const value = freeze(raw ?? { stopReason: 'completed', output: [] });
                        manager.emit('subagent/end', freeze({
                            ...info,
                            stopReason: value?.stopReason ?? 'completed',
                            ...(Array.isArray(value?.output) && value.output.length && { lastAssistantMessage: value.output }),
                        }));
                        return value;
                    },
                    err => {
                        startedThisCall.delete(runId);
                        manager.emit('subagent/end', freeze({ ...info, stopReason: 'error' }));
                        throw err;
                    },
                );
                settledResult.catch(() => {});
                return Object.freeze({
                    childId: id,
                    ...(messageId !== undefined && { messageId }),
                    name: visibleName,
                    result: settledResult,
                    async dispose() {
                        if (typeof handle.dispose === 'function') await handle.dispose();
                    },
                });
            }
            const record = {};
            const result = new Promise((resolve, reject) => Object.assign(record, { resolve, reject }));
            result.catch(() => {});
            active.set(runId, record);
            let accepted;
            try {
                accepted = await manager.host('subagent_start', {
                    runId, provider, prompt: normalizedPrompt, label, name,
                    ...(validatedChildId !== undefined && { childId: validatedChildId }),
                    parentId: parentSessionId,
                    cwd, agentPreset, agentOptions, outputSchema, maxDepth, delivery,
                }, startup);
            } catch (error) {
                active.delete(runId);
                record.reject(error);
                throw error;
            }
            const { childId: id, messageId, name: visibleName } = accepted;
            record.info = Object.freeze({runId, provider, parentSessionId, id, local: true});
            startedThisCall.add(runId);
            manager.emit('subagent/start', record.info);
            settle(record);
            return Object.freeze({
                childId: id, messageId, name: visibleName, result,
                async dispose() {
                    if (active.has(runId)) await manager.host('subagent_dispose', {runId});
                },
            });
        },
    });
    return {
        service,
        enter(agent, currentScope) {
            parent = agent;
            scope = currentScope;
            root = !scope?.rootId ? undefined : (scope.rootId === agent.id || scope.rootId === scope.selfId) ? agent
                : Object.freeze({id: scope.rootId, status: scope.rootStatus});
        },
        async flush() {
            if (interrupts.length) await Promise.all(interrupts.splice(0));
        },
        leave() {
            parent = undefined;
            scope = undefined;
            root = undefined;
            startedThisCall.clear();
            interrupts.length = 0;
        },
        settle(completion) {
            const record = active.get(completion.runId);
            if (record === undefined) return;
            record.completion = completion;
            settle(record);
        },
    };
})
