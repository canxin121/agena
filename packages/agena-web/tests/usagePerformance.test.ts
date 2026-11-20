import { describe, expect, it } from 'bun:test'
import { formatDurationMs, formatTokenRate, requestPerformanceMetrics } from '../src/lib/usagePerformance'

describe('model request performance', () => {
  it('separates generation speed from waiting and final usage delivery', () => {
    expect(
      requestPerformanceMetrics({
        duration_ms: 5000,
        ttft_ms: 1000,
        generation_ms: 2000,
        generated_tokens: 100,
      }),
    ).toEqual({
      durationMs: 5000,
      ttftMs: 1000,
      generationMs: 2000,
      tokensPerSecond: 50,
      endToEndTokensPerSecond: 20,
    })
  })

  it('preserves timing when usage is absent', () => {
    expect(requestPerformanceMetrics({ duration_ms: 1500, ttft_ms: 200, generation_ms: 1000 })).toEqual({
      durationMs: 1500,
      ttftMs: 200,
      generationMs: 1000,
      tokensPerSecond: null,
      endToEndTokensPerSecond: null,
    })
  })

  it('does not synthesize measurements for legacy sessions', () => {
    for (const performance of [undefined, null]) {
      expect(requestPerformanceMetrics(performance)).toEqual({
        durationMs: null,
        ttftMs: null,
        generationMs: null,
        tokensPerSecond: null,
        endToEndTokensPerSecond: null,
      })
    }
  })

  it('keeps zero latency distinct from unknown timing and avoids division by zero', () => {
    expect(requestPerformanceMetrics({ duration_ms: 0, ttft_ms: 0, generation_ms: 0, generated_tokens: 30 })).toEqual({
      durationMs: 0,
      ttftMs: 0,
      generationMs: 0,
      tokensPerSecond: null,
      endToEndTokensPerSecond: null,
    })
  })

  it('requires multiple output tokens for generation speed', () => {
    expect(
      requestPerformanceMetrics({ duration_ms: 1000, ttft_ms: 100, generation_ms: 500, generated_tokens: 1 }),
    ).toMatchObject({
      tokensPerSecond: null,
      endToEndTokensPerSecond: 1,
    })
    expect(
      requestPerformanceMetrics({ duration_ms: 1000, ttft_ms: 100, generation_ms: 500, generated_tokens: 0 }),
    ).toMatchObject({
      tokensPerSecond: null,
      endToEndTokensPerSecond: null,
    })
  })

  it('rejects generation intervals outside the measured request', () => {
    for (const timing of [
      { ttft_ms: undefined, generation_ms: 100 },
      { ttft_ms: 1100, generation_ms: 100 },
      { ttft_ms: 600, generation_ms: 500 },
    ]) {
      expect(requestPerformanceMetrics({ duration_ms: 1000, generated_tokens: 20, ...timing })).toMatchObject({
        generationMs: null,
        tokensPerSecond: null,
        endToEndTokensPerSecond: 20,
      })
    }
    expect(requestPerformanceMetrics({ duration_ms: 1000, ttft_ms: 1100 }).ttftMs).toBeNull()
    expect(
      requestPerformanceMetrics({ duration_ms: 1000, ttft_ms: 600, generation_ms: 400, generated_tokens: 20 })
        .tokensPerSecond,
    ).toBe(50)
  })

  it('rejects invalid counters and non-finite computed rates', () => {
    for (const invalid of [-1, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(
        requestPerformanceMetrics({
          duration_ms: invalid,
          ttft_ms: invalid,
          generation_ms: invalid,
          generated_tokens: invalid,
        }),
      ).toEqual({
        durationMs: null,
        ttftMs: null,
        generationMs: null,
        tokensPerSecond: null,
        endToEndTokensPerSecond: null,
      })
    }
    expect(
      requestPerformanceMetrics({
        duration_ms: Number.MIN_VALUE,
        ttft_ms: 0,
        generation_ms: Number.MIN_VALUE,
        generated_tokens: Number.MAX_VALUE,
      }),
    ).toMatchObject({
      tokensPerSecond: null,
      endToEndTokensPerSecond: null,
    })
  })
})

describe('performance labels', () => {
  it('uses an em dash for absent and invalid values', () => {
    for (const value of [undefined, null, '', '12', false, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(formatDurationMs(value, 'en-US')).toBe('—')
      expect(formatTokenRate(value, 'en-US')).toBe('—')
    }
  })

  it('formats zero, milliseconds, seconds, and fractional token rates', () => {
    expect(formatDurationMs(0, 'en-US')).toBe('0 ms')
    expect(formatDurationMs(123.4, 'en-US')).toBe('123 ms')
    expect(formatDurationMs(1234, 'en-US')).toBe('1.23 s')
    expect(formatDurationMs(12000, 'en-US')).toBe('12 s')
    expect(formatTokenRate(12.34, 'en-US')).toBe('12.3 tok/s')
    expect(formatTokenRate(0, 'en-US')).toBe('0.0 tok/s')
  })

  it('respects the locale when formatting numbers', () => {
    expect(formatDurationMs(1234, 'de-DE')).toBe('1,23 s')
    expect(formatTokenRate(12.34, 'de-DE')).toBe('12,3 tok/s')
  })
})
