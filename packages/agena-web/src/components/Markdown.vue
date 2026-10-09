<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { onClickOutside } from '@vueuse/core'
import { RiListUnordered } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import { renderMermaid } from '@/lib/mermaidRenderer'
import { renderMarkdown, renderMarkdownPlainText, type MarkdownUiLabels } from '@/lib/markdown'
import { useNearViewport } from '@/composables/useNearViewport'
import { createMarkdownStreamKey, releaseMarkdownStream, renderMarkdownAsync } from '@/lib/markdownAsync'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { copyTextToClipboard } from '@/lib/clipboard'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { resolveWorkspaceFileLink, resolveWorkspaceMediaUrl } from '@/lib/workspaceLinks'
import { useDirectoryStore } from '@/stores/directory'
import { useToastsStore } from '@/stores/toasts'
import { useUiStore, type ImageViewerItem } from '@/stores/ui'

const props = withDefaults(
  defineProps<{
    content: string
    mode?: 'markdown' | 'plain'
    sourcePath?: string
    revealAnchor?: string
    revealRequestSeq?: number
    // When true, debounce markdown parsing to avoid re-rendering on every SSE chunk.
    stream?: boolean
    streamDebounceMs?: number
  }>(),
  {
    mode: 'markdown',
    sourcePath: '',
    revealAnchor: '',
    revealRequestSeq: 0,
    stream: false,
    streamDebounceMs: 60,
  },
)

const html = ref<string[]>([])
const markdownStreamKey = createMarkdownStreamKey()
let hasStreamed = false
const pane = useWorkspacePaneContext()
let parseController: AbortController | undefined
let renderVersion = 0
const rootEl = ref<HTMLElement | null>(null)
const nearViewport = useNearViewport(rootEl)
const deferredHtml = computed(() => renderMarkdownPlainText(props.content))
let timer: number | null = null
let copiedTimer: number | null = null
let mermaidTimer: number | null = null
let lastRevealAnchorKey = ''
let imageObserver: IntersectionObserver | null = null

type MarkdownImageSizeMeta = {
  width: number
  height: number
  touchedAt: number
}

type MarkdownImageCacheState = {
  sizeBySrc: Map<string, MarkdownImageSizeMeta>
  loadedSrc: Set<string>
  storageLoaded: boolean
  persistTimer: number | null
}

const MD_IMAGE_PLACEHOLDER_SRC = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw=='
const MD_IMAGE_SIZE_CACHE_STORAGE_KEY = 'agena.markdown.image-size-cache'
const MD_IMAGE_SIZE_CACHE_LIMIT = 320

function markdownImageCacheState(): MarkdownImageCacheState {
  const g = globalThis as typeof globalThis & { __agenaMarkdownImageCacheState?: MarkdownImageCacheState }
  if (!g.__agenaMarkdownImageCacheState) {
    g.__agenaMarkdownImageCacheState = {
      sizeBySrc: new Map<string, MarkdownImageSizeMeta>(),
      loadedSrc: new Set<string>(),
      storageLoaded: false,
      persistTimer: null,
    }
  }
  return g.__agenaMarkdownImageCacheState
}

function loadMarkdownImageSizeCacheFromStorage() {
  const state = markdownImageCacheState()
  if (state.storageLoaded) return
  state.storageLoaded = true

  if (typeof window === 'undefined') return
  let raw = ''
  try {
    raw = String(window.localStorage.getItem(MD_IMAGE_SIZE_CACHE_STORAGE_KEY) || '')
  } catch {
    return
  }
  if (!raw) return

  try {
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return
    for (const entry of parsed) {
      if (!entry || typeof entry !== 'object') continue
      const src = typeof (entry as { src?: unknown }).src === 'string' ? String((entry as { src?: unknown }).src) : ''
      const width = Number((entry as { width?: unknown }).width)
      const height = Number((entry as { height?: unknown }).height)
      const touchedAt = Number((entry as { touchedAt?: unknown }).touchedAt)
      if (!src || !Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) continue
      state.sizeBySrc.set(src, {
        width: Math.max(1, Math.round(width)),
        height: Math.max(1, Math.round(height)),
        touchedAt: Number.isFinite(touchedAt) ? touchedAt : Date.now(),
      })
    }
  } catch {
    // Ignore malformed cache payload.
  }
}

