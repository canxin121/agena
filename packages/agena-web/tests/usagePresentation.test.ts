import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { renderToString } from '@vue/server-renderer'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import { settingsTextForLocale } from '../src/i18n/settingsTextCatalog'
import * as usageFormatting from '../src/lib/usageFormatting'
import * as performanceFormatting from '../src/lib/usagePerformance'
import type { UsagePerformanceTotals } from '../src/types/usage'

const performance: UsagePerformanceTotals = {
  request_count: 3,
  ttft_count: 2,
  generation_count: 1,
  throughput_count: 2,
  total_duration_ms: 9000,
  total_ttft_ms: 800,
  total_generation_ms: 2000,
  timed_tokens: 100,
  timed_duration_ms: 5000,
  generation_tokens: 100,
  average_duration_ms: 3000,
  average_ttft_ms: 400,
  p50_duration_ms: 3000,
  p95_duration_ms: 5000,
  p50_ttft_ms: 200,
  p95_ttft_ms: 600,
  tokens_per_second: 50,
  end_to_end_tokens_per_second: 20,
}

async function renderComponent(
  name: 'settings/UsagePanel' | 'chat/ContextUsageChip',
  options: { stats?: Record<string, unknown>; props?: Record<string, unknown>; locale?: string } = {},
) {
  const source = readFileSync(new URL(`../src/components/${name}.vue`, import.meta.url), 'utf8')
  const { descriptor } = parse(source)
  const code = transformSync(compileScript(descriptor, { id: 'usage-presentation', inlineTemplate: true }).content, {
    loader: 'ts',
    format: 'cjs',
  }).code
  const text = (source: string, params?: Record<string, string | number>) =>
    settingsTextForLocale(options.locale || 'en-US', source, params)
  const container = vue.defineComponent({
    props: ['title'],
    setup(props, { slots }) {
      return () => vue.h('section', [vue.h('h2', props.title), slots.default?.()])
    },
  })
  const icon = () => vue.h('span')
  const modules: Record<string, unknown> = {
    vue,
    '@remixicon/vue': new Proxy({}, { get: () => icon }),
    '@/lib/api': { apiJson: async () => ({}) },
    '@/lib/usagePerformance': performanceFormatting,
    '@/lib/usageFormatting': usageFormatting,
    '@/i18n/settingsText': { settingsText: text },
    '@/composables/usePaneRead': {
      usePaneRead(_read: unknown, apply: (value: unknown) => void) {
        apply(options.stats)
        return { loading: vue.ref(false), error: vue.ref(''), refresh() {} }
      },
    },
  }
  const module = { exports: {} as { default: vue.Component } }
  new Function('require', 'module', 'exports', code)(
    (id: string) => {
      if (id.endsWith('.vue')) return container
      if (!(id in modules)) throw new Error(`Unexpected component dependency: ${id}`)
      return modules[id]
    },
    module,
    module.exports,
  )
  const app = vue.createSSRApp(module.exports.default, options.props)
  app.config.globalProperties.$st = text
  return renderToString(app)
}

function stats(section = 'by_day', withPerformance = true) {
  const totals = {
    requests: 7,
    sessions: 1,
    total_tokens: 1000,
    total_cost_usd: 0.01,
    ...(withPerformance ? { performance } : {}),
  }
  return {
    totals,
    period_label: 'Last 30 days',
    active_days: 1,
    [section]: [
      { ...totals, date: '2026-11-18', provider_id: 'test-provider', model_id: 'test-model', title: 'Example session' },
    ],
  }
}

describe('usage performance presentation', () => {
  it('keeps billion-scale token totals compact and exposes their exact count', async () => {
    const usage = stats()
    usage.totals.total_tokens = 2_500_000_123
    const html = await renderComponent('settings/UsagePanel', {
      stats: {
        ...usage,
        totals: {
          ...usage.totals,
          cache_input_tokens: 2_000_000,
          cache_read_tokens: 0,
          cache_hit_rate: 0,
          other_tokens: 999_950,
        },
        by_day: [{ ...usage.totals, date: '2026-11-18' }],
      },
    })
    expect(html.match(/>2\.5b</g)).toHaveLength(2)
    expect(html).toContain('title="2,500,000,123"')
    expect(html).toContain('>2m<')
    expect(html).toContain('>1m<')
    expect(html).toContain('>0%<')
  })

  it('renders server aggregates with the metric-specific sample counts and P95 timings', async () => {
    const html = await renderComponent('settings/UsagePanel', { stats: stats() })
    expect(html).toContain('50.0 tok/s')
    expect(html).toContain('20.0 tok/s')
    expect(html).toContain('400 ms')
    expect(html).toContain('600 ms')
    expect(html).toContain('5 s')
    expect(html).toContain('Completed requests with timing: 3')
    expect(html).toContain('Samples: 1')
    expect(html).toContain('Samples: 2')
    expect(html).toContain('Samples: 3')
  })

  it('renders each breakdown dimension with its own measurements', async () => {
    for (const [section, label] of Object.entries({
      by_day: '2026-11-18',
      by_provider: 'test-provider',
      by_model: 'test-provider / test-model',
      by_session: 'Example session',
    })) {
      const html = await renderComponent('settings/UsagePanel', { stats: stats(section) })
      expect(html).toContain(label)
      expect(html.match(/50\.0 tok\/s/g)).toHaveLength(2)
      expect(html.match(/400 ms/g)).toHaveLength(2)
      expect(html.match(/>3 s</g)).toHaveLength(2)
    }
  })

  it('keeps historical usage readable without making up performance measurements', async () => {
    const html = await renderComponent('settings/UsagePanel', { stats: stats('by_day', false) })
    expect(html).toContain('No performance samples yet.')
    expect(html).toContain('Completed requests with timing: 0')
    expect(html).toContain('—')
    expect(html).not.toContain('0.0 tok/s')
  })

  it('shows the last completed request in the Chinese context popover', async () => {
    const html = await renderComponent('chat/ContextUsageChip', {
      locale: 'zh-CN',
      props: {
        sessionId: '1',
        open: false,
        usage: {
          tokensValue: 1200,
          tokensLabel: '1.2k',
          percentUsed: 12,
          lastPerformance: { duration_ms: 5000, ttft_ms: 200, generation_ms: 2000, generated_tokens: 100 },
        },
      },
    })
    expect(html).toContain('最近完成的请求')
    expect(html).toContain('首输出延迟')
    expect(html).toContain('200 ms')
    expect(html).toContain('50.0 tok/s')
    expect(html).toContain('20.0 tok/s')
  })

  it('shows the context performance empty state when the session predates timing', async () => {
    const html = await renderComponent('chat/ContextUsageChip', {
      props: { sessionId: '1', open: false, usage: { tokensLabel: '—', percentUsed: null } },
    })
    expect(html).toContain('Performance appears after a completed model request.')
    expect(html).not.toContain('NaN')
    expect(html).not.toContain('tok/s')
  })
})
