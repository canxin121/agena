/** Timings measured by the runtime for one completed model request. */
export type RequestPerformance = {
  duration_ms: number
  ttft_ms?: number | null
  generation_ms?: number | null
  generated_tokens?: number | null
}

/** Each metric uses only the completed requests that contain its measurements. */
export type UsagePerformanceTotals = {
  request_count: number
  ttft_count: number
  generation_count: number
  throughput_count: number
  total_duration_ms: number
  total_ttft_ms: number
  total_generation_ms: number
  timed_tokens: number
  timed_duration_ms: number
  generation_tokens: number
  average_duration_ms: number | null
  average_ttft_ms: number | null
  p50_duration_ms: number | null
  p95_duration_ms: number | null
  p50_ttft_ms: number | null
  p95_ttft_ms: number | null
  tokens_per_second: number | null
  end_to_end_tokens_per_second: number | null
}