function schedulePersistMarkdownImageSizeCache() {
  if (typeof window === 'undefined') return
  const state = markdownImageCacheState()
  if (state.persistTimer !== null) return

  state.persistTimer = window.setTimeout(() => {
    state.persistTimer = null
    const entries = Array.from(state.sizeBySrc.entries())
      .sort((a, b) => (b[1].touchedAt || 0) - (a[1].touchedAt || 0))
      .slice(0, MD_IMAGE_SIZE_CACHE_LIMIT)
      .map(([src, meta]) => ({
        src,
        width: meta.width,
        height: meta.height,
        touchedAt: meta.touchedAt,
      }))

    try {
      window.localStorage.setItem(MD_IMAGE_SIZE_CACHE_STORAGE_KEY, JSON.stringify(entries))
    } catch {
      // Ignore storage quota / privacy mode failures.
    }
  }, 260)
}

function cachedMarkdownImageSize(src: string): MarkdownImageSizeMeta | null {
  const key = String(src || '').trim()
  if (!key) return null
  loadMarkdownImageSizeCacheFromStorage()
  const state = markdownImageCacheState()
  const found = state.sizeBySrc.get(key)
  if (!found) return null
  found.touchedAt = Date.now()
  return found
}

function rememberMarkdownImageSize(src: string, width: number, height: number) {
  const key = String(src || '').trim()
  if (!key) return
  const w = Math.max(1, Math.round(Number(width) || 0))
  const h = Math.max(1, Math.round(Number(height) || 0))
  if (!w || !h) return

  const state = markdownImageCacheState()
  state.sizeBySrc.set(key, { width: w, height: h, touchedAt: Date.now() })
  state.loadedSrc.add(key)

  if (state.sizeBySrc.size > MD_IMAGE_SIZE_CACHE_LIMIT * 1.35) {
    const sorted = Array.from(state.sizeBySrc.entries()).sort((a, b) => (b[1].touchedAt || 0) - (a[1].touchedAt || 0))
    state.sizeBySrc = new Map(sorted.slice(0, MD_IMAGE_SIZE_CACHE_LIMIT))
  }

  schedulePersistMarkdownImageSizeCache()
}

function markMarkdownImageLoaded(src: string) {
  const key = String(src || '').trim()
  if (!key) return
  markdownImageCacheState().loadedSrc.add(key)
}

function hasMarkdownImageLoaded(src: string): boolean {
  const key = String(src || '').trim()
  if (!key) return false
  return markdownImageCacheState().loadedSrc.has(key)
}

// Table of Contents
type TocItem = { id: string; text: string; level: number }
const toc = ref<TocItem[]>([])
const showToc = ref(false)
const tocMenuRef = ref<HTMLElement | null>(null)

onClickOutside(tocMenuRef, () => {
  showToc.value = false
})

function scanToc() {
  if (!rootEl.value) {
    toc.value = []
    return
  }
  const headings = rootEl.value.querySelectorAll('h1, h2, h3, h4, h5, h6')
  const items: TocItem[] = []

  headings.forEach((h) => {
    // Only include headings that have our generated ID (meaning they have valid content)
    const id = h.getAttribute('data-oc-id')
    if (id) {
      items.push({
        id,
        text: h.textContent || '',
        level: parseInt(h.tagName.substring(1), 10),
      })
    }
  })

  toc.value = items
}

function decodeAnchorId(rawId: string): string {
  const input = String(rawId || '').trim()
  if (!input) return ''
  try {
    return decodeURIComponent(input)
  } catch {
    return input
  }
}

function findHeadingByAnchor(rawId: string): HTMLElement | null {
  const id = decodeAnchorId(rawId)
  if (!id) return null

  const escaped = id.replace(/"/g, '\\"')
  let heading = rootEl.value?.querySelector(`[data-oc-id="${escaped}"]`) as HTMLElement | null
  if (heading) return heading

  // Loose match: normalize punctuation differences between external sluggers and ours.
  const normalize = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]/gu, '')
  const target = normalize(id)
  if (!target) return null

  const headings = rootEl.value?.querySelectorAll('[data-oc-id]')
  if (!headings) return null
  for (const h of headings) {
    const hid = h.getAttribute('data-oc-id') || ''
    if (normalize(hid) === target) {
      heading = h as HTMLElement
      break
    }
  }

  return heading
}

