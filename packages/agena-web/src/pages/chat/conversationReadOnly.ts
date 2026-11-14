import { computed, inject, provide, type InjectionKey, type Ref } from 'vue'

const conversationReadOnlyKey: InjectionKey<Readonly<Ref<boolean>>> = Symbol('conversation-read-only')

export function provideConversationReadOnly(readOnly: Readonly<Ref<boolean>>) {
  provide(conversationReadOnlyKey, readOnly)
}

export function useConversationReadOnly(): Readonly<Ref<boolean>> {
  return inject(
    conversationReadOnlyKey,
    computed(() => false),
  )
}
