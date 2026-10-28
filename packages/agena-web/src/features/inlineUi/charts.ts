/** Chart renderer props; documents are validated by the Inline UI contract. */
export type ChartSpec = {
  type: 'line' | 'area' | 'bar' | 'scatter' | 'pie'
  title?: string
  xLabel?: string
  yLabel?: string
  labels: string[]
  series: { name: string; values: (number | null)[] }[]
}