function scrollToAnchor(rawId: string): boolean {
  const heading = findHeadingByAnchor(rawId)
  if (!heading) return false
  heading.scrollIntoView({ behavior: 'smooth', block: 'start' })
  return true
}

function scrollToHeading(id: string) {
  showToc.value = false
  scrollToAnchor(id)
}

const toasts = useToastsStore()
const directoryStore = useDirectoryStore()
const ui = useUiStore()
const workspaceNavigation = useWorkspaceNavigation()
const { t } = useI18n()

function buildMarkdownLabels(): Partial<MarkdownUiLabels> {
  return {
    copyTitle: t('codeBlock.copy'),
    copyCodeAria: t('codeBlock.copyAria'),
    copyDiagramSourceAria: t('codeBlock.copyDiagramSourceAria'),
    toggleCodeAria: t('codeBlock.toggleCodeAria'),
    toggleDiagramAria: t('codeBlock.toggleDiagramAria'),
    expandTitle: t('codeBlock.expand'),
    expandCodeAria: t('codeBlock.expandAria'),
    expandDiagramAria: t('codeBlock.expandDiagramAria'),
    expandLinesTitle: (lines: number) => t('codeBlock.expandLines', { lines }),
  }
}

function clearTimer() {
  if (timer !== null) {
    window.clearTimeout(timer)
    timer = null
  }
}

async function updateNow() {
  if (!nearViewport.value || (pane && !pane.isVisible.value)) return
  parseController?.abort()
  const request = new AbortController()
  parseController = request
  const version = ++renderVersion
  const labels = buildMarkdownLabels()
  hasStreamed ||= props.stream
  let rendered: string[]
  try {
    rendered =
      hasStreamed || props.content.length > 8192
        ? await renderMarkdownAsync(props.content, labels, request.signal, markdownStreamKey)
        : [renderMarkdown(props.content, labels)]
  } catch (error) {
    if (request.signal.aborted) return
    console.error('Markdown rendering failed', error)
    rendered = [renderMarkdownPlainText(props.content)]
  }
  if (version !== renderVersion || request.signal.aborted) return
  resetMermaid()
  if (imageObserver) {
    imageObserver.disconnect()
    imageObserver = null
  }
  html.value = rendered
  // Scan TOC and rewrite local media links after rendering.
  nextTick(() => {
    scanToc()
    hydrateWorkspaceMedia()
    scheduleHydrateMermaid()
  })
}

function setMarkdownImageStateClass(image: HTMLImageElement, state: 'pending' | 'loading' | 'loaded' | 'error') {
  image.dataset.ocMdImageState = state
  image.classList.remove('oc-md-image--pending', 'oc-md-image--loading', 'oc-md-image--loaded', 'oc-md-image--error')
  image.classList.add(`oc-md-image--${state}`)
}

function applyCachedMarkdownImageDimensions(image: HTMLImageElement, src: string): boolean {
  const cached = cachedMarkdownImageSize(src)
  if (!cached) return false

  image.setAttribute('width', String(cached.width))
  image.setAttribute('height', String(cached.height))
  image.style.removeProperty('width')
  image.style.removeProperty('min-height')
  image.style.removeProperty('aspect-ratio')
  return true
}

function applyFallbackMarkdownImagePlaceholder(image: HTMLImageElement) {
  const width = Number(image.getAttribute('width') || '')
  const height = Number(image.getAttribute('height') || '')
  const hasExplicitSize = Number.isFinite(width) && width > 0 && Number.isFinite(height) && height > 0
  if (hasExplicitSize) return

  image.style.setProperty('width', '100%')
  image.style.setProperty('min-height', '140px')
  image.style.setProperty('aspect-ratio', '16 / 10')
}

function applyCachedMarkdownImageDimensionsInRoot(src: string) {
  const root = rootEl.value
  if (!root) return
  for (const image of root.querySelectorAll<HTMLImageElement>('img[data-oc-md-src]')) {
    if (String(image.getAttribute('data-oc-md-src') || '').trim() !== src) continue
    applyCachedMarkdownImageDimensions(image, src)
  }
}

function recordMarkdownImageLoaded(image: HTMLImageElement, src: string) {
  const width = Number(image.naturalWidth)
  const height = Number(image.naturalHeight)
  if (Number.isFinite(width) && width > 0 && Number.isFinite(height) && height > 0) {
    rememberMarkdownImageSize(src, width, height)
    applyCachedMarkdownImageDimensionsInRoot(src)
  } else {
    markMarkdownImageLoaded(src)
  }
}

