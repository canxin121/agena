((manager, uuid, maxAgents, jobPolicy, providers) => {
    let parent;
    function requireParent(value) {
        if (parent === undefined || value !== parent) {
            throw new Error('Cordis workflow requires the current tool caller');
        }
    }
    function prepare(request) {
        requireParent(request?.parent);
        const { script, meta, args, subagentProvider, maxTotalAgents } = request;
        if (typeof script !== 'string' || !script.trim() || script.length > 262144
            || typeof meta?.name !== 'string' || !/^[a-z0-9_-]{1,64}$/.test(meta.name)
            || typeof meta.description !== 'string' || !meta.description.trim()
            || meta.description.length > 1024) {
            throw new TypeError('Invalid Cordis workflow script or metadata');
        }
        if (subagentProvider !== undefined && !providers.includes(subagentProvider)) {
            throw new TypeError(`Cordis workflow subagent provider ${String(subagentProvider)} is unavailable`);
        }
        if (maxTotalAgents !== undefined && (!Number.isSafeInteger(maxTotalAgents)
            || maxTotalAgents < 1 || maxTotalAgents > maxAgents)) {
            throw new TypeError(`maxTotalAgents must be between 1 and ${maxAgents}`);
        }
        const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
        new AsyncFunction(script);
        return { id: uuid(), script, meta: JSON.parse(JSON.stringify(meta)), args, subagentProvider, maxTotalAgents };
    }
    function requireJobs() {
        if (jobPolicy === null) throw new Error('Cordis workflow job controller is unavailable');
    }
    const service = Object.freeze({
        start(request) {
            const workflow = prepare(request);
            const info = { id: workflow.id, meta: workflow.meta };
            const controller = new AbortController();
            const activeSignal = AbortSignal.any([controller.signal, manager.signal, ...(request.signal === undefined ? [] : [request.signal])]);
            let settled = false;
            const result = manager.host('workflow', workflow, activeSignal).catch(error => ({
                stopReason: activeSignal.aborted ? 'cancelled' : 'error',
                error: error?.message ?? String(error), agentsStarted: 0, value: null,
            })).then(outcome => {
                settled = true;
                controller.abort();
                const { stopReason, error, agentsStarted } = outcome;
                const detail = error === undefined ? { stopReason, agentsStarted } : { stopReason, error, agentsStarted };
                manager.sessionEvent({name: 'workflow/end', args: [info, detail]});
                return outcome;
            });
            manager.sessionEvent({name: 'workflow/start', args: [info]});
            return Object.freeze({
                ...info, result,
                cancel(reason) { if (!settled) controller.abort(new Error(reason ?? 'Workflow cancelled')); },
                async dispose() {
                    if (!settled) controller.abort(new Error('Workflow disposed'));
                    await result;
                },
            });
        },
        async startBackground(request, maxResultChars = 50000) {
            requireJobs();
            if (!Number.isSafeInteger(maxResultChars) || maxResultChars < 1) {
                throw new TypeError('maxResultChars must be a positive integer');
            }
            const workflow = prepare(request);
            const signal = AbortSignal.any([manager.signal, ...(request.signal === undefined ? [] : [request.signal])]);
            return manager.host('workflow_background', { workflow, maxResultChars }, signal);
        },
        job(request, exec) {
            requireParent(exec?.agent);
            requireJobs();
            if (request.action === 'output') {
                const { job_id, wait, timeout_ms } = request;
                const timeout = timeout_ms ?? jobPolicy.waitTimeoutMs;
                if (wait === true && (!Number.isFinite(timeout) || timeout < 1)) {
                    throw new TypeError('timeout_ms must be positive');
                }
                request = {
                    action: 'output', job_id,
                    ...(wait === true && { wait_ms: Math.trunc(Math.min(timeout, jobPolicy.maxWaitTimeoutMs)) }),
                };
            }
            return manager.host('workflow_job', request, exec.signal);
        },
    });
    return {
        service,
        enter(sessionId, status) { parent = Object.freeze({ id: String(sessionId), status }); return parent; },
        leave() { parent = undefined; },
    };
})
