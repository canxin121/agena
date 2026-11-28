import { defineTool } from '@deepseek-ai/dsh-tools';

export const inject = ['tools', 'subagents'];
const target = {
    type: 'string', required: true,
    description: 'The agent’s name from list_agents, or its session ID.',
};

export function apply(ctx, config) {
    if (config.interrupt !== undefined && typeof config.interrupt !== 'boolean') {
        throw new TypeError('The tool-subagent-control interrupt option must be a boolean');
    }
    ctx.tools.register(defineTool({
        name: 'list_agents',
        description: 'List the root and the continuable children in the current agent tree.',
        parameters: {},
        output: {
            schema: {
                type: 'array', items: { type: 'object', additionalProperties: false, properties: {
                    name: { type: 'string', required: true },
                    status: { type: 'string', required: true, enum: ['running', 'inactive', 'corrupt', 'unsupported', 'unavailable'] },
                    description: { type: 'string' },
                } },
            },
            render: (_args, rows) => [{
                type: 'text',
                text: rows.map(row => `${row.name} [${row.status}]${row.description ? ' — ' + row.description : ''}`).join('\n'),
            }],
        },
        async execute(_args, {agent, signal}) {
            const root = ctx.subagents.rootOf(agent);
            const entries = await ctx.subagents.listDescendants(root.id, signal);
            return [
                {name: 'lead', status: root.status, description: 'root agent'},
                ...entries.map(entry => entry.kind === 'diagnostic'
                    ? {name: entry.path ?? entry.id, status: entry.reason}
                    : {name: entry.path ?? entry.id, status: entry.activity, description: entry.label}),
            ];
        },
    }));
    ctx.tools.register(defineTool({
        name: 'send_message',
        description: 'Send a message to another agent in your tree. An idle agent can start a new turn with the message.',
        parameters: { agent: target, message: { type: 'string', required: true } },
        output: {
            schema: { type: 'object', additionalProperties: false, properties: {
                messageId: { type: 'string', required: true },
            } },
            render: (args, _result) => [{type: 'text', text: `message delivered to agent ${args.agent}`}],
        },
        async execute(args, {agent, signal}) {
            const targetId = await ctx.subagents.resolveName(agent, args.agent, signal);
            const messageId = await ctx.subagents.sendMessage(agent, targetId, [{type: 'text', text: args.message}], {signal});
            return {messageId};
        },
    }));
    ctx.tools.register(defineTool({
        name: 'wait_agent',
        description: 'Wait for pending agent messages, completion notices or queued user input.',
        parameters: { timeout_ms: { type: 'integer', minimum: 10000, maximum: 3600000 } },
        output: {
            schema: { type: 'object', additionalProperties: false, properties: {
                timedOut: { type: 'boolean', required: true },
            } },
            render: (_args, result) => [{type: 'text', text: JSON.stringify(result)}],
        },
        execute(args, {agent, signal}) {
            return ctx.subagents.waitForInput(agent, args.timeout_ms ?? 30000, signal);
        },
    }));
    if (config.interrupt === false) return;
    ctx.tools.register(defineTool({
        name: 'interrupt_agent',
        description: 'Interrupt the current attempt of one descendant in your agent tree.',
        parameters: { agent: target },
        output: {
            schema: { type: 'object', additionalProperties: false, properties: {
                accepted: { type: 'boolean', required: true },
            } },
            render: (args, _result) => [{type: 'text', text: `interrupt requested for agent ${args.agent}`}],
        },
        async execute(args, {agent, signal}) {
            const targetId = await ctx.subagents.resolveName(agent, args.agent, signal);
            ctx.subagents.interrupt(targetId, {kind: 'ancestor', agent});
            return {accepted: true};
        },
    }));
}
