/** Chart vocabulary shared with the legacy fenced-chart reader. */
const record = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === 'object' && !Array.isArray(value)
const text = (value: unknown, max: number): value is string => typeof value === 'string' && value.length <= max
export type ChartSpec = {
  type: 'line' | 'area' | 'bar' | 'scatter' | 'pie'
  title?: string
  xLabel?: string
  yLabel?: string
  labels: string[]
  series: { name: string; values: (number | null)[] }[]
}

export function parseChartSpec(raw: unknown): ChartSpec {
  if (
    !record(raw) ||
    !['line', 'area', 'bar', 'scatter', 'pie'].includes(String(raw.type)) ||
    !Array.isArray(raw.labels) ||
    !raw.labels.length ||
    raw.labels.length > 512 ||
    raw.labels.some((v) => !text(v, 100)) ||
    !Array.isArray(raw.series) ||
    !raw.series.length ||
    raw.series.length > 8 ||
    ['title', 'xLabel', 'yLabel'].some((k) => raw[k] !== undefined && !text(raw[k], 200))
  )
    throw new Error('Invalid chart schema or size limits exceeded.')
  for (const series of raw.series) {
    if (
      !record(series) ||
      !text(series.name, 100) ||
      !Array.isArray(series.values) ||
      series.values.length !== raw.labels.length ||
      series.values.some((v) => v !== null && (typeof v !== 'number' || !Number.isFinite(v) || Math.abs(v) > 1e100))
    )
      throw new Error('Chart series must match labels and contain finite numbers or null.')
  }
  if (!raw.series.some((s) => (s as { values: unknown[] }).values.some((v) => v !== null)))
    throw new Error('Chart has no numeric data.')
  if (
    raw.type === 'pie' &&
    (raw.series.length !== 1 ||
      raw.series[0].values.some((v: number | null) => v !== null && v < 0) ||
      !raw.series[0].values.some((v: number | null) => v !== null && v > 0))
  )
    throw new Error('Pie charts need one nonnegative series with a positive total.')
  return raw as ChartSpec
}
