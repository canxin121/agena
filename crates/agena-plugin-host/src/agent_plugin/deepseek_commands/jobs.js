((startNative, wake, queryNative, cancelWait, setControllerCount, manager, pollMs) => {
    const jobs = new Map();
    const timers = new Map();
    let serial = 0;
    let caller;
    let current;
    const listeners = new Set();
    const controllers = new Set();
    const waits = new Map();
    let nextWait = 0;

    const query = (request, caller) => JSON.parse(queryNative(JSON.stringify(request), caller, undefined, undefined));
    const jobId = id => {
        if (typeof id !== 'string' || !id || id.length > 256) throw new TypeError('Invalid Cordis job ID');
        return id;
    };

    const freeze = value => {
        if (value !== null && typeof value === 'object') {
            for (const child of Object.values(value)) freeze(child);
            Object.freeze(value);
        }
        return value;
    };

    function emit(event) {
        const immutable = freeze(event);
        const owner = event.job?.owner ?? event.owner;
        for (const subscription of [...listeners]) {
            if (subscription.owner !== undefined && subscription.owner !== owner) continue;
            try { Promise.resolve(subscription.listener(immutable)).catch(() => {}); } catch {}
        }
    }

    const withJob = (job, callback) => {
        const previous = current;
        current = job;
        try {
            return callback();
        } finally {
            current = previous;
        }
    };

    function detach(job) {
        jobs.delete(job.id);
        for (const [id, timer] of timers) {
            if (timer.job === job || jobs.size === 0) {
                timers.delete(id);
                timer.dispose?.();
            }
        }
    }

    function write(job, text, channel, gapBefore) {
        if (jobs.has(job.id) && typeof text === 'string' && text.length) {
            job.publish(JSON.stringify({
                type: 'append', text,
                channel: ['stdout', 'stderr', 'log'].includes(channel) ? channel : undefined,
                gap_before: gapBefore === true,
            }));
        }
    }

    function pull(job) {
        if (!job.sources.length) return;
        const deltas = [];
        for (const source of job.sources) {
            if (source.read === undefined) continue;
            try {
                const { text, nextOffset, lossy, spillPath } = source.read(source.offset);
                source.spillPath = typeof spillPath === 'string' && spillPath.length <= 4096
                    ? spillPath : undefined;
                if (typeof text === 'string' && Number.isSafeInteger(nextOffset)
                    && nextOffset >= source.offset && (nextOffset !== source.offset || text.length === 0)) {
                    source.offset = nextOffset;
                    if (text.length) deltas.push([text, source.channel, lossy]);
                }
            } catch (error) {
                source.read = undefined;
                source.spillPath = undefined;
                deltas.push([String(error), 'log']);
            }
        }
        const paths = [...new Set(job.sources.map(source => source.spillPath).filter(Boolean))];
        if (paths.length !== job.spillPaths.length || paths.some((path, index) => path !== job.spillPaths[index])) {
            job.spillPaths = paths;
            job.publish(JSON.stringify({type: 'spill_paths', paths}));
        }
        for (const delta of deltas) write(job, ...delta);
        job.nextPoll = Date.now() + pollMs;
    }

    function settle(job, outcome) {
        if (!jobs.has(job.id)) return;
        withJob(job, () => pull(job));
        try {
            const { status, detail, result } = outcome;
            if (!['completed', 'failed', 'killed'].includes(status)
                || (detail !== undefined && typeof detail !== 'string')
                || (result !== undefined && typeof result !== 'string')) {
                throw new TypeError('Invalid Cordis job outcome');
            }
            job.resolve(JSON.stringify({ status, detail, result }));
        } catch (error) {
            job.resolve(JSON.stringify({ status: 'failed', detail: String(error) }));
        } finally {
            detach(job);
        }
    }

    function requestStop(job, reason) {
        if (job.stopping) return;
        job.stopping = true;
        try {
            withJob(job, () => job.cancel(reason));
        } catch (error) {
            settle(job, { status: 'failed', detail: String(error) });
        }
    }

    const service = Object.freeze({
        attachController(name) {
            if (typeof name !== 'string') throw new TypeError('Cordis job controller name must be a string');
            if (controllers.size >= 128) throw new Error('Cordis job controller limit reached');
            const token = Symbol(name);
            controllers.add(token);
            setControllerCount(controllers.size);
            return () => {
                if (controllers.delete(token)) setControllerCount(controllers.size);
            };
        },
        list(caller) { return query({action: 'list'}, caller); },
        get(id, caller) { return query({action: 'get', id: jobId(id)}, caller); },
        read(id, caller) { return query({action: 'read', id: jobId(id)}, caller); },
        readAt(id, from, caller) {
            if (!Number.isSafeInteger(from) || from < 0) {
                throw new RangeError('Cordis job output offset must be a non-negative safe integer');
            }
            return query({action: 'read_at', id: jobId(id), from}, caller);
        },
        kill(id, caller, reason) {
            if (reason !== undefined && typeof reason !== 'string') {
                throw new TypeError('Cordis job cancellation reason must be a string');
            }
            return query({action: 'kill', id: jobId(id), reason}, caller);
        },
        remove(id, caller) {
            query({action: 'remove', id: jobId(id)}, caller);
        },
        async wait(id, timeoutMs, caller, signal) {
            if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
                throw new RangeError('Cordis job wait timeout must be positive and finite');
            }
            const timeout_ms = Math.ceil(Math.min(timeoutMs, 600000));
            const pre_aborted = signal?.aborted === true;
            const cancelMs = manager.abortDelay(signal);
            const request = {action: 'wait', id: jobId(id), timeout_ms, pre_aborted};
            const operation = ++nextWait;
            return new Promise((resolve, reject) => {
                const cancel = () => cancelWait(operation);
                const release = () => signal?.removeEventListener('abort', cancel);
                queryNative(JSON.stringify(request), caller, operation, cancelMs);
                waits.set(operation, {resolve, reject, release, signal});
                if (!pre_aborted) signal?.addEventListener('abort', cancel);
            });
        },
        events: Object.freeze({
            subscribe(filter, listener) {
                const owner = filter?.owner;
                const owners = filter?.owners;
                if (typeof listener !== 'function' || (owner === undefined
                    ? !['all', 'scope'].includes(owners) : typeof owner !== 'string' || owners !== undefined)) {
                    throw new TypeError('Invalid Cordis job event subscription');
                }
                if (listeners.size >= 128) throw new Error('Cordis job listener limit reached');
                const subscription = {owner, listener};
                listeners.add(subscription);
                return () => listeners.delete(subscription);
            },
        }),
        start(spec) {
            if (caller === undefined || current !== undefined) {
                throw new Error('Cordis jobs.start requires the creating tool or command invocation');
            }
            if (!spec || typeof spec.kind !== 'string' || typeof spec.label !== 'string'
                || typeof spec.run !== 'function') {
                throw new TypeError('Cordis jobs.start requires kind, label and run');
            }
            const { kind, label, owner, toolCall, outputLimitBytes, output } = spec;
            if (owner !== undefined && owner !== caller.owner) {
                throw new Error('Cordis job owner must be the current project session');
            }
            if (toolCall !== undefined && (caller.callId === undefined || toolCall?.callId !== caller.callId
                || toolCall?.rootCallId !== caller.callId)) {
                throw new Error('Cordis job toolCall must identify the creating tool call');
            }
            if (outputLimitBytes !== undefined
                && (!Number.isSafeInteger(outputLimitBytes) || outputLimitBytes <= 0)) {
                throw new RangeError('Cordis job outputLimitBytes must be a positive safe integer');
            }
            if (output !== undefined && (!Array.isArray(output) || output.length > 16)) {
                throw new TypeError('Cordis job output requires an array of up to sixteen sources');
            }
            const sources = (output ?? []).map(source => {
                if (typeof source?.read !== 'function') throw new TypeError('Invalid Cordis job output source');
                return { read: source.read.bind(source), channel: source.channel, offset: 0 };
            });
            let job;
            try {
                const id = startNative(JSON.stringify({ kind, label, outputLimitBytes }), (id, publish, resolve) => {
                    job = { id, publish, resolve, sources, spillPaths: [], nextPoll: Date.now() };
                    jobs.set(id, job);
                    const handle = Object.freeze({
                        id,
                        append(text, options) {
                            write(job, text, options?.channel, options?.gapBefore);
                        },
                        updateProgress(line) {
                            if (jobs.has(id) && typeof line === 'string') {
                                publish(JSON.stringify({ type: 'progress', line }));
                            }
                        },
                    });
                    const hooks = withJob(job, () => spec.run(handle));
                    if (typeof hooks?.cancel === 'function') job.cancel = hooks.cancel.bind(hooks);
                    if (!job.cancel || typeof hooks.done?.then !== 'function') {
                        throw new TypeError('Cordis job run must return cancel and a done promise');
                    }
                    Promise.resolve(hooks.done).then(
                        outcome => settle(job, outcome),
                        error => settle(job, { status: 'failed', detail: String(error) }),
                    );
                });
                wake();
                return id;
            } catch (error) {
                if (job !== undefined) {
                    if (job.cancel) requestStop(job, 'Cordis job registration failed');
                    detach(job);
                }
                throw error;
            }
        },
    });

    function schedule(callback, delay, args, dispose) {
        if (typeof callback !== 'function') throw new TypeError('Cordis timer requires a callback');
        if (jobs.size === 0) throw new Error('Cordis timers require a live background job');
        if (timers.size >= 64) throw new Error('Cordis job timer limit reached');
        delay = Number(delay);
        if (!Number.isFinite(delay) || delay < 1 || delay > 2147483647) delay = 1;
        const id = ++serial;
        const job = current ?? (jobs.size === 1 ? jobs.values().next().value : undefined);
        timers.set(id, { callback, args, job, due: Date.now() + Math.trunc(delay), dispose });
        wake();
        return id;
    }

    const clearTimeout = id => {
        const timer = timers.get(id);
        if (timers.delete(id)) timer.dispose?.();
    };
    globalThis.setTimeout = (callback, delay, ...args) => schedule(callback, delay, args);
    globalThis.clearTimeout = clearTimeout;
    globalThis.agenaCordisJobDelay = (delay, value, { signal } = {}) => new Promise((resolve, reject) => {
        signal?.throwIfAborted();
        const abort = () => {
            clearTimeout(id);
            reject(signal.reason);
        };
        const id = schedule(() => resolve(value), delay, [], () => signal?.removeEventListener('abort', abort));
        signal?.addEventListener('abort', abort);
    });

    return {
        service,
        enter(owner, callId) { caller = { owner, callId }; },
        leave() { caller = undefined; },
        pullProducer(id) {
            const job = jobs.get(id);
            if (job !== undefined) withJob(job, () => pull(job));
        },
        cancelProducer(id, reason) {
            const job = jobs.get(id);
            if (job === undefined) return false;
            withJob(job, () => job.cancel(reason ?? undefined));
            return true;
        },
        tick(controls, events) {
            for (const event of events) emit(event);
            for (const { id, action, reason } of controls) {
                const job = jobs.get(id);
                if (job === undefined) continue;
                if (action === 'cancel') requestStop(job, reason ?? undefined);
                if (action === 'abandon') detach(job);
            }
            const now = Date.now();
            const nextTimer = [...timers].find(([, timer]) => timer.due <= now);
            if (nextTimer !== undefined) {
                const [id, timer] = nextTimer;
                clearTimeout(id);
                // Keep ownership through the microtasks unblocked by this timer.
                current = timer.job;
                try {
                    Promise.resolve(timer.callback(...timer.args)).catch(error => {
                        if (timer.job) settle(timer.job, { status: 'failed', detail: String(error) });
                    });
                } catch (error) {
                    if (timer.job) settle(timer.job, { status: 'failed', detail: String(error) });
                }
                return;
            }
            for (const job of jobs.values()) {
                if (job.sources.length && job.nextPoll <= now) withJob(job, () => pull(job));
            }
        },
        endTick() {
            current = undefined;
        },
        resolveWaits(replies) {
            for (const {id, value, error} of replies) {
                const wait = waits.get(id);
                if (!wait) continue;
                waits.delete(id);
                wait.release();
                if (error === null) wait.resolve(value);
                else wait.reject(wait.signal?.aborted ? wait.signal.reason : new Error(error));
            }
        },
        schedule() {
            let next = Infinity;
            for (const timer of timers.values()) next = Math.min(next, timer.due);
            for (const job of jobs.values()) {
                if (job.sources.length) next = Math.min(next, job.nextPoll);
            }
            return {live: jobs.size > 0 || waits.size > 0,
                delayMs: Number.isFinite(next) ? Math.max(1, next - Date.now()) : null};
        },
    };
})