function loadMarkdownImageElement(image: HTMLImageElement) {
  const src = String(image.getAttribute('data-oc-md-src') || '').trim()
  if (!src) return

  const state = String(image.dataset.ocMdImageState || '')
  if (state === 'loaded' || state === 'loading') return

  setMarkdownImageStateClass(image, 'loading')

  let done = false
  const cleanup = () => {
    image.removeEventListener('load', handleLoad)
    image.removeEventListener('error', handleError)
  }

  const handleLoad = () => {
    if (done) return
    done = true
    cleanup()
    recordMarkdownImageLoaded(image, src)
    setMarkdownImageStateClass(image, 'loaded')
  }

  const handleError = () => {
    if (done) return
    done = true
    cleanup()
    setMarkdownImageStateClass(image, 'error')
  }

  image.addEventListener('load', handleLoad)
  image.addEventListener('error', handleError)
  image.setAttribute('src', src)

  if (image.complete && image.naturalWidth > 0) {
    handleLoad()
  }
}

function ensureMarkdownImageObserver(): IntersectionObserver | null {
  if (imageObserver) return imageObserver
  if (typeof window === 'undefined' || typeof window.IntersectionObserver === 'undefined') return null

  imageObserver = new window.IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting && entry.intersectionRatio <= 0) continue
        const image = entry.target as HTMLImageElement
        imageObserver?.unobserve(image)
        loadMarkdownImageElement(image)
      }
    },
    {
      root: null,
      rootMargin: '360px 0px',
      threshold: 0.01,
    },
  )

  return imageObserver
}

function markdownImageSourceFromNode(image: HTMLImageElement): string {
  const dataSrc = String(image.getAttribute('data-oc-md-src') || '').trim()
  if (dataSrc) return dataSrc

  const rawStored = String(image.getAttribute('data-oc-md-raw-src') || '').trim()
  if (rawStored) return resolveImageSrcForViewer(rawStored)

  const current = String(image.getAttribute('src') || image.currentSrc || image.src || '').trim()
  return resolveImageSrcForViewer(current)
}

function hydrateMarkdownImage(image: HTMLImageElement) {
  const currentRaw = String(image.getAttribute('data-oc-md-raw-src') || image.getAttribute('src') || '').trim()
  if (currentRaw && !image.getAttribute('data-oc-md-raw-src')) {
    image.setAttribute('data-oc-md-raw-src', currentRaw)
  }

  const src = resolveImageSrcForViewer(currentRaw || markdownImageSourceFromNode(image))
  if (!src) return

  image.setAttribute('data-oc-md-src', src)
  image.setAttribute('decoding', 'async')
  image.setAttribute('draggable', 'false')
  image.setAttribute('loading', 'lazy')

  const hadCachedSize = applyCachedMarkdownImageDimensions(image, src)
  if (!hadCachedSize) {
    applyFallbackMarkdownImagePlaceholder(image)
  }

  if (hasMarkdownImageLoaded(src)) {
    if (String(image.getAttribute('src') || '').trim() !== src) {
      image.setAttribute('src', src)
    }
    setMarkdownImageStateClass(image, 'loaded')
    return
  }

  const state = String(image.dataset.ocMdImageState || '')
  if (state !== 'loading' && state !== 'loaded') {
    if (String(image.getAttribute('src') || '').trim() !== MD_IMAGE_PLACEHOLDER_SRC) {
      image.setAttribute('src', MD_IMAGE_PLACEHOLDER_SRC)
    }
    setMarkdownImageStateClass(image, 'pending')
  }

  const observer = ensureMarkdownImageObserver()
  if (!observer) {
    loadMarkdownImageElement(image)
    return
  }
  observer.observe(image)
}

function hydrateWorkspaceMedia() {
  const root = rootEl.value
  if (!root) return

  const markdownImages = root.querySelectorAll<HTMLImageElement>('img')
  for (const image of markdownImages) {
    hydrateMarkdownImage(image)
  }

  const mediaNodes = root.querySelectorAll<HTMLElement>('video[src], audio[src], video source[src], audio source[src]')
  for (const node of mediaNodes) {
    const src = String(node.getAttribute('src') || '').trim()
    if (!src) continue
    const next = resolveImageSrcForViewer(src)
    if (!next || next === src) continue
    node.setAttribute('src', next)
  }
}

