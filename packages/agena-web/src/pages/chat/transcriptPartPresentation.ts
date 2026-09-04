import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'
import type { ToolDetailSection } from '@/stores/chat/api'
import type { JsonValue } from '@/types/json'

export type JsonRecord = Record<string, JsonValue>

export type PartStatusPresentation = {
  icon: string
  label: string
  tone: 'pending' | 'success' | 'warning' | 'danger' | 'muted'
  spinning: boolean
  terminal: boolean
}

export type OperationPermissionPresentation = {
  requestId: string
  pending: boolean
  status: string
  action: string
  reason: string
  explanation: string
  replyReason: string
  provenance: string
}

export type OperationPresentation = {
  title: string
  summary: string
  toolName: string
  input: JsonValue | null
  inputMarkdown: string
  error: string
  stdout: string
  structured: JsonValue | null
  rawOutput: JsonValue | null
  outputText: string
  managedOutputs: JsonValue | null
  truncated: boolean
  blocks: JsonRecord[]
  presentationBlocks: JsonRecord[]
  attachments: AttachmentPresentation[]
  metadata: JsonRecord
  outputMetadata: JsonRecord
  durationMs: number | null
  userInputs: InteractionPresentation[]
  permissions: OperationPermissionPresentation[]
}

export type OperationSectionValues = Partial<Record<ToolDetailSection, JsonValue>>

export type AttachmentPresentation = {
  key: string
  label: string
  kind: string
  mime: string
  url: string
  path: string
  sizeBytes: number | null
  width: number | null
  height: number | null
  durationMs: number | null
  pageCount: number | null
}

export type SkillPresentation = {
  name: string
  description: string
  instructions: string
  source: string
  contentHash: string
}

export type InteractionOptionPresentation = { label: string; description: string }
export type InteractionQuestionPresentation = {
  questionId: string
  header: string
  question: string
  multiple: boolean
  allowCustom: boolean
  options: InteractionOptionPresentation[]
}

export type InteractionPresentation = {
  requestId: string
  title: string
  bodyMarkdown: string
  kind: string
  pending: boolean
  reply: JsonValue | null
  questions: InteractionQuestionPresentation[]
}

export type ErrorPresentation = {
  message: string
  code: string
  category: string
  responsibility: string
  impact: string
  recovery: string
  retry: string
  correlationId: string
  details: JsonRecord
}

export function isJsonRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function jsonRecord(value: unknown): JsonRecord {
  return isJsonRecord(value) ? value : {}
}

export function jsonArray(value: unknown): JsonValue[] {
  return Array.isArray(value) ? value : []
}

export function stringValue(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

export function firstString(source: JsonRecord, keys: readonly string[]): string {
  for (const key of keys) {
    const candidate = stringValue(source[key])
    if (candidate) return candidate
  }
  return ''
}

function booleanValue(source: JsonRecord, keys: readonly string[]): boolean {
  return keys.some((key) => source[key] === true)
}

export function attentionRequestId(payload: JsonValue | null | undefined): string {
  const root = jsonRecord(payload)
  const properties = jsonRecord(root.properties)
  const propertyRequest = jsonRecord(properties.request)
  const request = jsonRecord(root.request)
  return (
    firstString(properties, ['id', 'request_id']) ||
    firstString(propertyRequest, ['request_id', 'id']) ||
    firstString(root, ['request_id', 'id']) ||
    firstString(request, ['request_id', 'id'])
  )
}

export type AttentionPresentationSource = {
  kind: 'permission' | 'question'
  payload: JsonValue
}

function numericValue(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

export function prettyJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value ?? '')
  }
}

