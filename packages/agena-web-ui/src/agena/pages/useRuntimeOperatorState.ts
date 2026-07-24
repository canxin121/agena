import type { Ref } from 'vue'

import type { RuntimeStatus } from '../lib/agenaApi'

export type RuntimeOperatorStateInput = {
  runtime: Ref<RuntimeStatus | null>
}

export function useRuntimeOperatorState(input: RuntimeOperatorStateInput) {
  return {
    runtime: input.runtime,
  }
}
