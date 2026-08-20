import { readFileSync } from 'node:fs'
import { globSync } from 'node:fs'
import { parse as parseSfc } from '@vue/compiler-sfc'
import { baseParse, NodeTypes } from '@vue/compiler-dom'
import { parse, parseExpression } from '@babel/parser'
import enMessages from '../src/i18n/messages/en-US.ts'

const rootFiles = [...globSync('src/components/settings/**/*.{vue,ts}'), 'src/pages/SettingsPage.vue'].sort()
const vueFiles = rootFiles.filter((file) => file.endsWith('.vue'))
const catalogFiles = globSync('src/i18n/settings-text/*.json').sort()
const overlayFiles = globSync('src/i18n/settings-overlays/*.json').sort()
const errors = []

const visibleAttrs = new Set([
  'title',
  'placeholder',
  'aria-label',
  'tooltip',
  'empty-label',
  'description',
  'label',
  'search-placeholder',
  'input-aria-label',
])
const visibleProps = new Set(['label', 'description', 'title', 'placeholder', 'emptyLabel', 'tooltip', 'summary'])
const technicalTemplateLiterals = new Set([
  '/v1/mcp/tunnel_id',
  'iss',
  'model-name',
  'OIDC',
  'PID',
  'shell',
  'TUI',
  'Top K',
  'Top P',
  'agena:tools',
  'example.com or 127.0.0.1:8080',
  'git push *',
  'npm version or semver',
  'openai',
  'shell or agena.web.fetch',
])
const technicalScriptLiterals = new Set([
  '',
  'AICORE_SERVICE_KEY',
  'API',
  'GitLab.com',
  'http://localhost:1455/auth/callback',
  'https://gitlab.com',
  'text',
  'tools_list',
  'tools_search',
  'tools_help',
  'tools_tags',
  'tools_call',
  'plugins_list',
  'plugins_search',
  'plugins_tags',
  'providers',
  'permission',
  'plugins',
  'runtime.providers.client_versions',
  'session.compaction',
  'ui',
  'tracing',
  'harnesses',
  'off',
  'error',
  'warn',
  'info',
  'debug',
  'trace',
])

function normalize(value) {
  return String(value || '')
    .replace(/\s+/g, ' ')
    .trim()
}
function isCss(value) {
  return /(?:^|\s)(?:flex|grid|relative|absolute|fixed|hidden|block|inline|items-|justify-|gap-|p[trblxy]?\-|m[trblxy]?\-|w-|h-|min-|max-|border|bg-|text-|font-|rounded|overflow|opacity|ring|animate-|hover:|focus:|dark:|sm:|md:|lg:|xl:)/.test(
    value,
  )
}
function looksUserFacing(value, allowlist) {
  const normalized = normalize(value)
  if (!/[A-Za-z]{2}/.test(normalized) || allowlist.has(normalized) || isCss(normalized)) return false
  if (/^(?:https?:\/\/|\/|[-_a-z0-9]+\.[-_a-z0-9./]+$)/i.test(normalized)) return false
  return true
}
function memberName(node) {
  if (!node) return ''
  if (node.type === 'Identifier') return node.name
  if (node.type === 'MemberExpression') return `${memberName(node.object)}.${memberName(node.property)}`
  return ''
}
function propertyName(node) {
  return node?.type === 'Identifier' ? node.name : node?.type === 'StringLiteral' ? node.value : ''
}
function insideTranslationCall(ancestors) {
  return ancestors.some(
    (node) => node?.type === 'CallExpression' && ['$st', 'st', 't', 'te'].includes(memberName(node.callee)),
  )
}
function auditOutputExpression(expression, file, lineOffset = 0) {
  let ast
  try {
    ast = parseExpression(expression, { plugins: ['typescript'] })
  } catch {
    return
  }
  function visitOutput(node, ancestors = []) {
    if (!node || typeof node !== 'object' || insideTranslationCall(ancestors)) return
    if (node.type === 'StringLiteral') {
      if (looksUserFacing(node.value, technicalTemplateLiterals)) {
        errors.push(`${file}:${lineOffset + (node.loc?.start.line || 1)} visible expression literal: ${node.value}`)
      }
      return
    }
    if (node.type === 'ConditionalExpression') {
      visitOutput(node.consequent, [...ancestors, node])
      visitOutput(node.alternate, [...ancestors, node])
      return
    }
    if (node.type === 'LogicalExpression') {
      visitOutput(node.right, [...ancestors, node])
      return
    }
    if (node.type === 'TemplateLiteral' && node.quasis.some((part) => /[A-Za-z]{2}/.test(part.value.cooked || ''))) {
      errors.push(`${file}:${lineOffset + (node.loc?.start.line || 1)} visible template literal is not localized`)
    }
  }
  visitOutput(ast)
}

