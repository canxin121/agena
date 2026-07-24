declare module 'bun:test' {
  export function describe(name: string, fn: () => void): void
  export function afterAll(fn: () => void | Promise<void>): void
  export function afterEach(fn: () => void | Promise<void>): void
  export function test(name: string, fn: () => void | Promise<void>): void
  type Matchers<T> = {
    toBe(expected: T): void
    toContain(expected: unknown): void
    toEqual(expected: unknown): void
  }
  export function expect<T>(actual: T): {
    not: Matchers<T>
  } & Matchers<T>
}