export function partStatusPresentation(statusInput: string): PartStatusPresentation {
  const status = String(statusInput || '')
    .trim()
    .toLowerCase()
  if (status === 'in_progress' || status === 'running') {
    return { icon: '⠋', label: 'running', tone: 'pending', spinning: true, terminal: false }
  }
  if (status === 'completed') {
    return { icon: '●', label: 'completed', tone: 'success', spinning: false, terminal: true }
  }
  if (status === 'policy_denied') {
    return { icon: '⊘', label: 'policy denied', tone: 'warning', spinning: false, terminal: true }
  }
  if (status === 'user_declined') {
    return { icon: '–', label: 'declined', tone: 'muted', spinning: false, terminal: true }
  }
  if (status === 'capability_unavailable' || status === 'tool_unavailable') {
    return { icon: '◇', label: 'unavailable', tone: 'warning', spinning: false, terminal: true }
  }
  if (status === 'failed' || status === 'error') {
    return { icon: '×', label: 'failed', tone: 'danger', spinning: false, terminal: true }
  }
  if (status === 'cancelled' || status === 'canceled') {
    return { icon: '–', label: 'cancelled', tone: 'muted', spinning: false, terminal: true }
  }
  if (status === 'pending') {
    return { icon: '○', label: 'pending', tone: 'pending', spinning: false, terminal: false }
  }
  return { icon: '', label: '', tone: 'muted', spinning: false, terminal: false }
}

/** Decode the API StructuredValue/StructuredObject envelope into ordinary JSON. */
export function decodeStructuredValue(value: JsonValue): JsonValue {
  if (Array.isArray(value)) return value.map((item) => decodeStructuredValue(item))
  if (!isJsonRecord(value)) return value

  const kind = stringValue(value.kind)
  if (kind === 'null') return null
  if (kind === 'text' || kind === 'number') return value.value ?? ''
  if (kind === 'integer' || kind === 'boolean') return value.value ?? null
  if (kind === 'array') return jsonArray(value.items).map((item) => decodeStructuredValue(item))
  if (kind === 'object' || Array.isArray(value.fields)) {
    const output: JsonRecord = {}
    for (const rawField of jsonArray(value.fields)) {
      const field = jsonRecord(rawField)
      const name = stringValue(field.name)
      if (!name) continue
      output[name] = decodeStructuredValue(field.value ?? null)
    }
    if (Object.keys(output).length || kind === 'object' || Array.isArray(value.fields)) return output
  }

  const output: JsonRecord = {}
  for (const [key, nested] of Object.entries(value)) output[key] = decodeStructuredValue(nested)
  return output
}

function inlineValue(value: JsonValue): string {
  if (value === null) return '`null`'
  if (typeof value === 'string') return value.includes('\n') ? value : `\`${value}\``
  if (typeof value === 'number' || typeof value === 'boolean') return `\`${String(value)}\``
  return ''
}

function markdownLines(value: JsonValue, depth: number): string[] {
  const indent = '  '.repeat(depth)
  if (Array.isArray(value)) {
    return value.flatMap((item) => {
      const inline = inlineValue(item)
      if (inline) return [`${indent}- ${inline}`]
      return [`${indent}-`, ...markdownLines(item, depth + 1)]
    })
  }
  if (isJsonRecord(value)) {
    return Object.entries(value).flatMap(([key, nested]) => {
      const inline = inlineValue(nested)
      if (inline) return [`${indent}- **${key}**: ${inline}`]
      return [`${indent}- **${key}**`, ...markdownLines(nested, depth + 1)]
    })
  }
  const inline = inlineValue(value)
  return inline ? [`${indent}- ${inline}`] : []
}

export function structuredValueMarkdown(value: JsonValue): string {
  return markdownLines(decodeStructuredValue(value), 0).join('\n')
}

function problemMessage(value: JsonValue): string {
  const problem = jsonRecord(value)
  const user = jsonRecord(problem.user)
  return firstString(user, ['fallback']) || firstString(problem, ['message'])
}

function operationFailureMessage(value: JsonValue): string {
  const error = jsonRecord(value)
  return problemMessage(error.failure) || problemMessage(error.problem) || firstString(error, ['message', 'detail'])
}

function normalizedBlockText(value: string): string {
  return value.replace(/\r\n/g, '\n').trim()
}

