/** Load an expanded tree with bounded work in flight, then flatten it once.
 * Row order is independent of request completion order. Deep trees never
 * recurse or copy every ancestor's growing descendant array.
 */
export async function loadExpandedTree<Input, Row>(
  roots: readonly Input[],
  read: (input: Input, depth: number, root: Input) => Promise<{ row: Row; children?: readonly Input[] } | null>,
  signal?: AbortSignal,
  concurrency = 4,
): Promise<Row[]> {
  type Node = { row: Row; children: Array<Node | null> }
  type Work = { input: Input; depth: number; root: Input; target: Array<Node | null>; index: number }
  const tree: Array<Node | null> = new Array(roots.length).fill(null)
  const queue: Work[] = roots.map((input, index) => ({ input, depth: 0, root: input, target: tree, index }))
  const width = Math.max(1, Math.floor(concurrency))
  let head = 0
  while (head < queue.length) {
    signal?.throwIfAborted()
    const batch = queue.slice(head, head + width)
    head += batch.length
    await Promise.all(
      batch.map(async (work) => {
        const result = await read(work.input, work.depth, work.root)
        signal?.throwIfAborted()
        if (!result) return
        const children = result.children ?? []
        const node: Node = { row: result.row, children: new Array(children.length).fill(null) }
        work.target[work.index] = node
        for (let index = 0; index < children.length; index++) {
          queue.push({ input: children[index]!, depth: work.depth + 1, root: work.root, target: node.children, index })
        }
      }),
    )
  }
  const rows: Row[] = []
  const stack = tree.slice().reverse()
  while (stack.length) {
    const node = stack.pop()
    if (!node) continue
    rows.push(node.row)
    for (let index = node.children.length - 1; index >= 0; index--) stack.push(node.children[index]!)
  }
  return rows
}
