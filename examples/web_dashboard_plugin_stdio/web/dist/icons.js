const paths = {
  plus: ['M12 5v14M5 12h14'],
  refresh: ['M20 7v5h-5', 'M4 17v-5h5', 'M6.1 7a7 7 0 0 1 11.6-2L20 8', 'M4 16l2.3 3A7 7 0 0 0 18 17'],
  search: ['m21 21-5-5', 'M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0'],
  check: ['m5 12 4 4L19 6'],
  arrow: ['M5 12h14m-6-6 6 6-6 6'],
  close: ['m6 6 12 12M6 18 18 6'],
  edit: ['m16 3 5 5-12 12-6 1 1-6L16 3Z', 'm13 6 5 5'],
  board: ['M3 4h18v16H3z', 'M8 4v16M16 4v16', 'M5 8h1m4 0h4m4 0h1M10 12h4'],
  dots: ['M5 12h.01M12 12h.01M19 12h.01'],
  trash: ['M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7'],
}

export function icon(document, name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
  svg.setAttribute('viewBox', '0 0 24 24')
  svg.setAttribute('fill', 'none')
  svg.setAttribute('stroke', 'currentColor')
  svg.setAttribute('stroke-width', '1.7')
  svg.setAttribute('stroke-linecap', 'round')
  svg.setAttribute('stroke-linejoin', 'round')
  svg.setAttribute('aria-hidden', 'true')
  for (const d of paths[name] || paths.board) {
    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path')
    path.setAttribute('d', d)
    svg.append(path)
  }
  return svg
}
