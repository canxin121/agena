/** Count text, breaks and attachment chips once without cloning DOM ranges.
 * Both endpoints share the same traversal, including collapsed selections.
 */
export function composerDomSelection(root: Node, range: Range): [number, number] {
  let length = 0
  let start = 0
  let end = 0
  const boundary = (node: Node, offset: number) => {
    if (node === range.startContainer && offset === range.startOffset) start = length
    if (node === range.endContainer && offset === range.endOffset) end = length
  }
  const visit = (node: Node) => {
    if (node.nodeType === 3) {
      if (node === range.startContainer) start = length + range.startOffset
      if (node === range.endContainer) end = length + range.endOffset
      length += node.textContent?.length ?? 0
    } else if (
      node.nodeType === 1 &&
      ((node as HTMLElement).dataset?.attachmentId || (node as Element).tagName === 'BR')
    ) {
      boundary(node, 0)
      length++
    } else {
      boundary(node, 0)
      for (let i = 0; i < node.childNodes.length; i++) {
        visit(node.childNodes[i]!)
        boundary(node, i + 1)
      }
    }
  }
  visit(root)
  return [Math.min(length, Math.max(0, start)), Math.min(length, Math.max(0, end))]
}