function splitOperationStdout(blocks: JsonRecord[]): { blocks: JsonRecord[]; stdout: string[] } {
  const outputBlocks: JsonRecord[] = []
  const stdout: string[] = []

  for (const block of blocks) {
    const kind = firstString(block, ['type', 'kind']).toLowerCase()
    if (kind === 'log' && firstString(block, ['stream']).toLowerCase() === 'stdout') {
      const text = firstString(block, ['text', 'markdown', 'content'])
      if (text) stdout.push(text)
      continue
    }

    if (kind === 'command') {
      const text = firstString(block, ['stdout'])
      if (text) stdout.push(text)
      const remainder: JsonRecord = { ...block }
      delete remainder.stdout
      const hasVisibleRemainder = Boolean(
        firstString(remainder, ['command', 'cwd', 'stderr']) || typeof remainder.exit_code === 'number',
      )
      if (hasVisibleRemainder) outputBlocks.push(remainder)
      continue
    }

    outputBlocks.push(block)
  }

  return { blocks: outputBlocks, stdout }
}

function operationStdout(blockStdout: string[]): { text: string; normalized: Set<string> } {
  const candidates = blockStdout
  const normalized = new Set<string>()
  const output: string[] = []
  for (const candidate of candidates) {
    const key = normalizedBlockText(candidate)
    if (!key || normalized.has(key)) continue
    normalized.add(key)
    output.push(candidate.trim())
  }
  const text = output.join('\n\n')
  if (text) normalized.add(normalizedBlockText(text))
  return { text, normalized }
}

function isTextOnlyStructured(value: JsonValue): boolean {
  if (!isJsonRecord(value)) return typeof value === 'string'
  return Object.keys(value).length === 1 && typeof value.text === 'string'
}

function attachmentFromRecord(value: JsonValue, index: number): AttachmentPresentation | null {
  const item = jsonRecord(value)
  if (!Object.keys(item).length) return null
  const source = jsonRecord(item.source)
  const sourceKind = firstString(source, ['source'])
  const mime = firstString(item, ['mime'])
  const path = firstString(source, ['path']) || firstString(item, ['path'])
  const directUrl = firstString(source, ['data_url', 'url']) || firstString(item, ['data_url', 'url'])
  const base64 =
    (sourceKind === 'base64' ? firstString(source, ['data']) : firstString(source, ['base64'])) ||
    firstString(item, ['base64'])
  const fileId = firstString(source, ['file_id']) || firstString(item, ['file_id'])
  const url = directUrl || (base64 && mime ? `data:${mime};base64,${base64}` : '') || path
  const label = firstString(item, ['title', 'filename', 'name']) || path || fileId || mime || `attachment-${index + 1}`
  return {
    key: firstString(item, ['sha256']) || url || `${label}:${index}`,
    label,
    kind: firstString(item, ['kind']),
    mime,
    url,
    path,
    sizeBytes: numericValue(item.size_bytes),
    width: numericValue(item.width),
    height: numericValue(item.height),
    durationMs: numericValue(item.duration_ms),
    pageCount: numericValue(item.page_count),
  }
}

function toolCallView(content: JsonRecord, presentationValue: JsonValue): JsonRecord {
  const presentation = jsonRecord(presentationValue)
  const title = firstString(presentation, ['title'])
  const summary = firstString(presentation, ['summary'])
  const invocation = {
    name: firstString(content, ['name']) || 'unknown',
    plugin_name: content.plugin,
    input: jsonRecord(content.input),
    tool_api_call: jsonRecord(content.tool_api_call),
  }
  const blocks = jsonArray(presentation.blocks)
  return {
    call_id: content.call_id ?? 0,
    invocation,
    title,
    summary,
    blocks,
    user_input: jsonRecord(content.user_input),
    authorization: jsonRecord(content.authorization),
    metadata: jsonRecord(content.metadata),
    error: content.error ?? null,
    lifecycle: jsonRecord(content.lifecycle),
    output: content.output ?? null,
  }
}

