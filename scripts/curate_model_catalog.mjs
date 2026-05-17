#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';

const catalogPath = path.resolve(process.cwd(), 'catalog/model-catalog.json');
const catalog = JSON.parse(fs.readFileSync(catalogPath, 'utf8'));
const models = Object.entries(catalog.models || {});

const SOURCE_PREFIX_RE = /^(?:us|eu|au|jp|global|apac)\./;
const BANNED_SOURCE_PREFIXES = [
  'openai.',
  'azure-',
  'google.',
  'cohere-',
  'ai21-',
  'amazon.',
  'anthropic.',
  'duo-chat-',
  'study_gpt-',
  'meta.',
  'mistral.',
  'moonshot.',
  'moonshotai.',
  'qwen.',
  'writer.',
  'zai.',
  'nvidia.',
  'minimax.',
];
const OFFICIAL_ROOTS = new Set([
  'agnes',
  'aion',
  'all',
  'allam',
  'aura',
  'autoglm',
  'bge',
  'brave',
  'c4ai',
  'chatgpt',
  'claude',
  'codegemma',
  'codestral',
  'codex',
  'codellama',
  'command',
  'deepseek',
  'devstral',
  'doubao',
  'e5',
  'elevenlabs',
  'embed',
  'ernie',
  'exa',
  'flux',
  'gemini',
  'gemma',
  'glm',
  'gpt',
  'granite',
  'grok',
  'gte',
  'hunyuan',
  'hy3',
  'ideogram',
  'imagen',
  'inflection',
  'internvl',
  'jais',
  'jamba',
  'kimi',
  'kling',
  'learnlm',
  'lfm',
  'ling',
  'llama',
  'llama3',
  'luma',
  'lyria',
  'magistral',
  'manta',
  'mercury',
  'minimax',
  'mimo',
  'ministral',
  'mistral',
  'mixtral',
  'moonshot',
  'morph',
  'nova',
  'o1',
  'o3',
  'o4',
  'olmo',
  'open',
  'orpheus',
  'palmyra',
  'phi',
  'pixtral',
  'qianfan',
  'qvq',
  'qwen',
  'qwen2',
  'qwen3',
  'qwq',
  'recraft',
  'rerank',
  'ring',
  'riverflow',
  'runway',
  'seed',
  'solar',
  'sonar',
  'step',
  'text',
  'trinity',
  'v0',
  'veo',
  'venice',
  'voxtral',
  'voyage',
  'whisper',
  'yi',
]);
const BLOCKED_TOKENS = [
  /(^|[-:.])(free|optimized)(?:$|[-:.])/,
  /(^|[-:.])tee(?:$|[-:.])/,
  /(^|[-:.])fp\d+(?:$|[-:.])/,
  /(^|[-:.])abliterated(?:$|[-:.])/,
  /(^|[-:.])uncensored(?:$|[-:.])/,
  /(^|[-:.])derestricted(?:$|[-:.])/,
  /(^|[-:.])chimera(?:$|[-:.])/,
  /(^|[-:.])iceblink(?:$|[-:.])/,
  /(^|[-:.])reextract(?:$|[-:.])/,
  /(^|[-:.])steam(?:$|[-:.])/,
  /(^|[-:.])omega(?:$|[-:.])/,
  /(^|[-:.])story(?:$|[-:.])/,
  /(^|[-:.])rp(?:$|[-:.])/,
  /(^|[-:.])rpmax(?:$|[-:.])/,
  /(^|[-:.])slop(?:$|[-:.])/,
  /(^|[-:.])slerp(?:$|[-:.])/,
  /(^|[-:.])malevolence(?:$|[-:.])/,
  /(^|[-:.])safeword(?:$|[-:.])/,
  /(^|[-:.])euryale(?:$|[-:.])/,
  /(^|[-:.])hanami(?:$|[-:.])/,
  /(^|[-:.])lumimaid(?:$|[-:.])/,
  /(^|[-:.])anubis(?:$|[-:.])/,
  /(^|[-:.])cydonia(?:$|[-:.])/,
  /(^|[-:.])forgotten(?:$|[-:.])/,
  /(^|[-:.])abomination(?:$|[-:.])/,
  /(^|[-:.])magnum(?:$|[-:.])/,
  /(^|[-:.])magidonia(?:$|[-:.])/,
  /(^|[-:.])laguna(?:$|[-:.])/,
  /(^|[-:.])dolphin(?:$|[-:.])/,
  /(^|[-:.])longcat(?:$|[-:.])/,
  /(^|[-:.])arliai(?:$|[-:.])/,
  /(^|[-:.])cheaper(?:$|[-:.])/,
  /(^|[-:.])raw(?:$|[-:.])/,
  /(^|[-:.])cs(?:$|[-:.])/,
  /(^|[-:.])exacto(?:$|[-:.])/,
  /(^|[-:.])high-throughput(?:$|[-:.])/,
  /(^|[-:.])tput(?:$|[-:.])/,
  /(^|[-:.])int4(?:$|[-:.])/,
  /(^|[-:.])mixed-ar(?:$|[-:.])/,
  /(^|[-:.])captioner(?:$|[-:.])/,
  /(^|[-:.])original(?:$|[-:.])/,
  /(^|[-:.])sambanova(?:$|[-:.])/,
  /(^|[-:.])terminus(?:$|[-:.])/,
  /(^|[-:.])speciale(?:$|[-:.])/,
  /(^|[-:.])nex(?:$|[-:.])/,
  /(^|[-:.])eva(?:$|[-:.])/,
];
const LLAMA_PATTERNS = [
  /^llama-2-(7b|13b|70b)-chat$/,
  /^llama-3-(8b|70b)-instruct$/,
  /^llama-3\.1-(8b|70b|405b)-instruct$/,
  /^llama-3\.2-(1b|3b)-instruct$/,
  /^llama-3\.2-(11b|90b)-vision-instruct$/,
  /^llama-3\.3-70b-instruct$/,
  /^llama-4-(maverick|scout)$/,
  /^llama-4-(maverick|scout)-17b(-128e|-16e)?-instruct$/,
  /^llama-guard-/,
];
const GEMMA_PATTERNS = [
  /^gemma-2-(2b|9b|27b)-it$/,
  /^gemma-3$/,
  /^gemma-3-(1b|4b|12b|27b)-it$/,
  /^gemma-3n-(e2b|e4b)-it$/,
  /^gemma-4-(26b-a4b|31b)-it$/,
];
const CODEGEMMA_PATTERNS = [/^codegemma-(2b|7b|7b-it)$/];