function resolveImageSrcForViewer(rawSrc: string): string {
  const src = String(rawSrc || '').trim()
  if (!src) return ''
  if (src.startsWith('data:') || src.startsWith('blob:')) return src

  const workspaceRoot = String(directoryStore.currentDirectory || '').trim()
  if (!workspaceRoot) return src

  const mediaUrl = resolveWorkspaceMediaUrl(src, {
    workspaceRoot,
    baseFilePath: props.sourcePath,
  })
  return mediaUrl || src
}

function openMarkdownImagePreview(targetImage: HTMLImageElement): boolean {
  const root = rootEl.value
  if (!root) return false

  const images = Array.from(root.querySelectorAll<HTMLImageElement>('img'))
  if (!images.length) return false

  const items: ImageViewerItem[] = []
  let activeIndex = -1

  for (const image of images) {
    const src = markdownImageSourceFromNode(image)
    if (!src) continue

    const alt = String(image.getAttribute('alt') || '').trim()
    const title = String(image.getAttribute('title') || alt).trim()
    const key = String(image.getAttribute('data-oc-md-image-key') || src).trim()
    const index =
      items.push({
        src,
        ...(alt ? { alt } : {}),
        ...(title ? { title } : {}),
        ...(key ? { key } : {}),
      }) - 1

    if (image === targetImage) {
      activeIndex = index
    }
  }

  if (!items.length) return false
  ui.openImageViewer(items, activeIndex >= 0 ? activeIndex : 0)
  return true
}

function openWorkspaceLink(href: string): boolean {
  const workspaceRoot = String(directoryStore.currentDirectory || '').trim()
  if (!workspaceRoot) return false

  const resolved = resolveWorkspaceFileLink(href, {
    workspaceRoot,
    baseFilePath: props.sourcePath,
  })
  if (!resolved?.path) return false

  const query: Record<string, string> = {
    filePath: resolved.path,
  }
  if (resolved.line) query.fileLine = String(resolved.line)
  if (resolved.column) query.fileColumn = String(resolved.column)
  if (resolved.anchor) query.fileAnchor = String(resolved.anchor)

  const fileName = resolved.path.split('/').filter(Boolean).pop() || String(t('nav.files'))
  void workspaceNavigation.openWorkspaceLocation('files', {
    query,
    title: fileName,
    matchKeys: ['filePath'],
  })
  return true
}

function hashString(input: string): string {
  // Small non-cryptographic hash for caching rendered diagrams.
  let h = 5381
  for (let i = 0; i < input.length; i++) {
    h = ((h << 5) + h) ^ input.charCodeAt(i)
  }
  return (h >>> 0).toString(16)
}

let mermaidObserver: IntersectionObserver | null = null
let mermaidController: AbortController | null = null

function resetMermaid() {
  mermaidObserver?.disconnect()
  mermaidObserver = null
  mermaidController?.abort()
  mermaidController = null
}

function hydrateMermaid() {
  resetMermaid()
  const root = rootEl.value
  if (!root) return
  const blocks = Array.from(root.querySelectorAll<HTMLElement>('[data-oc-mermaidblock]'))
  if (!blocks.length) return
  const controller = new AbortController()
  mermaidController = controller
  const theme = document.documentElement.classList.contains('dark') ? 'dark' : 'neutral'

  async function renderBlock(block: HTMLElement) {
    const srcEl = block.querySelector<HTMLElement>('[data-oc-mermaid-source]')
    const renderEl = block.querySelector<HTMLElement>('[data-oc-mermaid-render]')
    if (!srcEl || !renderEl) return
    const source = (srcEl.textContent || '').trim()
    if (!source) return
    const cacheKey = `${theme}:${hashString(source)}`
    if (block.dataset.ocMermaidHash === cacheKey && block.dataset.ocMermaidStatus === 'rendered') return
    block.dataset.ocMermaidStatus = 'pending'
    try {
      const out = await renderMermaid(source, theme, controller.signal)
      if (!out || controller.signal.aborted || !root?.contains(block)) return
      const svg = typeof out === 'string' ? out : out.svg
      if (!svg?.trim()) throw new Error(t('errors.markdown.mermaidEmptyOutput'))
      renderEl.innerHTML = svg
      if (typeof out !== 'string') out.bindFunctions?.(renderEl)
      block.dataset.ocMermaidHash = cacheKey
      block.dataset.ocMermaidStatus = 'rendered'
    } catch {
      if (!controller.signal.aborted) block.dataset.ocMermaidStatus = 'error'
    }
  }

  if (typeof IntersectionObserver === 'undefined') {
    for (const block of blocks) void renderBlock(block)
    return
  }
  mermaidObserver = new IntersectionObserver(
    (entries, observer) => {
      for (const entry of entries) {
        if (!entry.isIntersecting || controller.signal.aborted) continue
        observer.unobserve(entry.target)
        void renderBlock(entry.target as HTMLElement)
      }
    },
    { rootMargin: '360px 0px' },
  )
  for (const block of blocks) mermaidObserver.observe(block)
}