for (const file of vueFiles) {
  const source = readFileSync(file, 'utf8')
  const { descriptor } = parseSfc(source)
  const template = descriptor.template?.content
  if (!template) continue
  const ast = baseParse(template)
  function walk(node) {
    if (node.type === NodeTypes.TEXT) {
      const value = normalize(node.content)
      if (looksUserFacing(value, technicalTemplateLiterals)) {
        errors.push(`${file}:${node.loc.start.line} raw template text: ${value}`)
      }
    }
    if (node.type === NodeTypes.INTERPOLATION && typeof node.content?.content === 'string') {
      auditOutputExpression(node.content.content, file, node.content.loc.start.line - 1)
    }
    if (node.type === NodeTypes.ELEMENT) {
      for (const prop of node.props || []) {
        if (prop.type === NodeTypes.ATTRIBUTE && visibleAttrs.has(prop.name) && prop.value?.content) {
          const value = normalize(prop.value.content)
          if (looksUserFacing(value, technicalTemplateLiterals)) {
            errors.push(`${file}:${prop.loc.start.line} raw ${prop.name} attribute: ${value}`)
          }
        }
        if (
          prop.type === NodeTypes.DIRECTIVE &&
          prop.name === 'bind' &&
          prop.arg?.type === NodeTypes.SIMPLE_EXPRESSION &&
          visibleAttrs.has(prop.arg.content) &&
          prop.exp?.type === NodeTypes.SIMPLE_EXPRESSION
        ) {
          auditOutputExpression(prop.exp.content, file, prop.exp.loc.start.line - 1)
        }
      }
    }
    for (const child of node.children || []) walk(child)
    if (node.branches) for (const branch of node.branches) walk(branch)
  }
  walk(ast)

  const script = descriptor.scriptSetup?.content
  if (!script) continue
  const scriptAst = parse(script, { sourceType: 'module', plugins: ['typescript', 'topLevelAwait'] })
  function auditScript(node, parent = null, ancestors = []) {
    if (!node || typeof node !== 'object') return
    if (node.type === 'StringLiteral' && !insideTranslationCall(ancestors)) {
      const value = node.value
      let visible = false
      if (parent?.type === 'ObjectProperty' && parent.value === node && visibleProps.has(propertyName(parent.key)))
        visible = true
      if (parent?.type === 'NewExpression' && memberName(parent.callee) === 'Error') visible = true
      if (parent?.type === 'CallExpression') {
        const name = memberName(parent.callee)
        const index = parent.arguments?.indexOf(node) ?? -1
        if (name === 'window.confirm' || (name === 'toasts.push' && index >= 1)) visible = true
      }
      if (visible && looksUserFacing(value, technicalScriptLiterals)) {
        errors.push(`${file}:${node.loc?.start.line || 1} raw script UI literal: ${value}`)
      }
    }
    for (const [key, child] of Object.entries(node)) {
      if (['loc', 'start', 'end', 'extra'].includes(key)) continue
      if (Array.isArray(child)) for (const item of child) auditScript(item, node, [...ancestors, node])
      else if (child && typeof child === 'object') auditScript(child, node, [...ancestors, node])
    }
  }
  auditScript(scriptAst)
}