const DUO_MODEL_MAP = new Map([
  ['duo-chat-sonnet-4-5', 'claude-sonnet-4-5'],
  ['duo-chat-sonnet-4-6', 'claude-sonnet-4-6'],
  ['duo-chat-opus-4-5', 'claude-opus-4-5'],
  ['duo-chat-opus-4-6', 'claude-opus-4-6'],
  ['duo-chat-opus-4-7', 'claude-opus-4-7'],
  ['duo-chat-haiku-4-5', 'claude-haiku-4-5'],
  ['duo-chat-gpt-5', 'gpt-5'],
  ['duo-chat-gpt-5-mini', 'gpt-5-mini'],
  ['duo-chat-gpt-5-codex', 'gpt-5-codex'],
  ['duo-chat-gpt-5-1', 'gpt-5.1'],
  ['duo-chat-gpt-5-2', 'gpt-5.2'],
  ['duo-chat-gpt-5-2-codex', 'gpt-5.2-codex'],
  ['duo-chat-gpt-5-3-codex', 'gpt-5.3-codex'],
  ['duo-chat-gpt-5-4', 'gpt-5.4'],
  ['duo-chat-gpt-5-4-mini', 'gpt-5.4-mini'],
  ['duo-chat-gpt-5-4-nano', 'gpt-5.4-nano'],
]);

