import type { AssistantMemory, AssistantMemoryUpdate, MemoryKind } from '../generated/agenaState'

const memoryKinds: readonly MemoryKind[] = ['fact', 'preference', 'episode', 'procedure', 'task']

export function isMemoryKind(value: string): value is MemoryKind {
  return memoryKinds.some((kind) => kind === value)
}

export function memoryRevision(sha256: string): string {
  if (!/^[a-f0-9]{64}$/i.test(sha256)) throw new Error('Memory revision is missing or invalid.')
  return sha256
}

export function memoryWriteRequest(kind: MemoryKind, body: string, description: string, expectedSha256: string) {
  return { kind, body, description, expected_sha256: memoryRevision(expectedSha256) }
}

export function memoryBatchDeleteRequest(records: readonly { name: string; sha256: string }[]) {
  const names = new Set<string>()
  return {
    records: records.map(({ name, sha256 }) => {
      if (!name || names.has(name)) throw new Error('Select each memory only once.')
      names.add(name)
      return { name, sha256: memoryRevision(sha256) }
    }),
  }
}

export function assistantMemoryRequest(
  original: AssistantMemory,
  edit: { body: string; description: string } | null,
): AssistantMemoryUpdate {
  const revision = memoryRevision(original.sha256)
  if (edit === null) return { name: original.name, expected_sha256: revision }
  return {
    name: original.name,
    kind: original.metadata.kind,
    body: edit.body,
    description: edit.description,
    expected_sha256: revision,
  }
}
