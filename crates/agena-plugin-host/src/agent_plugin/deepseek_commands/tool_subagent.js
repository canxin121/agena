import { defineTool } from '@deepseek-ai/dsh-tools';

export const inject = ['tools', 'subagents', 'systemPrompt'];

function normalizeRoutes(payload) {
    if (Array.isArray(payload?.routes)) return payload.routes;
    if (payload?.provider && payload?.model) return [payload];
    return [];
}

function registerModelDiscovery(ctx) {
    ctx.tools.register(defineTool({
        name: 'list_subagent_models',
        description: 'Inspect the available subagent model providers, models, and reasoning efforts.',
        parameters: { provider: { type: 'string' }, model: { type: 'string' } },
        output: {
            schema: { type: 'string' },
            render: (_args, result) => [{type: 'text', text: result}],
        },
        async execute({provider, model}, exec) {
            if (model !== undefined && provider === undefined) throw new Error('model requires provider');
            const routes = normalizeRoutes(await ctx.subagents.modelRoutes(exec.agent, exec.signal));
            if (provider === undefined) {
                return Array.from(new Set(routes.map(route => route.provider))).join('\n');
            }
            const matching = routes.filter(route => route.provider === provider);
            if (!matching.length) {
                throw new Error('This child model route is unavailable for the session');
            }
            if (model === undefined) {
                return matching
                    .map(route => `${route.provider}/${route.model}${route.description ? ' — ' + route.description : ''}`)
                    .join('\n');
            }
            const route = matching.find(candidate => candidate.model === model);
            if (!route) {
                throw new Error('This child model route is unavailable for the session');
            }
            const line = `${route.provider}/${route.model}${route.description ? ' — ' + route.description : ''}`;
            const efforts = (route.efforts ?? []).map(({id, name, default: isDefault, description}) =>
                `${id}${isDefault ? ' (default)' : ''} — ${name}${description ? ': ' + description : ''}`);
            return `${line}\nReasoning efforts:\n${efforts.join('\n') || '(no advertised reasoning efforts)'}`;
        },
    }));
}

export function apply(ctx, config) {
    const {
        provider = 'spawn', toolName = 'subagent', maxDepth, agentOptions,
        agentPreset, allowedAgentPresets, modelSelection, modelDiscovery,
    } = config;
    if ('persona' in config || 'toolFilter' in config) {
        throw new Error('Configure a child preset through agentPreset instead of persona or toolFilter');
    }
    if (typeof provider !== 'string' || !provider || typeof toolName !== 'string'
        || (agentPreset !== undefined && (typeof agentPreset !== 'string' || !agentPreset))
        || (modelSelection !== undefined && typeof modelSelection !== 'boolean')
        || (modelDiscovery !== undefined && typeof modelDiscovery !== 'boolean')) {
        throw new TypeError('Invalid tool-subagent configuration');
    }
    if (allowedAgentPresets !== undefined && (!Array.isArray(allowedAgentPresets)
        || !allowedAgentPresets.length || allowedAgentPresets.some(id => typeof id !== 'string' || !id)
        || new Set(allowedAgentPresets).size !== allowedAgentPresets.length
        || (agentPreset !== undefined && !allowedAgentPresets.includes(agentPreset)))) {
        throw new TypeError('Invalid tool-subagent allowedAgentPresets');
    }
    const depth = ctx.subagents.resolveMaxDepth(maxDepth);
    if (modelDiscovery === true) registerModelDiscovery(ctx);
    const selected = ctx.subagents.getProvider(provider);
    if (selected === undefined) return;
    if (!selected.capabilities.agentPreset && (agentPreset !== undefined || allowedAgentPresets !== undefined)) {
        throw new Error(`Subagent provider "${provider}" cannot select an independent preset`);
    }
    const definition = defineTool({
        name: toolName,
        description: selected.inheritsParentContext
            ? 'Start a subagent with the completed turns of this conversation. Returns its name after acceptance and delivers the result to this session when it finishes.'
            : 'Start a subagent with an independent context. Returns its name after acceptance and delivers the result to this session when it finishes.',
        parameters: {
            cwd: { type: 'string', description: 'Initial child working directory.' },
            description: { type: 'string', required: true, description: 'Short label for the delegated task.' },
            name: { type: 'string', description: 'Optional name starting with a lowercase letter, with up to 32 lowercase letters, digits or hyphens.' },
            prompt: { type: 'string', required: true, description: 'Task to give the subagent.' },
            ...(allowedAgentPresets && { agent_preset: { type: 'string', enum: allowedAgentPresets } }),
            ...(modelSelection && {
                provider: { type: 'string', description: 'Child provider id. Supply together with model.' },
                model: { type: 'string', description: 'Child model id. Supply together with provider.' },
                reasoning_effort: { type: 'string', description: 'Child reasoning effort. Supply with provider and model.' },
            }),
        },
        output: {
            schema: {
                type: 'object', additionalProperties: false,
                properties: {
                    kind: { type: 'string', required: true, const: 'activation' },
                    name: { type: 'string', required: true },
                },
            },
            render: (_args, result) => [{type: 'text', text: `started subagent ${result.name}`}],
        },
        async execute(args, {agent, signal}) {
            let options = agentOptions;
            if (args.provider !== undefined || args.model !== undefined || args.reasoning_effort !== undefined) {
                if (!modelSelection || !args.provider || !args.model) {
                    throw new Error('Explicit child model selection requires provider and model');
                }
                const routes = normalizeRoutes(await ctx.subagents.modelRoutes(agent, signal));
                const route = routes.find(candidate =>
                    candidate.provider === args.provider && candidate.model === args.model);
                if (!route) {
                    throw new Error(`Child route "${args.provider}/${args.model}" is unavailable for this session`);
                }
                if (args.reasoning_effort !== undefined
                    && !(route.efforts ?? []).some(effort => effort.id === args.reasoning_effort)) {
                    throw new Error('The selected model does not support that reasoning effort');
                }
                options = {...options, provider: args.provider, model: args.model, reasoningEffort: args.reasoning_effort};
            }
            const child = await ctx.subagents.startActivation({
                provider, parent: agent, signal, name: args.name, label: args.description,
                cwd: args.cwd, prompt: [{type: 'text', text: args.prompt}], delivery: 'parent',
                agentPreset: args.agent_preset ?? agentPreset, agentOptions: options, maxDepth: depth,
            });
            return {kind: 'activation', name: child.name};
        },
    });
    ctx.tools.register(definition);
    ctx.systemPrompt.section({
        name: `tool:${toolName}`, order: ctx.systemPrompt.getSectionOrder('TOOL_SUBAGENT'), interpolate: false,
        text: `Use ${toolName} to start background delegation. The direct creator receives a completion message with the child's result.`,
    });
}