function canonicalModelId(rawId) {
  let id = rawId.trim().toLowerCase();
  id = id.replace(/@default$/, '').replace(/-maas$/, '').replace(/:free$/, '');
  id = id.replace(/^study_gpt-chatgpt-4o-latest$/, 'gpt-4o');
  id = id.replace(SOURCE_PREFIX_RE, '');
  id = id.replace(/^amazon\.nova-([a-z0-9-]+)-v1:0$/, 'nova-$1-v1');
  id = id.replace(/^amazon\.nova-([a-z0-9-]+):0$/, 'nova-$1');
  id = id.replace(/^anthropic\./, '');
  id = id.replace(/^openai\./, '');
  id = id.replace(/^azure-/, '');
  id = id.replace(/^google\./, '');
  id = id.replace(/^cohere-command-/, 'command-');
  id = id.replace(/^cohere-embed-v-4-0$/, 'embed-v4.0');
  id = id.replace(/^cohere-embed-v3-/, 'embed-v3-');
  id = id.replace(/^ai21-jamba-/, 'jamba-');
  id = id.replace(/^moonshot-kimi-/, 'kimi-');
  id = id.replace(/^moonshot\./, '');
  id = id.replace(/^moonshotai\./, '');
  id = id.replace(/^zai\./, '');
  id = id.replace(/^minimax\./, '');
  id = id.replace(/^qwen\./, '');
  id = id.replace(/^writer\./, '');
  id = id.replace(/^nvidia\./, '');
  id = id.replace(/^mistral\./, '');
  id = id.replace(/^meta\.llama3-1-(\d+b)-instruct-v1:0$/, 'llama-3.1-$1-instruct');
  id = id.replace(/^meta\.llama3-3-(70b)-instruct-v1:0$/, 'llama-3.3-$1-instruct');
  id = id.replace(/^meta\.llama3-(70b)-instruct$/, 'llama-3-$1-instruct');
  id = id.replace(
    /^meta\.llama4-maverick-17b-instruct-v1:0$/,
    'llama-4-maverick-17b-128e-instruct',
  );
  id = id.replace(
    /^meta\.llama4-scout-17b-instruct-v1:0$/,
    'llama-4-scout-17b-16e-instruct',
  );
  id = id.replace(/^openai\.gpt-oss-(120b|20b)-1:0$/, 'gpt-oss-$1');
  id = id.replace(/^gpt-oss-(120b|20b)-1:0$/, 'gpt-oss-$1');
  id = id.replace(/^(claude-(?:haiku|opus|sonnet)-[^:]+)-v\d+:0$/, '$1');
  id = id.replace(/^claude-3-5-haiku-20241022-v1:0$/, 'claude-haiku-3-5-20241022');
  id = id.replace(/^claude-3-5-sonnet-20241022-v2:0$/, 'claude-sonnet-3-5-20241022');
  id = id.replace(/^claude-3-7-sonnet-20250219-v1:0$/, 'claude-sonnet-3-7-20250219');
  id = id.replace(/^claude-opus-4-6-v1$/, 'claude-opus-4-6');
  id = id.replace(/^deepseek\.r1-v1:0$/, 'deepseek-r1');
  id = id.replace(/^deepseek\.v3-v1:0$/, 'deepseek-v3');
  id = id.replace(/^qwen3-235b-a22b-2507-v1:0$/, 'qwen3-235b-a22b-2507');
  id = id.replace(/^qwen3\.235b-a22b-instruct-2507$/, 'qwen3-235b-a22b-instruct-2507');
  id = id.replace(/^qwen3\.5:397b$/, 'qwen3.5-397b-a17b');
  id = id.replace(/^qwen3-32b-v1:0$/, 'qwen3-32b');
  id = id.replace(/^qwen3-coder-30b-a3b-v1:0$/, 'qwen3-coder-30b-a3b');
  id = id.replace(/^qwen3-coder-480b-a35b-v1:0$/, 'qwen3-coder-480b-a35b-instruct');
  id = id.replace(/^qwen3-coder:480b$/, 'qwen3-coder-480b-a35b-instruct');
  id = id.replace(/^qwen3-next-80b-a3b$/, 'qwen3-next-80b-a3b-instruct');
  id = id.replace(/^qwen3-next:80b$/, 'qwen3-next-80b');
  id = id.replace(/^qwen3-vl:235b$/, 'qwen3-vl-235b-a22b');
  id = id.replace(/^qwen3-vl:235b-instruct$/, 'qwen3-vl-235b-a22b-instruct');
  id = id.replace(/^palmyra-x([45])-v1:0$/, 'palmyra-x$1');
  id = id.replace(/^gpt-oss:(120b|20b)$/, 'gpt-oss-$1');
  id = id.replace(/^glm4\.7$/, 'glm-4.7');
  id = id.replace(/^glm5$/, 'glm-5');
  id = id.replace(/^llama3\.1-8b$/, 'llama-3.1-8b');
  id = id.replace(/^llama3\.3-70b-instruct$/, 'llama-3.3-70b-instruct');
  id = id.replace(/^devstral-2:123b$/, 'devstral-2-123b');
  id = id.replace(/^devstral-small-2:24b$/, 'devstral-small-2-24b');
  id = id.replace(/^ministral-3:14b$/, 'ministral-3-14b');
  id = id.replace(/^ministral-3:8b$/, 'ministral-3-8b');
  id = id.replace(/^ministral-3:3b$/, 'ministral-3-3b');
  id = id.replace(/^pixtral-large-2502-v1:0$/, 'pixtral-large-2502');
  if (DUO_MODEL_MAP.has(id)) {
    id = DUO_MODEL_MAP.get(id);
  }
  return id;
}

function isCanonicalSourceAlias(id) {
  return SOURCE_PREFIX_RE.test(id) || BANNED_SOURCE_PREFIXES.some((prefix) => id.startsWith(prefix));
}