function scheduleHydrateMermaid() {
  if (mermaidTimer) window.clearTimeout(mermaidTimer)
  mermaidTimer = window.setTimeout(() => {
    mermaidTimer = null
    void nextTick(() => hydrateMermaid())
  }, 50)
}

async function handleRootClick(event: MouseEvent) {
  const target = event.target as HTMLElement | null
  if (!target) return

  const image = target.closest('img') as HTMLImageElement | null
  if (image && rootEl.value?.contains(image) && openMarkdownImagePreview(image)) {
    event.preventDefault()
    return
  }

  // Handle TOC anchor links
  const link = target.closest('a')
  if (link) {
    const href = link.getAttribute('href')
    if (href && href.startsWith('#')) {
      event.preventDefault()
      const rawId = href.substring(1)
      if (!rawId) return

      if (scrollToAnchor(rawId)) return
      const decoded = decodeAnchorId(rawId)
      if (props.sourcePath && /^(?:L\d+(?:C\d+)?|\d+(?::\d+)?)$/i.test(decoded)) {
        // Support line-style fragments in markdown previews (e.g. #L42 / #42:3)
        // by treating them as "open current file at location".
        void openWorkspaceLink(`${props.sourcePath}#${decoded}`)
      }
      return
    }

    if (href && openWorkspaceLink(href)) {
      event.preventDefault()
      return
    }
  }

  const btn = target.closest?.('button[data-oc-code-action]') as HTMLButtonElement | null
  if (!btn) return
  const action = btn.getAttribute('data-oc-code-action') || ''
  const block = btn.closest?.('[data-oc-codeblock], [data-oc-mermaidblock]') as HTMLElement | null
  if (!block) return

  if (action === 'toggle') {
    const next = block.getAttribute('data-oc-expanded') === '1' ? '0' : '1'
    block.setAttribute('data-oc-expanded', next)
    return
  }

  if (action === 'copy') {
    const mermaidSrcEl = block.querySelector<HTMLElement>('[data-oc-mermaid-source]')
    const codeEl = block.querySelector('pre code') as HTMLElement | null
    const code = mermaidSrcEl?.textContent ?? codeEl?.textContent ?? ''
    const ok = await copyTextToClipboard(code)
    if (!ok) {
      toasts.push('error', t('common.copyFailed'))
      return
    }

    btn.setAttribute('data-oc-state', 'copied')
    if (copiedTimer) window.clearTimeout(copiedTimer)
    copiedTimer = window.setTimeout(() => {
      btn.removeAttribute('data-oc-state')
      copiedTimer = null
    }, 1200)
  }
}

function positiveInt(raw: unknown): number | undefined {
  const value = Number(raw)
  if (!Number.isFinite(value) || value <= 0) return undefined
  return Math.floor(value)
}

function applyRequestedAnchorReveal() {
  const anchor = String(props.revealAnchor || '').trim()
  if (!anchor) return

  const seq = positiveInt(props.revealRequestSeq) || 0
  const sourcePath = String(props.sourcePath || '').trim()
  const key = `${seq}:${sourcePath}:${anchor}`
  if (key === lastRevealAnchorKey) return

  if (scrollToAnchor(anchor)) {
    lastRevealAnchorKey = key
    return
  }

  if (sourcePath && /^(?:L\d+(?:C\d+)?|\d+(?::\d+)?)$/i.test(anchor) && openWorkspaceLink(`${sourcePath}#${anchor}`)) {
    lastRevealAnchorKey = key
  }
}

onMounted(() => {
  // Event delegation: rendered markdown comes from v-html.
  rootEl.value?.addEventListener('click', handleRootClick)
  scheduleHydrateMermaid()
})

