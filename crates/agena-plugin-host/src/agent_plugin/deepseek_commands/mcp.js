((initial, foregroundEffects) => {
    let views = initial?.views ?? [];
    let revision = initial?.revision ?? 0;
    let unavailable = initial?.error;
    const pending = new Map();
    const outgoing = [];
    const listeners = new Map();
    const sessionEvents = [
        'workflow/start', 'workflow/phase', 'workflow/log',
        'workflow/agent-start', 'workflow/agent-end', 'workflow/end',
        'subagent/start', 'subagent/end',
    ];
    const eventNames = new Set(['mcp-manager/status', 'domain/changed', 'skills/change',
        ...sessionEvents]);
    const emit = (event, ...args) => {
        for (const listener of [...(listeners.get(event) ?? [])]) {
            try { Promise.resolve(listener(...args)).catch(() => {}); } catch {}
        }
    };
    let nextId = 0;
    let running = false;
    let controller;

    const signals = new WeakMap();
    const abort = (signal, reason) => {
        const state = signals.get(signal);
        if (state.aborted) return;
        state.aborted = true;
        state.reason = reason;
        state.detach?.();
        state.detach = undefined;
        for (const listener of [...state.listeners]) {
            try { listener.call(signal, { type: 'abort', target: signal }); } catch {}
        }
        state.listeners.clear();
    };
    class AbortSignal {
        constructor() {
            signals.set(this, { aborted: false, reason: undefined, listeners: new Set() });
        }
        get aborted() {
            const state = signals.get(this);
            if (state.aborted) return true;
            const source = state.sources?.find(signal => signal.aborted);
            if (source !== undefined) {
                abort(this, source.reason);
            } else if (state.deadline !== undefined && Date.now() >= state.deadline) {
                abort(this, new Error('The operation timed out'));
            }
            return state.aborted;
        }
        get reason() {
            return this.aborted ? signals.get(this).reason : undefined;
        }
        throwIfAborted() {
            if (this.aborted) throw this.reason;
        }
        addEventListener(type, listener) {
            if (type === 'abort' && typeof listener === 'function') signals.get(this).listeners.add(listener);
        }
        removeEventListener(type, listener) {
            if (type === 'abort') signals.get(this).listeners.delete(listener);
        }
        static abort(reason) {
            const controller = new AbortController();
            controller.abort(reason);
            return controller.signal;
        }
        static timeout(milliseconds) {
            if (!Number.isInteger(milliseconds) || milliseconds < 0 || milliseconds > 2147483647) {
                throw new RangeError('Invalid AbortSignal timeout');
            }
            const signal = new AbortSignal();
            signals.get(signal).deadline = Date.now() + milliseconds;
            return signal;
        }
        static any(iterable) {
            const sources = [...iterable];
            if (sources.some(signal => !signals.has(signal))) {
                throw new TypeError('AbortSignal.any requires AbortSignals');
            }
            const controller = new AbortController();
            const aborted = sources.find(signal => signal.aborted);
            if (aborted !== undefined) {
                controller.abort(aborted.reason);
                return controller.signal;
            }
            const state = signals.get(controller.signal);
            state.sources = sources;
            for (const signal of sources) {
                const deadline = signals.get(signal).deadline;
                if (deadline !== undefined && (state.deadline === undefined || deadline < state.deadline)) {
                    state.deadline = deadline;
                }
            }
            const onAbort = event => controller.abort(event.target.reason);
            for (const signal of sources) signal.addEventListener('abort', onAbort);
            state.detach = () => {
                for (const signal of sources) signal.removeEventListener('abort', onAbort);
                state.sources = undefined;
            };
            return controller.signal;
        }
    }
    class AbortController {
        #signal = new AbortSignal();
        get signal() { return this.#signal; }
        abort(reason = new Error('The operation was aborted')) { abort(this.#signal, reason); }
    }
    globalThis.AbortSignal = AbortSignal;
    globalThis.AbortController = AbortController;

    const copy = value => JSON.parse(JSON.stringify(value));
    const list = () => {
        if (unavailable !== undefined) throw new Error(unavailable);
        return copy(views);
    };
    const updateViews = (next, nextRevision) => {
        if (nextRevision < revision) return;
        const previous = new Map(views.map(view => [view.name, view.status]));
        views = next;
        revision = nextRevision;
        for (const { name, status } of next) {
            if (JSON.stringify(previous.get(name)) === JSON.stringify(status)) continue;
            emit('mcp-manager/status', name, copy(status));
        }
    };
    const host = (service, request, signal) => {
        if (!running || !foregroundEffects()) throw new Error('Cordis host operations require a running command or tool');
        if (signal?.aborted && service !== 'workflow') {
            if (service !== 'mcp' || request.operation !== 'probe') return Promise.reject(signal.reason);
            request.options.aborted = true;
            signal = undefined;
        }
        if (service === 'mcp' && request.operation !== 'watch') {
            const { name } = request;
            if (typeof name !== 'string') throw new TypeError('invalid MCP server name');
            const separator = name.startsWith('plugin:') ? name.lastIndexOf(':') : -1;
            if ((name.startsWith('plugin:') && separator <= 'plugin:'.length)
                || !/^[A-Za-z0-9_-]{1,32}$/.test(name.slice(separator + 1))) {
                throw new TypeError('invalid MCP server name');
            }
        }
        const id = ++nextId;
        if (id > 32) throw new Error('Cordis invocation exceeded its host operation limit');
        const deadline = signals.get(signal)?.deadline;
        const cancel_after_ms = deadline === undefined ? undefined : Math.max(0, deadline - Date.now());
        return new Promise((resolve, reject) => {
            let queued = false;
            const cancel = () => {
                if (queued) return;
                queued = true;
                let reason;
                if (service === 'workflow') {
                    try {
                        const cause = signal.reason;
                        const text = typeof cause === 'string' ? cause : cause?.message;
                        if (typeof text === 'string') reason = text.slice(0, 320);
                    } catch {}
                }
                outgoing.push({ kind: 'cancel', id, reason });
            };
            const release = () => signal?.removeEventListener?.('abort', cancel);
            pending.set(id, { resolve, reject, release });
            outgoing.push({ kind: 'request', id, service, request, cancel_after_ms });
            signal?.addEventListener?.('abort', cancel);
            if (signal?.aborted) cancel();
        });
    };
    const request = (request, signal) => host('mcp', request, signal);
    const on = (event, listener, options = {}) => {
        if (!eventNames.has(event) || typeof listener !== 'function') {
            throw new TypeError(`unsupported Cordis event ${String(event)}`);
        }
        const subscriptions = listeners.get(event) ?? [];
        if (subscriptions.length >= 128) {
            throw new Error('Cordis module exceeded its event listener limit');
        }
        listeners.set(event, subscriptions);
        const callback = (...args) => listener(...args);
        if (options === true || options?.prepend === true) {
            subscriptions.unshift(callback);
        } else {
            subscriptions.push(callback);
        }
        return () => {
            const index = subscriptions.indexOf(callback);
            if (index < 0) return false;
            subscriptions.splice(index, 1);
            return true;
        };
    };
    const once = (event, listener, options) => {
        if (typeof listener !== 'function') throw new TypeError('invalid Cordis status listener');
        const dispose = on(event, (...args) => {
            if (dispose()) return listener(...args);
        }, options);
        return dispose;
    };

    return {
        get active() { return running && foregroundEffects(); },
        activate() {
            running = true;
            controller = new AbortController();
        },
        deactivate() {
            running = false;
            controller.abort();
            for (const { reject, release } of pending.values()) {
                release();
                reject(new Error('The Cordis command ended'));
            }
            pending.clear();
            outgoing.length = 0;
            nextId = 0;
        },
        update(snapshot) {
            unavailable = snapshot?.error;
            if (snapshot?.views !== undefined) updateViews(snapshot.views, snapshot.revision);
        },
        on,
        once,
        emit,
        observesSessionEvents: () => sessionEvents.some(event => listeners.get(event)?.length > 0),
        sessionEvent({name, args}) {
            const freeze = value => {
                if (value !== null && typeof value === 'object') {
                    for (const nested of Object.values(value)) freeze(nested);
                    Object.freeze(value);
                }
                return value;
            };
            emit(name, ...args.map(freeze));
        },
        host,
        hasPendingRequests: () => pending.size > 0,
        abortDelay(signal) {
            if (signal === undefined) return undefined;
            const state = signals.get(signal);
            if (!state) throw new TypeError('a Cordis wait requires an AbortSignal');
            if (state.deadline === undefined) return undefined;
            return Math.max(0, state.deadline - Date.now());
        },
        get signal() { return controller.signal; },
        service(register) {
            return Object.freeze({
                register,
                list,
                async *watch(signal) {
                    let previous = list();
                    while (!signal?.aborted) {
                        yield copy(previous);
                        if (signal?.aborted) return;
                        const snapshot = await request({ operation: 'watch', previous }, signal);
                        if (snapshot === null) return;
                        previous = snapshot.views;
                        updateViews(previous, snapshot.revision);
                    }
                },
                async restart(name) {
                    const reply = await request({ operation: 'restart', name });
                    updateViews(views.map(view => view.name === reply.view.name ? reply.view : view), reply.revision);
                },
                async probe(name, options = {}) {
                    const { timeoutMs, maxPages, maxToolNames, signal } = options;
                    if (timeoutMs !== undefined && !(Number.isFinite(timeoutMs) && timeoutMs > 0)) {
                        throw new TypeError('MCP probe timeoutMs must be a positive finite number');
                    }
                    for (const value of [maxPages, maxToolNames]) {
                        if (value !== undefined && !(Number.isSafeInteger(value) && value > 0)) {
                            throw new TypeError('MCP probe listing limits must be positive safe integers');
                        }
                    }
                    const probeOptions = { timeoutMs, maxPages, maxToolNames, aborted: false };
                    return request({ operation: 'probe', name, options: probeOptions }, signal);
                },
            });
        },
        requests(allowJobWait) {
            if (!pending.size && !allowJobWait) throw new Error('Cordis command is waiting on unavailable host work');
            return outgoing.splice(0);
        },
        resume({ id, value, error }) {
            const operation = pending.get(id);
            if (operation === undefined) return;
            const { resolve, reject, release } = operation;
            pending.delete(id);
            release();
            if (error === undefined) resolve(value);
            else reject(new Error(error));
        },
    };
})