export function operationPresentation(
  part: TranscriptDisplayPart,
  sectionValues: OperationSectionValues = {},
): OperationPresentation {
  const sourceContent = jsonRecord(part.source.agenaContent)
  const content: JsonRecord = { ...sourceContent }
  const hasSection = (section: ToolDetailSection): boolean =>
    Object.prototype.hasOwnProperty.call(sectionValues, section) && sectionValues[section] !== undefined
  if (hasSection('metadata')) content.metadata = sectionValues.metadata as JsonValue
  if (hasSection('input')) content.input = sectionValues.input as JsonValue
  if (hasSection('output')) content.output = sectionValues.output as JsonValue
  if (hasSection('output_metadata')) {
    const output = jsonRecord(content.output)
    content.output = { ...output, metadata: sectionValues.output_metadata as JsonValue }
  }
  const presentationValue = hasSection('presentation')
    ? (sectionValues.presentation as JsonValue)
    : (part.source.agenaPresentation ?? null)
  const operation = toolCallView(content, presentationValue)
  const invocation = jsonRecord(operation.invocation)
  const output = jsonRecord(operation.output)
  const canonicalInput = jsonRecord(content.input)
  const encodedInput = Object.keys(canonicalInput).length ? canonicalInput : jsonRecord(invocation.input)
  const input = Object.keys(encodedInput).length ? decodeStructuredValue(encodedInput) : null
  const toolName = firstString(content, ['name']) || firstString(invocation, ['name']) || stringValue(part.source.tool)
  const rawStructured = output.payload ?? null
  const rawOutput: JsonValue | null = operation.output === null ? null : { ...output }
  if (isJsonRecord(rawOutput)) delete rawOutput.metadata
  const projectedBlocks = jsonArray(operation.blocks)
    .map(jsonRecord)
    .filter((item) => Object.keys(item).length > 0)
    .filter(
      (item, index, values) => values.findIndex((candidate) => prettyJson(candidate) === prettyJson(item)) === index,
    )
  const splitBlocks = splitOperationStdout(projectedBlocks)
  const blocks = splitBlocks.blocks
  const stdout = operationStdout(splitBlocks.stdout)
  const outputText = firstString(output, ['text'])
  // A stdout ViewBlock is the human projection of a text-only raw payload.
  // Keep the durable payload intact, but do not render the same text again in
  // the separate structured Output section.
  const structured =
    rawStructured !== null && !(stdout.text && isTextOnlyStructured(rawStructured)) ? rawStructured : null

  const attachments = jsonArray(output.attachments)
    .map(attachmentFromRecord)
    .filter((item): item is AttachmentPresentation => Boolean(item))
    .filter((item, index, list) => list.findIndex((candidate) => candidate.key === item.key) === index)

  const startMs = numericValue(jsonRecord(operation.lifecycle).start_ms) ?? part.source.time?.start ?? null
  const endMs = numericValue(jsonRecord(operation.lifecycle).end_ms) ?? part.source.time?.end ?? null
  const userInput = jsonRecord(operation.user_input)
  const userInputs = jsonArray(userInput.requests)
    .map((raw) => {
      const record = jsonRecord(raw)
      const request = jsonRecord(record.request)
      if (!Object.keys(request).length) return null
      return userInputPresentationFromRequest(request, record.reply ?? null, part.title)
    })
    .filter((item): item is InteractionPresentation => Boolean(item))
  const authorization = jsonRecord(operation.authorization)
  const permissions = jsonArray(authorization.permissions).map((raw) => {
    const permission = jsonRecord(raw)
    const request = jsonRecord(permission.request)
    const hasReply = permission.reply !== null && permission.reply !== undefined
    const reply = jsonRecord(permission.reply)
    const action = jsonRecord(request.action)
    const replyKind = firstString(reply, ['kind'])
    const status =
      replyKind === 'allow_once'
        ? 'Allowed once'
        : replyKind === 'allow_always'
          ? 'Allowed persistently'
          : replyKind === 'auto_approve'
            ? 'Approved automatically'
            : replyKind === 'deny_once' || replyKind === 'reject'
              ? 'Denied once'
              : replyKind === 'deny_always' || replyKind === 'reject_always'
                ? 'Denied persistently'
                : replyKind
                  ? `Replied (${replyKind})`
                  : 'Awaiting user approval'
    return {
      requestId: firstString(request, ['request_id']),
      pending: !hasReply,
      status,
      action: permissionActionLabel(action),
      reason: firstString(request, ['reason']),
      explanation: firstString(request, ['explanation']),
      replyReason: firstString(reply, ['reason']),
      provenance: [firstString(request, ['source']), firstString(request, ['scope'])].filter(Boolean).join(' · '),
    }
  })

  return {
    title: firstString(operation, ['title']) || part.title,
    summary: firstString(operation, ['summary']) || part.summary,
    toolName,
    input,
    inputMarkdown: input === null ? '' : structuredValueMarkdown(input),
    error: operationFailureMessage(operation.error ?? null),
    stdout: stdout.text,
    structured,
    rawOutput,
    outputText,
    managedOutputs: output.managed_outputs ?? null,
    truncated: output.truncated === true,
    blocks,
    presentationBlocks: blocks,
    attachments,
    metadata: jsonRecord(content.metadata),
    outputMetadata: jsonRecord(output.metadata),
    durationMs: startMs !== null && endMs !== null && endMs >= startMs ? endMs - startMs : null,
    userInputs,
    permissions,
  }
}

