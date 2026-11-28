import { defineTool } from '@deepseek-ai/dsh-tools';

export const inject = ['tools', 'workflowEngine', 'systemPrompt'];

const foreground = {
    type: 'object', additionalProperties: false, properties: {
        kind: { type: 'string', const: 'foreground', required: true },
        runId: { type: 'string', required: true },
        agentsStarted: { type: 'integer', required: true },
        result: { type: 'json', required: true },
    },
};
const background = {
    type: 'object', additionalProperties: false, properties: {
        kind: { type: 'string', const: 'background', required: true },
        runId: { type: 'string', required: true },
        jobId: { type: 'string', required: true },
    },
};

export function apply(ctx, config) {
    const { toolName = 'workflow', maxResultChars = 50000, enableRunInBackground = true } = config;
    if (!Number.isSafeInteger(maxResultChars) || maxResultChars < 1) {
        throw new TypeError('maxResultChars must be a positive integer');
    }
    if (typeof enableRunInBackground !== 'boolean') {
        throw new TypeError('enableRunInBackground must be a boolean');
    }
    ctx.systemPrompt.section({
        name: `tool:${toolName}`,
        order: ctx.systemPrompt.getSectionOrder('TOOL_WORKFLOW'),
        text: `Use ${toolName} when the user asks for a workflow or large multi-agent orchestration.`,
    });
    ctx.tools.register(defineTool({
        name: toolName,
        description: 'Run a JavaScript workflow using agent(prompt, options), pipeline(items, ...stages), parallel(thunks), phase(title), log(message), and args. Return a JSON value.',
        parameters: {
            script: { type: 'string', required: true },
            meta: { type: 'object', required: true, additionalProperties: true, properties: {
                name: { type: 'string', required: true },
                description: { type: 'string', required: true },
            } },
            args: { type: 'object', additionalProperties: true },
            ...(enableRunInBackground && {
                run_in_background: { type: 'boolean', description: 'Return a job ID immediately. Use job_output to read progress and the result.' },
            }),
        },
        output: {
            schema: enableRunInBackground ? { oneOf: [foreground, background] } : foreground,
            render(args, value) {
                if (value.kind === 'background') {
                    return [{ type: 'text', text: `workflow "${args.meta.name}" started as background job ${value.jobId}.` }];
                }
                const text = JSON.stringify(value.result, null, 2);
                const preview = text.length <= maxResultChars
                    ? text : `${text.slice(0, maxResultChars)}\n… [truncated: ${text.length - maxResultChars} more characters]`;
                return [{ type: 'text', text: `workflow "${args.meta.name}" completed (${value.agentsStarted} agents).\nReturn value:\n${preview}` }];
            },
        },
        async execute(args, exec) {
            const request = { ...args, parent: exec.agent, signal: exec.signal };
            if (args.run_in_background === true) {
                if (!enableRunInBackground) throw new Error('This workflow tool has background runs disabled');
                return ctx.workflowEngine.startBackground(request, maxResultChars);
            }
            const run = ctx.workflowEngine.start(request);
            try {
                const { stopReason, error, agentsStarted, value } = await run.result;
                if (stopReason !== 'completed') throw new Error(error ?? `Workflow ${stopReason}`);
                return { kind: 'foreground', runId: run.id, agentsStarted, result: value };
            } finally {
                await run.dispose();
            }
        },
    }));
}