const sourcePattern = /(?:\$st|\bst)\(\s*'((?:\\'|\\\\|[^'])*)'/g
const referenced = new Set()
for (const file of rootFiles) {
  const source = readFileSync(file, 'utf8')
  for (const match of source.matchAll(sourcePattern)) {
    referenced.add(match[1].replaceAll("\\'", "'").replaceAll('\\\\', '\\'))
  }
}
const catalogs = Object.fromEntries(
  catalogFiles.map((file) => [file.split('/').at(-1).replace('.json', ''), JSON.parse(readFileSync(file, 'utf8'))]),
)
const english = catalogs['en-US'] || {}
if (JSON.stringify([...referenced].sort()) !== JSON.stringify(Object.keys(english).sort())) {
  const missing = [...referenced].filter((key) => !(key in english))
  const extra = Object.keys(english).filter((key) => !referenced.has(key))
  errors.push(`en-US source catalog mismatch; missing=${missing.join(' | ')} extra=${extra.join(' | ')}`)
}
const placeholderPattern = /\{([A-Za-z0-9_]+)\}/g
function placeholders(value) {
  return [...String(value).matchAll(placeholderPattern)].map((match) => match[1]).sort()
}
for (const [locale, catalog] of Object.entries(catalogs)) {
  const missing = [...referenced].filter((key) => !(key in catalog))
  const extra = Object.keys(catalog).filter((key) => !referenced.has(key))
  if (missing.length || extra.length)
    errors.push(`${locale} catalog keys mismatch; missing=${missing.length} extra=${extra.length}`)
  for (const key of referenced) {
    const value = catalog[key]
    if (typeof value !== 'string' || !value.trim()) errors.push(`${locale} has an empty translation for ${key}`)
    if (JSON.stringify(placeholders(value)) !== JSON.stringify(placeholders(key))) {
      errors.push(`${locale} placeholder mismatch for ${key}: ${value}`)
    }
  }
}

function flatten(value, prefix = '', output = {}) {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(value)) flatten(child, prefix ? `${prefix}.${key}` : key, output)
  } else if (typeof value === 'string') output[prefix] = value
  return output
}
const traditionalKeys = new Set()
const traditionalPattern =
  /(?:\bt|\bte)\(\s*['"](settings\.[A-Za-z0-9_.-]+)['"]|labelKey:\s*['"](settings\.[A-Za-z0-9_.-]+)['"]/g
for (const file of rootFiles) {
  const source = readFileSync(file, 'utf8')
  for (const match of source.matchAll(traditionalPattern)) traditionalKeys.add(match[1] || match[2])
}
const enFlat = flatten(enMessages)
for (const file of overlayFiles) {
  const locale = file.split('/').at(-1).replace('.json', '')
  const overlay = flatten({ settings: JSON.parse(readFileSync(file, 'utf8')) })
  const missing = [...traditionalKeys].filter((key) => key in enFlat && !(key in overlay))
  if (missing.length) errors.push(`${locale} settings overlay misses current keys: ${missing.join(' | ')}`)
}

const sameAsEnglishAllowlist = new Set([
  'Alpha',
  'Beta',
  'Bedrock SigV4',
  'Cline API',
  'Configuration',
  'Driver',
  'Experimental',
  'GitHub Copilot',
  'GitLab',
  'GitLab API',
  'Global',
  'Google ADC',
  'Mode',
  'No',
  'OpenAI ChatGPT',
  'Plugins',
  'Provenance',
  'SAP AI Core',
  'Source',
  'Unicode',
  'global',
  'owner/repository@v0.1.0',
  'tool_calling, reasoning, streaming',
  '{adapter_id} / {id}',
  '{path} · {target_path}',
  '{provider_id} · {defaultLabel}',
  '{tool_name} · {qualifier}',
  '{type} {index}',
])

const contextualGlossary = {
  'ar-SA': {
    'All states': 'جميع حالات التنفيذ',
    Driver: 'برنامج التشغيل',
    Adapter: 'مهايئ',
    Providers: 'مقدّمو الخدمات',
    'Tool harnesses': 'بيئات تنفيذ الأدوات',
    Global: 'عام',
    Workspace: 'مساحة العمل',
    'Provider configuration': 'إعدادات مزوّد الخدمة',
    'Save changes': 'حفظ التغييرات',
    'Discard changes': 'التراجع عن التغييرات',
    'Apply JSON': 'تطبيق JSON',
    Dark: 'داكن',
    Light: 'فاتح',
    'Clear approval model': 'مسح نموذج الموافقة',
    'Plugin Marketplace': 'متجر الإضافات',
  },
  'es-ES': {
    'All states': 'Todos los estados de ejecución',
    Driver: 'Controlador',
    Adapter: 'Adaptador',
    Providers: 'Proveedores',
    'Tool harnesses': 'Entornos de ejecución de herramientas',
    Global: 'Global',
    Workspace: 'Espacio de trabajo',
    'Provider configuration': 'Configuración del proveedor',
    'Save changes': 'Guardar cambios',
    'Discard changes': 'Descartar cambios',
    'Apply JSON': 'Aplicar JSON',
    Dark: 'Oscuro',
    Light: 'Claro',
    'Clear approval model': 'Borrar modelo de aprobación',
    'Plugin Marketplace': 'Marketplace de plugins',
  },
  'fr-FR': {
    'All states': "Tous les statuts d'exécution",
    Driver: 'Pilote',
    Adapter: 'Adaptateur',
    Providers: 'Fournisseurs',
    'Tool harnesses': 'Environnements d’exécution des outils',
    Global: 'Global',
    Workspace: 'Espace de travail',
    'Provider configuration': 'Configuration du fournisseur',
    'Save changes': 'Enregistrer les modifications',
    'Discard changes': 'Annuler les modifications',
    'Apply JSON': 'Appliquer le JSON',
    Dark: 'Sombre',
    Light: 'Clair',
    'Clear approval model': 'Effacer le modèle d’approbation',
    'Plugin Marketplace': 'Place de marché des plugins',
  },
  'hi-IN': {
    'All states': 'सभी निष्पादन स्थितियाँ',
    Driver: 'ड्राइवर',
    Adapter: 'एडाप्टर',
    Providers: 'प्रदाता',
    'Tool harnesses': 'टूल निष्पादन परिवेश',
    Global: 'वैश्विक',
    Workspace: 'कार्यक्षेत्र',
    'Provider configuration': 'एआई प्रदाता कॉन्फ़िगरेशन',
    'Save changes': 'परिवर्तन सहेजें',
    'Discard changes': 'परिवर्तन त्यागें',
    'Apply JSON': 'JSON लागू करें',
    Dark: 'गहरा',
    Light: 'हल्का',
    'Clear approval model': 'अनुमोदन मॉडल हटाएँ',
    'Plugin Marketplace': 'प्लगइन मार्केटप्लेस',
  },
  'pt-BR': {
    'All states': 'Todos os status de execução',
    Driver: 'Driver',
    Adapter: 'Adaptador',
    Providers: 'Provedores',
    'Tool harnesses': 'Ambientes de execução de ferramentas',
    Global: 'Global',
    Workspace: 'Espaço de trabalho',
    'Provider configuration': 'Configuração do provedor',
    'Save changes': 'Salvar alterações',
    'Discard changes': 'Descartar alterações',
    'Apply JSON': 'Aplicar JSON',
    Dark: 'Escuro',
    Light: 'Claro',
    'Clear approval model': 'Limpar modelo de aprovação',
    'Plugin Marketplace': 'Marketplace de plugins',
  },
  'zh-CN': {
    'All states': '所有状态',
    Driver: '驱动',
    Adapter: '适配器',
    Providers: '服务商',
    'Tool harnesses': '工具执行环境',
    Global: '全局',
    Workspace: '工作区',
    'Provider configuration': '服务商配置',
    'Save changes': '保存修改',
    'Discard changes': '放弃修改',
    'Apply JSON': '应用 JSON',
    Dark: '深色',
    Light: '浅色',
    'Clear approval model': '清除审批模型',
    'Plugin Marketplace': '插件市场',
  },
}

for (const [locale, catalog] of Object.entries(catalogs)) {
  if (locale === 'en-US') continue
  for (const source of referenced) {
    if (catalog[source] === english[source] && !sameAsEnglishAllowlist.has(source)) {
      errors.push(`${locale} leaves user-facing Settings text in English: ${source}`)
    }
  }
  for (const [source, expected] of Object.entries(contextualGlossary[locale] || {})) {
    if (catalog[source] !== expected) {
      errors.push(`${locale} violates Settings glossary for ${source}: expected=${expected} actual=${catalog[source]}`)
    }
  }
}

const coreTranslatedSources = [
  'Advanced settings',
  'Allow',
  'Disabled',
  'Model Catalog',
  'Models & Providers',
  'Permission Studio',
  'Plugin Workbench',
  'Provider Studio',
  'Refresh',
  'Save',
  'Tool harnesses',
  'Workspace',
]
for (const [locale, catalog] of Object.entries(catalogs)) {
  if (locale === 'en-US') continue
  for (const source of coreTranslatedSources) {
    if (!(source in catalog)) continue
    if (catalog[source] === source) errors.push(`${locale} leaves core Settings source untranslated: ${source}`)
  }
}

if (errors.length) {
  console.error(`Settings I18N check failed with ${errors.length} issue(s):`)
  for (const error of errors) console.error(`- ${error}`)
  process.exit(1)
}
console.log(`Settings I18N check passed: ${referenced.size} source strings, ${traditionalKeys.size} settings.* keys`)