export function partInteractionRequestIds(part: TranscriptDisplayPart): string[] {
  if (part.kind !== 'operation') return []
  const operation = operationPresentation(part)
  return [...operation.userInputs.map((item) => item.requestId), ...operation.permissions.map((item) => item.requestId)]
    .filter(Boolean)
    .filter((id, index, values) => values.indexOf(id) === index)
}

export function attachmentPresentations(part: TranscriptDisplayPart): AttachmentPresentation[] {
  const content = jsonRecord(part.source.agenaContent)
  const attachments = jsonArray(content.attachments)
  const projected = attachments
    .map(attachmentFromRecord)
    .filter((item): item is AttachmentPresentation => Boolean(item))
  if (projected.length) return projected

  const mime = stringValue(part.source.mime) || firstString(content, ['mime'])
  const path = stringValue(part.source.serverPath) || firstString(content, ['path'])
  const url = stringValue(part.source.url) || path
  const label =
    stringValue(part.source.filename) || firstString(content, ['name', 'title']) || path || mime || 'attachment'
  return [
    {
      key: url || label,
      label,
      kind: firstString(content, ['kind']),
      mime,
      url,
      path,
      sizeBytes: null,
      width: null,
      height: null,
      durationMs: null,
      pageCount: null,
    },
  ]
}

export function skillPresentations(part: TranscriptDisplayPart): SkillPresentation[] {
  const content = jsonRecord(part.source.agenaContent)
  const values = jsonArray(content.skills)
  const sourceValues = values.length ? values : [content]
  return sourceValues
    .map((value) => {
      const skill = jsonRecord(value)
      const name = firstString(skill, ['name', 'skill'])
      if (!name) return null
      return {
        name,
        description: firstString(skill, ['description']),
        instructions: firstString(skill, ['instructions']),
        source: firstString(skill, ['source']),
        contentHash: firstString(skill, ['content_hash']),
      }
    })
    .filter((item): item is SkillPresentation => Boolean(item))
}

function interactionQuestion(value: JsonValue, index: number): InteractionQuestionPresentation | null {
  const question = jsonRecord(value)
  const label = firstString(question, ['question'])
  if (!label) return null
  return {
    questionId: String(index),
    header: firstString(question, ['header']),
    question: label,
    multiple: booleanValue(question, ['multiple']),
    allowCustom: booleanValue(question, ['allow_custom']),
    options: jsonArray(question.options)
      .map((raw) => {
        const option = jsonRecord(raw)
        const label = firstString(option, ['label'])
        return label ? { label, description: firstString(option, ['description']) } : null
      })
      .filter((item): item is InteractionOptionPresentation => Boolean(item)),
  }
}

function userInputPresentationFromRequest(
  request: JsonRecord,
  reply: JsonValue | null,
  fallbackTitle: string,
): InteractionPresentation {
  return {
    requestId: firstString(request, ['request_id']),
    title: firstString(request, ['title']) || fallbackTitle,
    bodyMarkdown: firstString(request, ['body_markdown']),
    kind: firstString(request, ['input_kind', 'kind']),
    pending: reply === null || reply === undefined,
    reply,
    questions: jsonArray(request.questions)
      .map((value, index) => interactionQuestion(value, index))
      .filter((item): item is InteractionQuestionPresentation => Boolean(item)),
  }
}

