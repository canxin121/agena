(manager => {
    // Section positions exported by @deepseek-ai/dsh-system-prompt.
    const orders = Object.freeze(Object.assign(Object.create(null), {
        HARNESS_IDENTITY: -1000,
        DEPLOYMENT_PERSONA_PREFIX: 0,
        PLAN_POLICY: 500,
        TEAM_POLICY: 600,
        PTC_ONLY: 800,
        FILE_REFERENCE: 900,
        TOOL_BASH: 1000,
        TOOL_PWSH: 1010,
        TOOL_READ: 1100,
        TOOL_WRITE: 1200,
        TOOL_EDIT: 1300,
        TOOL_APPLY_PATCH: 1350,
        TOOL_GLOB: 1400,
        TOOL_GREP: 1500,
        TOOL_JOBS: 1600,
        TOOL_PTY: 1700,
        TOOL_WEB_SEARCH: 2000,
        TOOL_WEB_FETCH: 2100,
        TOOL_LSP: 2200,
        TOOL_SESSION_QUERY: 2300,
        TOOL_GOAL: 2400,
        TOOL_SCHEDULE: 2500,
        TOOL_WORKFLOW: 2600,
        TOOL_RALPH: 2700,
        TOOL_SUBAGENT: 2800,
        TOOL_REPORT: 2900,
        TOOL_COMPUTER_USE: 3000,
        MCP_SERVERS: 3100,
        TOOLS_SDK: 5000,
        DELIVERABLE_FILE_REFERENCES: 9000,
        STRUCTURED_OUTPUT: 9900,
        HARNESS_SOURCE: 10000,
        WEB_SURFACE: 10100,
        DEPLOYMENT_PERSONA_SUFFIX: 10200,
    }));
    const sections = new Map();
    const variables = new Map();
    const put = (entries, name, value) => {
        if (entries.size >= 64 || entries.has(name)) throw new Error(`duplicate or excessive Cordis prompt registrations: ${name}`);
        entries.set(name, value);
        return () => entries.get(name) === value && entries.delete(name);
    };
    const service = Object.freeze({
        getSectionOrder(name) { return orders[name]; },
        section({name, order, text, interpolate, complete}) {
            if (typeof name !== 'string' || !name || name.length > 128 || !Number.isFinite(order)
                || (typeof text !== 'string' && typeof text !== 'function')
                || (interpolate !== undefined && typeof interpolate !== 'boolean')
                || (complete !== undefined && typeof complete !== 'boolean')) {
                throw new TypeError('invalid Cordis prompt section');
            }
            return put(sections, name, {
                name, order, text, interpolate: interpolate !== false, complete: complete === true,
            });
        },
        variable(name, provider) {
            if (typeof name !== 'string' || !/^[a-z][a-z0-9_]*$/.test(name) || name.length > 128
                || typeof provider !== 'function') {
                throw new TypeError('invalid Cordis prompt variable');
            }
            return put(variables, name, provider);
        },
    });
    return {
        service,
        async assemble(prepare) {
            manager.activate();
            try {
                await prepare();
                const context = Object.freeze({ signal: manager.signal });
                let size = 0;
                const checkText = text => {
                    if (typeof text !== 'string' || (size += text.length) > 32768) {
                        throw new Error('Cordis prompt providers must return bounded text');
                    }
                    return text;
                };
                return {
                    sections: Array.from(sections.values(), entry => ({
                        ...entry,
                        text: checkText(typeof entry.text === 'function' ? entry.text(context) : entry.text),
                    })),
                    variables: Array.from(variables, ([name, provider]) => {
                        const value = provider(context);
                        return [name, value === undefined ? null : checkText(value)];
                    }),
                };
            } finally {
                manager.deactivate();
            }
        },
    };
})
