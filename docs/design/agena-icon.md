# Agena app icon

The selected Agena identity is **Shift (01)**. The mark is a custom uppercase
**A** with a rounded apex and a rising crossbar. A diagonal opening on the
right gives it a forward direction while keeping the silhouette readable at
favicon sizes. The warm orange palette follows the application's accent colors.

![Primary and dark Shift icons with small-size previews](agena-icon-preview.png)

The preview above is a design sheet, not a runtime asset. The icons shipped by
the application are generated from the SVG source described below.

## Source

The editable source is:

```text
packages/agena-web/src/assets/agena.svg
```

It is a 1024 × 1024 SVG with three important parts:

| SVG element | Purpose |
| --- | --- |
| `#agena-background` | Three-stop orange background gradient |
| `#agena-tile` | Rounded 960 × 960 tile inset by 32 px |
| `#agena-mark` | The two path elements that draw the Shift letter |

`AgenaLogo.vue` imports this source directly for the login page and the
desktop/mobile workspace and conversation empty states. Vite bundles the
imported image with the application.

The generator in
`packages/agena-web/scripts/generate-app-icons.mjs` reads the `agena-mark`
group and the `agena-tile` rectangle by ID. Renaming either element breaks icon
generation; the script fails instead of producing a partial asset set.

## Generated assets

All of these files are written to `packages/agena-web/public/`. There are 17
outputs in total.

| Asset | Dimensions | How it is produced | Purpose |
| --- | --- | --- | --- |
| `favicon.svg` | 1024 × 1024 viewBox | Exact copy of the source SVG | Scalable browser favicon |
| `logo-dark.svg` | 1024 × 1024 | Shift mark on the charcoal tile | Dark-background logo mark |
| `safari-pinned-tab.svg` | 1024 × 1024, viewBox `192 192 640 640` | Monochrome mark only, `fill="#000000"` | Safari pinned-tab mask |
| `favicon.ico` | Multi-size ICO | PNG payloads at 16, 32, 48, 64, 128, and 256 px | Legacy browser and Windows favicon |
| `favicon-16.png` | 16 × 16 | Rendered from the source | Browser favicon fallback |
| `favicon-32.png` | 32 × 32 | Rendered from the source | Browser favicon fallback |
| `favicon-48.png` | 48 × 48 | Rendered from the source | Browser favicon fallback |
| `agena-icon-1024.png` | 1024 × 1024 | Rendered from the source | Primary raster app icon |
| `agena-icon-dark-1024.png` | 1024 × 1024 | Rendered from the dark SVG | Dark raster app icon |
| `apple-touch-icon.png` | 180 × 180 | Full-bleed source rendering | Default Apple touch icon |
| `apple-touch-icon-152x152.png` | 152 × 152 | Full-bleed source rendering | iPad touch icon |
| `apple-touch-icon-167x167.png` | 167 × 167 | Full-bleed source rendering | iPad Pro touch icon |
| `apple-touch-icon-180x180.png` | 180 × 180 | Full-bleed source rendering | iPhone touch icon |
| `android-chrome-192x192.png` | 192 × 192 | Rendered from the source | Standard Android icon |
| `android-chrome-512x512.png` | 512 × 512 | Rendered from the source | Standard Android/desktop install icon |
| `android-chrome-maskable-512x512.png` | 512 × 512 | Full-bleed source rendering | Maskable Android icon |
| `site.webmanifest` | — | Generated JSON | Web app manifest |

### Rendering rules

- `favicon.svg` keeps the rounded tile and its transparency outside the tile.
- `logo-dark.svg` and `agena-icon-dark-1024.png` use a separate charcoal
  background gradient from `#34312E` to `#1D1C1B` and a warm mark gradient from
  `#FFC17B` to `#FF9450`.
- `safari-pinned-tab.svg` contains only the black letter paths. Its tighter
  viewBox keeps the letter legible in a pinned tab. The displayed color comes
  from the `mask-icon` link in `index.html`, currently `#F47B38`.
- The primary and standard Android PNGs preserve transparency outside the
  rounded tile.
- Apple touch icons and the Android maskable icon replace the inset rounded
  tile with an opaque full-bleed 1024 × 1024 background. This lets the platform
  apply its own corner mask without showing transparent corners. The Shift
  letter stays inside the central maskable safe area.
- `favicon.ico` uses PNG payloads inside the ICO directory entries. Modern
  browsers and Windows support this format; the generator does not emit BMP
  icon data.

### Manifest

`site.webmanifest` contains:

```json
{
  "id": "/",
  "name": "Agena",
  "short_name": "Agena",
  "start_url": "/",
  "scope": "/",
  "display": "standalone",
  "background_color": "#fbf8f3",
  "theme_color": "#fbf8f3",
  "icons": [
    { "src": "/android-chrome-192x192.png?v=shift", "sizes": "192x192", "type": "image/png", "purpose": "any" },
    { "src": "/android-chrome-512x512.png?v=shift", "sizes": "512x512", "type": "image/png", "purpose": "any" },
    { "src": "/android-chrome-maskable-512x512.png?v=shift", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
  ]
}
```

## Browser references

`packages/agena-web/index.html` references the icon set with a `?v=shift`
cache-busting query so previously cached icons are refreshed:

- `favicon.ico?v=shift`
- `favicon-16.png?v=shift`
- `favicon-32.png?v=shift`
- `favicon.svg?v=shift`
- `safari-pinned-tab.svg?v=shift` with `color="#F47B38"`
- `apple-touch-icon-180x180.png?v=shift`
- `apple-touch-icon-167x167.png?v=shift`
- `apple-touch-icon-152x152.png?v=shift`
- `site.webmanifest?v=shift`

The page theme colors are `#fbf8f3` for light mode and `#151313` for dark
mode. The manifest keeps the light background and theme colors used for
installation surfaces.

## Palette

| Element | Colors |
| --- | --- |
| Primary tile | `#FFC17B` → `#FF9A51` (52%) → `#F47B38` |
| Primary letter | `#252423` |
| Dark tile | `#34312E` → `#1D1C1B` |
| Dark letter | `#FFC17B` → `#FF9450` |

## Regenerate and verify

From `packages/agena-web`:

```sh
bun run icons:generate
bun run check:icons
```

`bun run icons:generate` writes all 17 assets. `bun run check:icons` compares
the expected bytes with the files in `public/` and reports every stale or
missing asset; it exits nonzero instead of rewriting them.

Icons are checked as part of both frontend build modes:

```sh
bun run build          # production, output: dist/
bun run build:rust-debug  # source maps, output: dist-rust-debug/
```

Both begin with `check:icons`, so a stale icon fails the build.

The generator uses `@resvg/resvg-js` version `2.6.2`. It does not require
ImageMagick, system fonts, or external image files. Edit the SVG source and
regenerate; do not edit files under `public/` by hand.

## Candidate archive

`docs/design/agena-icon-options/` preserves the original design exploration.
It contains eight directions (`01`–`08`), each with an SVG, a 1024 px PNG, and
32 px and 64 px previews, plus `overview.png` comparing the set. `01-shift`
became the production source; the other seven are design references and are not
loaded by the application.

The candidate directory has no generation script. Its PNGs are static exports;
if a candidate SVG is edited, re-export the corresponding preview sizes
manually. See the [candidate index](agena-icon-options/README.md) for the
direction descriptions.
