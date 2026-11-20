# Model performance and usage

Agena records client-observed performance for completed provider requests and
persists it alongside token and cost usage. The Web context popover and terminal
status show the latest request. Usage reports aggregate measurements by day,
provider, model, and session, using the report's existing filters and timezone.

## Reading the metrics

| Metric | Meaning |
| --- | --- |
| First output latency (TTFT) | Time from starting a provider request to its first non-empty text, reasoning, or tool-call output. |
| Request duration | Time from starting the request through its completed response, including the terminal usage response. |
| Generation duration | Time between the first and last effective output observed in a stream. |
| Generation speed (TPS) | Provider-reported output **plus reasoning** tokens divided by generation duration, in seconds. |
| End-to-end speed | The same token count divided by the complete request duration, in seconds. |
| P50 / P95 duration | The nearest-rank median / 95th percentile of observed request durations or first-output latencies. |

These intervals use a monotonic clock at Agena's provider boundary. They measure
the request as observed by the runtime, including network and provider queueing
time. Local tool execution and the waits between provider attempts are outside
the intervals. A provider-hosted tool running inside one request remains part of
that request's elapsed time.

Empty chunks, metadata, retry notifications, repeated final tool snapshots, and
the terminal usage event do not count as newly generated output. A delayed usage
response therefore increases request duration without reducing generation TPS.
Reasoning-only and tool-call-only streams can still produce measurements.

Generation speed requires at least two reported generated tokens, a positive
generation interval, and consistent first-output/request timing. A one-chunk
stream has no measurable generation interval. Non-streaming requests can report
request duration and end-to-end speed; their first-output and generation timings
remain unknown. Unknown values appear as `—`, while a measured zero-millisecond
latency remains zero.

## Aggregation and sample counts

Generation speed is calculated as:

```text
sum(generated_tokens for valid generation samples)
------------------------------------------------- × 1000
sum(generation_ms for those same samples)
```

End-to-end speed uses the equivalent calculation over requests with known token
usage and a positive request duration. The report never averages individual TPS
values, and it never adds historical tokens without timing to a rate's
numerator.

Average latency uses the sum of measured durations divided by its measurement
count. Percentiles sort the individual measurements and select rank
`ceil(percentile × count / 100)`. Each metric's sample count can differ:

- Request duration can be measured even when the provider omits token usage.
- First-output latency requires effective streamed output.
- Generation speed additionally requires a measurable generation interval and
  reported generated tokens.
- End-to-end speed needs reported generated tokens and a positive request
  duration, including for non-streaming requests.

Completed internal protocol-repair attempts retain separate performance samples
and their billable usage. The latest-request view retains the final physical
request's timing and tokens. Failed or cancelled attempts without a completed
provider response do not supply a successful-request speed sample. Their timing
is not used to infer a success rate or a total user-turn duration.

## Surfaces and data contract

- **Web chat:** open the context chip for the latest request's timing and speed,
  alongside current context, cache usage, and conversation totals.
- **Web settings → Usage:** performance cards show rates, average latency, P95,
  and sample counts. The day/provider/model/session tables include performance
  columns.
- **Terminal:** the input status bar shows the latest measured speed or request
  duration. Clicking it opens usage; `/status` shows the request's detailed
  measurements. The usage dashboard includes summaries, wider-screen columns,
  and selected-row details on narrow screens.

`CompletionUsage.performance` stores individual `RequestPerformance` values:

```json
{
  "duration_ms": 2400,
  "ttft_ms": 400,
  "generation_ms": 1800,
  "generated_tokens": 90
}
```

This example yields 50 generation tok/s and 37.5 end-to-end tok/s. The final
physical request is also stored as `CompletionUsage.request_performance` and
exposed in the session API as `usage.last_performance`. `/api/v1/usage` includes
`performance` in its overall and grouped totals, with raw paired totals, derived
rates, averages, percentiles, and measurement counts.

The collector covers streaming completions, non-streaming completions, and the
runtime's direct image request route. Synchronous provider-backed tool requests
also retain their individual HTTP duration when the response contains terminal
usage for that single inference operation. They report request duration and,
when output token usage is available, end-to-end speed. Attributed tool usage
keeps its own provider/model identity.

Polling an existing task, fetching earlier results, batch jobs, and bounded
provider API event streams do not associate their HTTP wait time with a model's
token generation. Those usage records keep their token and cost accounting
without adding an unrelated performance sample.

Existing stored conversations continue to deserialize with empty performance
data. A timing-only response preserves the previous measured context occupancy
instead of replacing it with a zero-token measurement. Cache and context values
use the final request's token snapshot when retries add billable usage.

## Implementation

- [Request timing contract](../crates/agena-domain/src/request_performance.rs)
- [Provider stream collector](../crates/agena-runtime-provider/src/provider/registry/performance.rs)
- [Usage aggregation](../crates/agena-runtime-session/src/usage_stats.rs)
- [Session projection](../crates/agena-runtime-session/src/session/manager/queries.rs)
- [Web formatting and validation](../packages/agena-web/src/lib/usagePerformance.ts)
- [Terminal formatting](../crates/agena-tui/src/performance.rs)

Regression coverage includes reasoning and tool streams, late usage delivery,
single-chunk and non-streaming responses, missing token usage, retry accounting,
weighted aggregation, percentiles, old records, persistence, and narrow layouts.
