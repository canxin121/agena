import { defineTool } from '@deepseek-ai/dsh-tools';

export const inject = ['tools', 'workflowEngine', 'systemPrompt'];

const job = {
    type: 'object', additionalProperties: false, properties: {
        id: { type: 'string', required: true },
        kind: { type: 'string', required: true },
        label: { type: 'string', required: true },
        status: { type: 'string', required: true, enum: ['running', 'stopping', 'completed', 'killed', 'failed'] },
        detail: { type: 'string' },
        progress: { type: 'string' },
        owner: { type: 'string' },
        outputLimitBytes: { type: 'integer' },
        startedAt: { type: 'integer', required: true },
        finishedAt: { type: 'integer' },
    },
};
const jobId = { type: 'string', required: true, description: 'ID returned by the tool that started the background job.' };
const text = value => [{ type: 'text', text: value }];
const status = value => `[status: ${value.status}${value.detail ? `, ${value.detail}` : ''}]`;

export function apply(ctx) {
    ctx.systemPrompt.section({
        name: 'tool:job_list',
        order: ctx.systemPrompt.getSectionOrder('TOOL_JOBS'),
        text: 'Track the IDs of your background jobs. Use job_output to collect results, and job_kill to cancel work you no longer need.',
    });
    ctx.tools.register(defineTool({
        name: 'job_list',
        description: 'List the background jobs in this session.',
        parameters: {},
        output: {
            schema: { type: 'object', additionalProperties: false, properties: {
                jobs: { type: 'array', items: job, required: true },
            } },
            render: (_args, { jobs }) => text(jobs.length ? jobs.map(job =>
                `${job.id} [${job.kind}] ${job.status} — ${job.label}${job.progress ? ' · ' + job.progress : ''}`).join('\n') : '(no background jobs)'),
        },
        async execute(_args, exec) {
            return { jobs: await ctx.workflowEngine.job({ action: 'list' }, exec) };
        },
    }));
    ctx.tools.register(defineTool({
        name: 'job_output',
        description: 'Read the progress and final result of a background job.',
        parameters: {
            job_id: jobId,
            wait: { type: 'boolean', description: 'Wait until the job settles or the timeout expires.' },
            timeout_ms: { type: 'number', description: 'Maximum time to wait in milliseconds.' },
        },
        output: {
            schema: { type: 'object', additionalProperties: false, properties: {
                text: { type: 'string', required: true }, job: { ...job, required: true },
            } },
            render: (_args, value) => text(`${value.text.trimEnd() || '(no new output)'}\n${status(value.job)}`),
        },
        execute: (args, exec) => ctx.workflowEngine.job({ ...args, action: 'output' }, exec),
    }));
    ctx.tools.register(defineTool({
        name: 'job_kill',
        description: 'Request cancellation of a background job.',
        parameters: {
            job_id: jobId,
            reason: { type: 'string', description: 'Optional cancellation reason.' },
        },
        output: {
            schema: { type: 'object', additionalProperties: false, properties: {
                outcome: { type: 'string', required: true, enum: ['cancellation-requested', 'already-finished'] },
                job: { ...job, required: true },
            } },
            render: (_args, value) => text(value.outcome === 'already-finished'
                ? `job ${value.job.id} had already finished ${status(value.job)}`
                : `requested cancellation of job ${value.job.id}`),
        },
        execute: (args, exec) => ctx.workflowEngine.job({ ...args, action: 'kill' }, exec),
    }));
}