function permissionActionLabel(action: JsonRecord): string {
  const actionKind = firstString(action, ['kind'])
  if (actionKind === 'path_access') {
    return [firstString(action, ['access_kind']), firstString(action, ['target_path'])].filter(Boolean).join(' ')
  }
  if (actionKind === 'network_access') {
    return `network ${firstString(action, ['target', 'host'])}`.trim()
  }
  return (
    [firstString(action, ['tool_name', 'name']), firstString(action, ['qualifier'])].filter(Boolean).join(' · ') ||
    actionKind ||
    'permission'
  )
}

function attentionProperties(attention: AttentionPresentationSource): JsonRecord {
  return jsonRecord(jsonRecord(attention.payload).properties)
}

function attentionRequest(attention: AttentionPresentationSource): JsonRecord {
  const properties = attentionProperties(attention)
  const nested = jsonRecord(properties.request)
  return Object.keys(nested).length ? { ...properties, ...nested } : properties
}

/** Convert the state-driven question attention into the same presentation used by durable parts. */
export function interactionPresentationFromAttention(
  attention: AttentionPresentationSource | null | undefined,
): InteractionPresentation | null {
  if (!attention || attention.kind !== 'question') return null
  const request = attentionRequest(attention)
  const requestId = attentionRequestId(attention.payload) || firstString(request, ['request_id', 'id'])
  if (!requestId) return null
  const presentation = userInputPresentationFromRequest(
    { ...request, request_id: requestId },
    null,
    firstString(request, ['title']) || 'Question',
  )
  return presentation.questions.length ? presentation : null
}

/** Convert the state-driven permission attention into the durable permission presentation. */
export function permissionPresentationFromAttention(
  attention: AttentionPresentationSource | null | undefined,
): OperationPermissionPresentation | null {
  if (!attention || attention.kind !== 'permission') return null
  const request = attentionRequest(attention)
  const requestId = attentionRequestId(attention.payload) || firstString(request, ['request_id', 'id'])
  if (!requestId) return null
  const action = jsonRecord(request.action)
  return {
    requestId,
    pending: true,
    status: 'Awaiting user approval',
    action: permissionActionLabel(action),
    reason: firstString(request, ['reason']),
    explanation: firstString(request, ['explanation']),
    replyReason: '',
    provenance: [firstString(request, ['source']), firstString(request, ['scope'])].filter(Boolean).join(' · '),
  }
}

/**
 * Return the temporary transcript projection for a pending request. Durable
 * operation parts own the same request once they arrive; the id set prevents
 * the state-driven fallback from rendering a second copy during that handoff.
 */
export function pendingInteractionPresentationFromAttention(
  attention: AttentionPresentationSource | null | undefined,
  durableRequestIds: ReadonlySet<string>,
): {
  requestId: string
  interaction: InteractionPresentation | null
  permission: OperationPermissionPresentation | null
} | null {
  if (!attention) return null
  const requestId = attentionRequestId(attention.payload)
  if (!requestId || durableRequestIds.has(requestId)) return null
  const interaction = interactionPresentationFromAttention(attention)
  const permission = permissionPresentationFromAttention(attention)
  return interaction || permission ? { requestId, interaction, permission } : null
}

export function errorPresentation(part: TranscriptDisplayPart): ErrorPresentation {
  const content = jsonRecord(part.source.agenaContent)
  const problem = jsonRecord(content.problem)
  const user = jsonRecord(problem.user)
  return {
    message:
      firstString(user, ['fallback']) ||
      firstString(problem, ['message']) ||
      firstString(content, ['message']) ||
      part.summary,
    code: firstString(problem, ['code']),
    category: firstString(problem, ['category']),
    responsibility: firstString(problem, ['responsibility']),
    impact: firstString(problem, ['impact']),
    recovery: firstString(problem, ['recovery']),
    retry: firstString(problem, ['retry']),
    correlationId: firstString(problem, ['correlation_id', 'id']),
    details: problem,
  }
}