watch(
  () =>
    [
      props.mode,
      props.content,
      props.sourcePath,
      props.stream,
      props.streamDebounceMs,
      nearViewport.value,
      !pane || pane.isVisible.value,
    ] as const,
  () => {
    if (!nearViewport.value || (pane && !pane.isVisible.value)) {
      clearTimer()
      parseController?.abort()
      resetMermaid()
      imageObserver?.disconnect()
      return
    }
    if (props.mode !== 'markdown') {
      resetMermaid()
      clearTimer()
      if (imageObserver) {
        imageObserver.disconnect()
        imageObserver = null
      }
      html.value = []
      return
    }
    if (!props.stream) {
      clearTimer()
      updateNow()
      scheduleHydrateMermaid()
      return
    }
    // A continuous stream must repaint at a bounded rate instead of
    // postponing every render until the stream becomes quiet.
    if (timer !== null) return
    const delay = Math.max(0, Math.floor(props.streamDebounceMs || 0))
    timer = window.setTimeout(() => {
      timer = null
      updateNow()
      scheduleHydrateMermaid()
    }, delay)
  },
  { immediate: true },
)

watch(
  () => [directoryStore.currentDirectory, props.sourcePath] as const,
  () => {
    void nextTick(() => hydrateWorkspaceMedia())
  },
)

watch(
  () => [props.revealRequestSeq, props.revealAnchor, html.value] as const,
  () => {
    void nextTick(() => applyRequestedAnchorReveal())
  },
)

onBeforeUnmount(() => {
  parseController?.abort()
  releaseMarkdownStream(markdownStreamKey)
  resetMermaid()
  clearTimer()
  if (copiedTimer) {
    window.clearTimeout(copiedTimer)
    copiedTimer = null
  }
  if (mermaidTimer) {
    window.clearTimeout(mermaidTimer)
    mermaidTimer = null
  }
  if (imageObserver) {
    imageObserver.disconnect()
    imageObserver = null
  }
  rootEl.value?.removeEventListener('click', handleRootClick)
})
</script>

<template>
  <div v-if="mode === 'plain'" class="whitespace-pre-wrap break-words">{{ content }}</div>
  <div v-else class="relative group">
    <!-- TOC Overlay: Absolute wrapper to ensure it doesn't take up layout space, but stays sticky -->
    <div v-if="toc.length > 0" class="absolute inset-0 pointer-events-none z-10 h-full">
      <div class="sticky top-0 flex justify-end p-2">
        <div
          class="relative pointer-events-auto transition-opacity duration-200"
          :class="[showToc ? 'opacity-100' : 'opacity-60 hover:opacity-100 lg:opacity-0 lg:group-hover:opacity-100']"
          ref="tocMenuRef"
        >
          <button
            type="button"
            class="inline-flex items-center justify-center w-8 h-8 rounded-md border border-border bg-background/80 backdrop-blur shadow-sm text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
            @click="showToc = !showToc"
            :title="t('markdown.tableOfContents')"
          >
            <RiListUnordered class="w-4 h-4" />
          </button>

          <!-- TOC Dropdown -->
          <div
            v-if="showToc"
            class="absolute top-full right-0 mt-1 w-56 max-w-[80vw] max-h-80 overflow-y-auto rounded-md border border-border bg-popover shadow-lg p-1.5 z-50 text-sm origin-top-right animate-in fade-in zoom-in-95 duration-100"
          >
            <nav class="flex flex-col">
              <button
                v-for="item in toc"
                :key="item.id"
                type="button"
                class="text-left px-2 py-1.5 rounded-sm hover:bg-accent hover:text-accent-foreground truncate transition-colors"
                :class="{ 'font-medium': item.level === 1 }"
                :style="{ paddingLeft: `${(item.level - 1) * 0.75 + 0.5}rem` }"
                @click="scrollToHeading(item.id)"
                :title="item.text"
              >
                {{ item.text }}
              </button>
            </nav>
          </div>
        </div>
      </div>
    </div>

    <div ref="rootEl" class="prose prose-sm max-w-none break-words">
      <template v-if="html.length"
        ><div v-for="(part, index) in html" :key="index" class="contents" data-markdown-segment v-html="part"
      /></template>
      <div v-else v-html="deferredHtml" />
    </div>
  </div>
</template>
