import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { Resvg } from '@resvg/resvg-js'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const publicDir = path.join(root, 'public')
const source = readFileSync(path.join(root, 'src/assets/agena.svg'), 'utf8')
const mark = source.match(/<g\b(?=[^>]*\bid="agena-mark")[^>]*>([\s\S]*?)<\/g>/)?.[1]
if (!mark) throw new Error('The Shift source must contain an agena-mark group.')

const fullBleed = source.replace(
  /<rect\b(?=[^>]*\bid="agena-tile")[^>]*\/>/,
  '<rect id="agena-tile" width="1024" height="1024" fill="url(#agena-background)"/>',
)
if (fullBleed === source) throw new Error('The Shift source must contain an agena-tile rectangle.')

const dark = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" fill="none">
  <title>Agena — dark</title>
  <defs>
    <linearGradient id="agena-background" x1="160" y1="80" x2="870" y2="1000" gradientUnits="userSpaceOnUse">
      <stop stop-color="#34312E"/>
      <stop offset="1" stop-color="#1D1C1B"/>
    </linearGradient>
    <linearGradient id="agena-ink" x1="350" y1="260" x2="700" y2="760" gradientUnits="userSpaceOnUse">
      <stop stop-color="#FFC17B"/>
      <stop offset="1" stop-color="#FF9450"/>
    </linearGradient>
  </defs>
  <rect x="32" y="32" width="960" height="960" rx="224" fill="url(#agena-background)"/>
  <g id="agena-mark" fill="url(#agena-ink)">${mark}</g>
</svg>
`
const mask = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="192 192 640 640">
  <title>Agena</title>
  <g id="agena-mark" fill="#000000">${mark}</g>
</svg>
`
const render = (svg, size) => new Resvg(svg, { fitTo: { mode: 'width', value: size } }).render().asPng()

// ICO directory entries point at PNG payloads, supported by modern browsers and Windows.
function makeIco(sizes) {
  const images = sizes.map((size) => render(source, size))
  const header = Buffer.alloc(6 + sizes.length * 16)
  header.writeUInt16LE(1, 2)
  header.writeUInt16LE(sizes.length, 4)
  let offset = header.length
  sizes.forEach((size, index) => {
    const entry = 6 + index * 16
    header[entry] = size === 256 ? 0 : size
    header[entry + 1] = size === 256 ? 0 : size
    header.writeUInt16LE(1, entry + 4)
    header.writeUInt16LE(32, entry + 6)
    header.writeUInt32LE(images[index].length, entry + 8)
    header.writeUInt32LE(offset, entry + 12)
    offset += images[index].length
  })
  return Buffer.concat([header, ...images])
}

const outputs = new Map([
  ['favicon.svg', source],
  ['logo-dark.svg', dark],
  ['safari-pinned-tab.svg', mask],
  ['favicon.ico', makeIco([16, 32, 48, 64, 128, 256])],
  ['agena-icon-1024.png', render(source, 1024)],
  ['agena-icon-dark-1024.png', render(dark, 1024)],
  ['apple-touch-icon.png', render(fullBleed, 180)],
  ['android-chrome-192x192.png', render(source, 192)],
  ['android-chrome-512x512.png', render(source, 512)],
  ['android-chrome-maskable-512x512.png', render(fullBleed, 512)],
])
for (const size of [16, 32, 48]) outputs.set(`favicon-${size}.png`, render(source, size))
for (const size of [152, 167, 180]) outputs.set(`apple-touch-icon-${size}x${size}.png`, render(fullBleed, size))
outputs.set(
  'site.webmanifest',
  `${JSON.stringify(
    {
      id: '/',
      name: 'Agena',
      short_name: 'Agena',
      start_url: '/',
      scope: '/',
      display: 'standalone',
      background_color: '#fbf8f3',
      theme_color: '#fbf8f3',
      icons: [
        { src: '/android-chrome-192x192.png?v=shift', sizes: '192x192', type: 'image/png', purpose: 'any' },
        { src: '/android-chrome-512x512.png?v=shift', sizes: '512x512', type: 'image/png', purpose: 'any' },
        {
          src: '/android-chrome-maskable-512x512.png?v=shift',
          sizes: '512x512',
          type: 'image/png',
          purpose: 'maskable',
        },
      ],
    },
    null,
    2,
  )}\n`,
)

const checkOnly = process.argv.includes('--check')
const stale = []
for (const [name, content] of outputs) {
  const filename = path.join(publicDir, name)
  const expected = Buffer.from(content)
  if (checkOnly) {
    if (!existsSync(filename) || !readFileSync(filename).equals(expected)) stale.push(name)
  } else {
    writeFileSync(filename, expected)
  }
}
if (stale.length) {
  console.error(`Shift icons need regeneration: ${stale.join(', ')}. Run bun run icons:generate.`)
  process.exit(1)
}
console.log(`${checkOnly ? 'Verified' : 'Generated'} ${outputs.size} Shift icon assets from src/assets/agena.svg.`)
