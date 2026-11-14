import { STATUSES, PRIORITIES, translator, readBoard, projectBoard } from './model.js'
import { icon } from './icons.js'

/** Mount one independent page instance. All DOM and listeners belong to this handle. */
export function mount(container, sdk) {
  const document = container.ownerDocument
  const lifetime = new AbortController()
  const instanceId = 'agwd-' + Math.random().toString(36).slice(2)
  let environment = sdk.environment
  let t = translator(environment.locale)
  let board = null
  let query = ''
  let filter = 'all'
  let loading = false
  let saving = false
  let syncFailed = false
  let disposed = false
  let editingId = null
  let pendingDelete = false
  let unsubscribe = () => {}

  const el = (tag, className = '', text = '') => {
    const node = document.createElement(tag)
    if (className) node.className = className
    if (text) node.textContent = text
    return node
  }
  const copy = (tag, className, key) => {
    const node = el(tag, className, t(key))
    node.dataset.copy = key
    return node
  }
  const button = (action, key, iconName, className = '', iconOnly = false) => {
    const node = el('button', 'agwd-button ' + className)
    node.type = 'button'
    node.dataset.action = action
    node.dataset.label = key
    node.setAttribute('aria-label', t(key))
    if (iconName) node.append(icon(document, iconName))
    node.append(copy('span', iconOnly ? 'agwd-sr-only' : '', key))
    return node
  }
  const root = el('section', 'agwd')
  root.dataset.pluginDashboard = ''
  root.tabIndex = -1

  const hero = el('header', 'agwd-hero')
  const heading = el('div', 'agwd-heading')
  const eyebrow = el('div', 'agwd-eyebrow')
  eyebrow.append(copy('span', '', 'eyebrow'), copy('span', 'agwd-example', 'example'))
  const title = copy('h1', 'agwd-title', 'title')
  const subtitle = copy('p', 'agwd-subtitle', 'subtitle')
  heading.append(eyebrow, title, subtitle)
  const newButton = button('new', 'add', 'plus', 'agwd-primary')
  newButton.dataset.mutation = ''
  hero.append(heading, newButton)

  const stats = el('div', 'agwd-stats')
  const values = {}
  for (const [key, number, accent] of [['total', 'total', 'neutral'], ['active', 'active', 'amber'], ['completed', 'completed', 'green']]) {
    const card = el('div', 'agwd-stat agwd-stat-' + accent)
    const line = el('div', 'agwd-stat-heading')
    line.append(copy('span', '', key), el('span', 'agwd-stat-dot'))
    values[number] = el('strong', 'agwd-stat-value', '—')
    values[number].dataset.stat = number
    card.append(line, values[number])
    if (key === 'completed') {
      const progressTrack = el('div', 'agwd-progress')
      progressTrack.setAttribute('role', 'progressbar')
      progressTrack.setAttribute('aria-valuemin', '0')
      progressTrack.setAttribute('aria-valuemax', '100')
      progressTrack.dataset.progress = ''
      progressTrack.append(el('span', 'agwd-progress-fill'))
      card.append(progressTrack)
    } else card.append(copy('span', 'agwd-stat-caption', key === 'total' ? 'taskCount' : 'progressHint'))
    stats.append(card)
  }

  const toolbar = el('div', 'agwd-toolbar')
  const filters = el('div', 'agwd-filters')
  filters.setAttribute('role', 'group')
  for (const key of ['all', ...STATUSES]) {
    const node = button('filter', key, null, 'agwd-filter')
    node.dataset.filter = key
    filters.append(node)
  }
  const tools = el('div', 'agwd-tools')
  const searchWrap = el('label', 'agwd-search')
  searchWrap.append(icon(document, 'search'), copy('span', 'agwd-sr-only', 'search'))
  const search = el('input')
  search.type = 'search'
  search.name = 'search'
  search.maxLength = 300
  search.dataset.search = ''
  searchWrap.append(search)
  const refreshButton = button('refresh', 'refresh', 'refresh', 'agwd-icon-button', true)
  tools.append(searchWrap, refreshButton)
  toolbar.append(filters, tools)

  const errorBanner = el('div', 'agwd-error')
  errorBanner.hidden = true
  errorBanner.setAttribute('role', 'alert')
  const errorText = el('p')
  errorBanner.append(errorText, button('refresh', 'retry', 'refresh', 'agwd-small'))

  const boardGrid = el('div', 'agwd-board')
  boardGrid.dataset.board = ''

  const footer = el('footer', 'agwd-footer')
  const syncState = el('span', 'agwd-sync')
  syncState.setAttribute('role', 'status')
  syncState.setAttribute('aria-live', 'polite')
  const footerHint = copy('span', 'agwd-footer-hint', 'sampleHint')
  footer.append(syncState, footerHint)

  const dialog = el('dialog', 'agwd-dialog')
  dialog.setAttribute('aria-labelledby', instanceId + '-editor-title')
  const form = el('form', 'agwd-form')
  form.dataset.taskForm = ''
  const formHeader = el('header', 'agwd-form-header')
  const formHeading = el('div')
  const editorTitle = el('h2')
  editorTitle.id = instanceId + '-editor-title'
  formHeading.append(editorTitle, copy('p', '', 'editorHint'))
  formHeader.append(formHeading, button('close', 'close', 'close', 'agwd-icon-button', true))
  const formBody = el('div', 'agwd-form-body')
  const fields = {}
  function addField(name, key, tag, options) {
    const wrapper = el('label', 'agwd-field')
    const label = copy('span', 'agwd-field-label', key)
    const control = el(tag, 'agwd-control')
    control.name = name
    control.id = instanceId + '-' + name
    wrapper.htmlFor = control.id
    if (tag === 'select') {
      for (const value of options) {
        const option = copy('option', '', value)
        option.value = value
        control.append(option)
      }
    }
    wrapper.append(label, control)
    fields[name] = control
    return wrapper
  }
  formBody.append(addField('title', 'taskTitle', 'input'))
  fields.title.type = 'text'
  fields.title.required = true
  fields.title.maxLength = 160
  formBody.append(addField('description', 'description', 'textarea'))
  fields.description.rows = 4
  fields.description.maxLength = 2000
  const fieldRow = el('div', 'agwd-field-row')
  fieldRow.append(addField('status', 'status', 'select', STATUSES), addField('priority', 'priority', 'select', PRIORITIES))
  formBody.append(fieldRow)
  const editorError = el('p', 'agwd-editor-error')
  editorError.hidden = true
  editorError.setAttribute('role', 'alert')
  formBody.append(editorError)
  const deleteConfirm = el('div', 'agwd-delete-confirm')
  deleteConfirm.hidden = true
  deleteConfirm.append(copy('p', '', 'deleteQuestion'), button('delete-confirm', 'confirmDelete', 'trash', 'agwd-danger'), button('delete-cancel', 'keep', null, 'agwd-small'))
  formBody.append(deleteConfirm)
  const formFooter = el('footer', 'agwd-form-footer')
  const deleteButton = button('delete-request', 'remove', 'trash', 'agwd-text-danger')
  const formActions = el('div', 'agwd-form-actions')
  const cancelButton = button('close', 'cancel', null, '')
  const saveButton = button('save', 'save', 'check', 'agwd-primary')
  saveButton.type = 'submit'
  formActions.append(cancelButton, saveButton)
  formFooter.append(deleteButton, formActions)
  form.append(formHeader, formBody, formFooter)
  dialog.append(form)

  root.append(hero, stats, toolbar, errorBanner, boardGrid, footer, dialog)
  container.append(root)

  function showError(message, inEditor = false) {
    syncFailed = true
    const target = inEditor ? editorError : errorText
    target.textContent = message
    if (inEditor) editorError.hidden = false
    else errorBanner.hidden = false
  }
  function clearErrors() {
    syncFailed = false
    errorBanner.hidden = true
    editorError.hidden = true
  }
  function applyLabels() {
    root.lang = environment.locale
    root.dataset.theme = environment.theme
    for (const node of root.querySelectorAll('[data-copy]')) node.textContent = t(node.dataset.copy)
    for (const node of root.querySelectorAll('[data-label]')) node.setAttribute('aria-label', t(node.dataset.label))
    search.placeholder = t('searchPlaceholder')
    fields.title.placeholder = t('titlePlaceholder')
    fields.description.placeholder = t('descriptionPlaceholder')
    editorTitle.textContent = t(editingId ? 'edit' : 'create')
    stats.querySelector('[data-progress]').setAttribute('aria-label', t('progress'))
    filters.setAttribute('aria-label', t('status'))
    sdk.ui.setTitle(t('title'))
  }
  function renderBusy() {
    root.setAttribute('aria-busy', String(loading))
    for (const node of root.querySelectorAll('[data-mutation]')) node.disabled = !board || loading || saving
    refreshButton.disabled = loading || saving
    refreshButton.classList.toggle('agwd-is-loading', loading)
    for (const node of form.querySelectorAll('input,textarea,select,button')) node.disabled = saving
    saveButton.querySelector('[data-copy]').textContent = t(saving ? 'saving' : 'save')
    syncState.textContent = t(saving ? 'saving' : loading ? 'loading' : syncFailed ? 'syncError' : board ? 'synced' : 'loading')
    syncState.dataset.busy = String(loading || saving)
    syncState.dataset.error = String(syncFailed && !loading && !saving)
  }
  function renderBoard() {
    boardGrid.replaceChildren()
    for (const node of filters.querySelectorAll('[data-filter]')) {
      node.setAttribute('aria-pressed', String(node.dataset.filter === filter))
    }
    if (!board) {
      if (syncFailed && !loading) {
        const empty = el('div', 'agwd-empty')
        empty.append(el('h2', '', t('loadError')))
        boardGrid.append(empty)
        return
      }
      for (let i = 0; i < 3; i++) {
        const skeleton = el('div', 'agwd-skeleton-lane')
        skeleton.setAttribute('aria-hidden', 'true')
        skeleton.append(el('div', 'agwd-skeleton-heading'), el('div', 'agwd-skeleton-card'), el('div', 'agwd-skeleton-card'))
        boardGrid.append(skeleton)
      }
      return
    }
    const projection = projectBoard(board, query, filter)
    values.total.textContent = String(projection.total)
    values.active.textContent = String(projection.counts.in_progress)
    values.completed.textContent = String(projection.counts.done)
    const progressTrack = stats.querySelector('[data-progress]')
    progressTrack.setAttribute('aria-valuenow', String(projection.percentage))
    progressTrack.querySelector('span').style.width = projection.percentage + '%'
    boardGrid.dataset.single = String(filter !== 'all')
    if (!projection.visible.length && (query.trim() || filter !== 'all' || !board.tasks.length)) {
      const empty = el('div', 'agwd-empty')
      const emblem = el('div', 'agwd-empty-icon')
      emblem.append(icon(document, query.trim() || filter !== 'all' ? 'search' : 'board'))
      const isFiltered = Boolean(query.trim() || filter !== 'all')
      empty.append(emblem, el('h2', '', t(isFiltered ? 'noMatches' : 'emptyTitle')), el('p', '', t(isFiltered ? 'noMatchesHint' : 'emptyText')))
      const action = button(isFiltered ? 'clear' : 'new', isFiltered ? 'clear' : 'add', isFiltered ? null : 'plus', 'agwd-primary')
      if (!isFiltered) action.dataset.mutation = ''
      empty.append(action)
      boardGrid.append(empty)
      renderBusy()
      return
    }
    for (const status of STATUSES) {
      if (filter !== 'all' && status !== filter) continue
      const lane = el('section', 'agwd-lane')
      lane.dataset.status = status
      const laneHeader = el('header', 'agwd-lane-header')
      const laneTitle = el('h2')
      laneTitle.append(el('span', 'agwd-lane-dot'), el('span', '', t(status)), el('span', 'agwd-lane-count', String(projection.lanes[status].length)))
      const laneAdd = button('new', 'add', 'plus', 'agwd-icon-button agwd-lane-add', true)
      laneAdd.dataset.status = status
      laneAdd.dataset.mutation = ''
      laneHeader.append(laneTitle, laneAdd)
      const laneBody = el('div', 'agwd-lane-body')
      for (const task of projection.lanes[status]) {
        const card = el('article', 'agwd-task')
        card.dataset.taskItem = task.id
        const cardTop = el('div', 'agwd-task-top')
        const priority = el('span', 'agwd-priority', t(task.priority))
        priority.dataset.priority = task.priority
        const editButton = button('edit', 'edit', 'dots', 'agwd-icon-button agwd-edit', true)
        editButton.dataset.taskId = task.id
        editButton.dataset.mutation = ''
        editButton.setAttribute('aria-label', t('edit') + ': ' + task.title)
        cardTop.append(priority, editButton)
        const taskHeading = el('h3')
        const taskLink = el('button', 'agwd-task-title', task.title)
        taskLink.type = 'button'
        taskLink.dataset.action = 'edit'
        taskLink.dataset.taskId = task.id
        taskLink.dataset.mutation = ''
        taskHeading.append(taskLink)
        card.append(cardTop, taskHeading)
        if (task.description) card.append(el('p', 'agwd-task-description', task.description))
        const cardBottom = el('footer', 'agwd-task-footer')
        const id = el('span', 'agwd-task-id', 'WB-' + task.id.slice(5).padStart(3, '0'))
        id.title = t('updatedAt') + ' ' + new Intl.DateTimeFormat(environment.locale, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(task.updated_at_ms))
        const nextKey = task.status === 'planned' ? 'start' : task.status === 'done' ? 'reopen' : 'finish'
        const advance = button('advance', nextKey, task.status === 'in_progress' ? 'check' : 'arrow', 'agwd-advance')
        advance.dataset.taskId = task.id
        advance.dataset.mutation = ''
        cardBottom.append(id, advance)
        card.append(cardBottom)
        laneBody.append(card)
      }
      if (!projection.lanes[status].length) laneBody.append(el('div', 'agwd-lane-empty', t('laneEmpty')))
      lane.append(laneHeader, laneBody)
      boardGrid.append(lane)
    }
    renderBusy()
  }
  function resetDelete() {
    pendingDelete = false
    deleteConfirm.hidden = true
    deleteButton.hidden = !editingId
  }
  function closeEditor() {
    if (saving) return
    dialog.close()
    resetDelete()
    const previous = editingId && root.querySelector('[data-task-item="' + editingId + '"] .agwd-task-title')
    ;(previous || newButton).focus()
  }
  function openEditor(taskId = null, initialStatus = 'planned') {
    if (!board || loading || saving) return
    const task = taskId ? board.tasks.find((item) => item.id === taskId) : null
    if (taskId && !task) return
    editingId = taskId
    fields.title.value = task?.title || ''
    fields.description.value = task?.description || ''
    fields.status.value = task?.status || initialStatus
    fields.priority.value = task?.priority || 'normal'
    editorTitle.textContent = t(taskId ? 'edit' : 'create')
    editorError.hidden = true
    resetDelete()
    dialog.showModal()
    fields.title.focus()
  }

  async function refresh() {
    if (disposed || loading || saving) return
    loading = true
    clearErrors()
    if (!board) renderBoard()
    renderBusy()
    try {
      const result = await sdk.commands.invoke('list', {})
      if (disposed) return
      if (result.status !== 'succeeded') throw new Error(result.detail || result.summary || t('loadError'))
      board = readBoard(result.output)
      renderBoard()
    } catch (error) {
      if (!disposed && !sdk.signal.aborted) showError(error instanceof Error ? error.message : t('loadError'))
    } finally {
      if (!disposed) {
        loading = false
        if (!board) renderBoard()
        renderBusy()
      }
    }
  }
  async function update(operation, inEditor = false) {
    if (disposed || !board || saving || loading) return false
    saving = true
    clearErrors()
    renderBusy()
    try {
      const result = await sdk.commands.invoke('update', { revision: board.revision, operation })
      if (disposed) return false
      if (result.status !== 'succeeded') {
        if (result.output?.conflict && result.output.board) {
          board = readBoard(result.output.board)
          renderBoard()
          throw new Error(t('conflict'))
        }
        throw new Error(result.detail || result.summary || t('saveError'))
      }
      board = readBoard(result.output)
      renderBoard()
      sdk.ui.notify(t(operation.type === 'delete' ? 'deleted' : inEditor ? 'saved' : 'updated'), 'success')
      return true
    } catch (error) {
      if (!disposed && !sdk.signal.aborted) showError(error instanceof Error ? error.message : t('saveError'), inEditor)
      return false
    } finally {
      if (!disposed) {
        saving = false
        renderBusy()
      }
    }
  }
  async function saveEditor(event) {
    event.preventDefault()
    if (saving || !form.reportValidity()) return
    const saved = await update({
      type: 'save',
      id: editingId,
      title: fields.title.value,
      description: fields.description.value,
      status: fields.status.value,
      priority: fields.priority.value,
    }, true)
    if (saved && !disposed) closeEditor()
  }
  async function onClick(event) {
    const target = event.target instanceof Element ? event.target.closest('[data-action]') : null
    if (!target || !root.contains(target) || target.disabled) return
    const action = target.dataset.action
    if (action === 'new') openEditor(null, target.dataset.status || 'planned')
    else if (action === 'edit') openEditor(target.dataset.taskId)
    else if (action === 'refresh') await refresh()
    else if (action === 'close') closeEditor()
    else if (action === 'filter' || action === 'clear') {
      filter = action === 'clear' ? 'all' : target.dataset.filter
      if (action === 'clear') { query = ''; search.value = '' }
      renderBoard()
    } else if (action === 'delete-request') {
      pendingDelete = true
      deleteConfirm.hidden = false
      deleteButton.hidden = true
      deleteConfirm.querySelector('button').focus()
    } else if (action === 'delete-cancel') {
      resetDelete()
      deleteButton.focus()
    } else if (action === 'delete-confirm' && pendingDelete && editingId) {
      if (await update({ type: 'delete', id: editingId }, true) && !disposed) closeEditor()
    } else if (action === 'advance') {
      const task = board?.tasks.find((item) => item.id === target.dataset.taskId)
      if (!task) return
      const saved = await update({
        type: 'save', id: task.id, title: task.title, description: task.description,
        priority: task.priority, status: task.status === 'in_progress' ? 'done' : 'in_progress',
      })
      if (saved && !disposed) {
        const replacement = root.querySelector('[data-task-item="' + task.id + '"] [data-action="advance"]')
        ;(replacement || search).focus()
      }
    }
  }
  function dispose() {
    if (disposed) return
    disposed = true
    lifetime.abort()
    unsubscribe()
    sdk.signal.removeEventListener('abort', dispose)
    if (dialog.open) dialog.close()
    root.remove()
  }

  root.addEventListener('click', (event) => { void onClick(event) }, { signal: lifetime.signal })
  search.addEventListener('input', () => { query = search.value; renderBoard() }, { signal: lifetime.signal })
  form.addEventListener('submit', (event) => { void saveEditor(event) }, { signal: lifetime.signal })
  dialog.addEventListener('cancel', (event) => { event.preventDefault(); closeEditor() }, { signal: lifetime.signal })
  sdk.signal.addEventListener('abort', dispose, { once: true })
  unsubscribe = sdk.subscribeEnvironment((next) => {
    if (disposed) return
    environment = next
    t = translator(environment.locale)
    applyLabels()
    renderBoard()
    renderBusy()
  })
  if (sdk.signal.aborted) dispose()
  else void refresh()
  return { dispose }
}
