// Readers writing the same projection share ownership. A late response (or
// failure) cannot replace a newer read, even when two loaders share one ref.
const owners = new WeakMap<object, number>()

export function createLatestRequestGuard(scope: () => unknown, target: object = {}) {
  return () => {
    const identity = scope()
    const owner = (owners.get(target) ?? 0) + 1
    owners.set(target, owner)
    return () => owners.get(target) === owner && scope() === identity
  }
}