function isAllowedCanonicalModelId(id) {
  if (!id || id.includes('/')) {
    return false;
  }
  if (isCanonicalSourceAlias(id)) {
    return false;
  }
  if (BLOCKED_TOKENS.some((pattern) => pattern.test(id))) {
    return false;
  }
  if (id.startsWith('llama-') && !LLAMA_PATTERNS.some((pattern) => pattern.test(id))) {
    return false;
  }
  if (id.startsWith('llama3') && !/^llama3(\.1-8b|\.3-70b-instruct)$/.test(id)) {
    return false;
  }
  if (id.startsWith('gemma-') && !GEMMA_PATTERNS.some((pattern) => pattern.test(id))) {
    return false;
  }
  if (id.startsWith('codegemma-') && !CODEGEMMA_PATTERNS.some((pattern) => pattern.test(id))) {
    return false;
  }

  const root = (id.match(/^[a-z0-9]+/) || [''])[0];
  return (
    OFFICIAL_ROOTS.has(root) ||
    id.startsWith('all-mini-lm-l6-v2') ||
    id.startsWith('text-embedding-') ||
    id.startsWith('gpt-image-') ||
    id.startsWith('omni-moderation-') ||
    id.startsWith('tts-') ||
    id.startsWith('whisper-') ||
    id.startsWith('dall-e-')
  );
}

function sourcePreferenceScore(rawId, canonicalId, definition) {
  let score = 0;
  if (rawId.toLowerCase() === canonicalId) {
    score += 500;
  }
  if (rawId === rawId.toLowerCase()) {
    score += 25;
  }
  if (SOURCE_PREFIX_RE.test(rawId)) {
    score -= 200;
  }
  for (const prefix of BANNED_SOURCE_PREFIXES) {
    if (rawId.startsWith(prefix)) {
      score -= 150;
    }
  }
  if (/@default$/.test(rawId) || /-maas$/.test(rawId) || /:free$/.test(rawId)) {
    score -= 200;
  }
  if (definition?.display_name) {
    const display = definition.display_name.toLowerCase();
    if (display.includes('bedrock') || display.includes('gitlab') || display.includes('free')) {
      score -= 50;
    }
  }
  return score;
}

function modelRichnessScore(definition) {
  let score = 0;
  if (!definition || typeof definition !== 'object') {
    return score;
  }
  if (definition.display_name) {
    score += 5;
  }
  if (definition.description) {
    score += 5;
  }
  if (definition.context_window_tokens) {
    score += 2;
  }
  if (definition.max_output_tokens) {
    score += 2;
  }
  if (Array.isArray(definition.input?.supported)) {
    score += definition.input.supported.length;
  }
  if (Array.isArray(definition.features?.supported)) {
    score += definition.features.supported.length;
  }
  return score;
}

function compareCandidates(next, current) {
  const sourceDelta = sourcePreferenceScore(next.rawId, next.canonicalId, next.definition)
    - sourcePreferenceScore(current.rawId, current.canonicalId, current.definition);
  if (sourceDelta !== 0) {
    return sourceDelta;
  }
  return modelRichnessScore(next.definition) - modelRichnessScore(current.definition);
}

function mergeDefinitions(primary, fallback) {
  if (Array.isArray(primary) && Array.isArray(fallback)) {
    return Array.from(new Set([...primary, ...fallback]));
  }
  if (Array.isArray(primary) || Array.isArray(fallback)) {
    return structuredClone(primary ?? fallback);
  }
  if (primary && fallback && typeof primary === 'object' && typeof fallback === 'object') {
    const merged = structuredClone(primary);
    for (const [key, fallbackValue] of Object.entries(fallback)) {
      if (!(key in merged)) {
        merged[key] = structuredClone(fallbackValue);
        continue;
      }
      merged[key] = mergeDefinitions(merged[key], fallbackValue);
    }
    return merged;
  }
  return primary ?? fallback;
}

const curatedModels = new Map();
let dropped = 0;

for (const [rawId, definition] of models) {
  const canonicalId = canonicalModelId(rawId);
  if (!isAllowedCanonicalModelId(canonicalId)) {
    dropped += 1;
    continue;
  }

  const candidate = {
    rawId,
    canonicalId,
    definition,
  };
  const current = curatedModels.get(canonicalId);
  if (!current) {
    curatedModels.set(canonicalId, candidate);
    continue;
  }

  const primary = compareCandidates(candidate, current) > 0 ? candidate : current;
  const secondary = primary === candidate ? current : candidate;
  curatedModels.set(canonicalId, {
    rawId: primary.rawId,
    canonicalId,
    definition: mergeDefinitions(primary.definition, secondary.definition),
  });
}

const output = {
  models: Object.fromEntries(
    [...curatedModels.entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([modelId, { definition }]) => [modelId, definition]),
  ),
};

fs.writeFileSync(catalogPath, `${JSON.stringify(output, null, 2)}\n`);

console.log(
  `curated ${models.length} raw models -> ${curatedModels.size} canonical models (${dropped} dropped)`,
);
