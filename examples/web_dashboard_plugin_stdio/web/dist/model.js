export const STATUSES = ['planned', 'in_progress', 'done']
export const PRIORITIES = ['low', 'normal', 'high']

const copy = {
  en: {
    title: 'Workspace board', eyebrow: 'YOUR WORKSPACE', subtitle: 'A little clarity. A little progress. A place for what comes next.',
    example: 'Example plugin', add: 'New task', refresh: 'Refresh board', search: 'Search tasks',
    searchPlaceholder: 'Find a task…', all: 'All tasks', planned: 'To do', in_progress: 'In progress', done: 'Done',
    total: 'On your board', active: 'In motion', completed: 'Completed', progress: 'Board progress',
    progressHint: 'One step at a time.', emptyTitle: 'Room for your next idea', emptyText: 'Add a task to get started.',
    noMatches: 'No matching tasks', noMatchesHint: 'Try a different search or filter.', clear: 'Clear filters',
    laneEmpty: 'Nothing here yet', low: 'Low priority', normal: 'Normal priority', high: 'High priority',
    edit: 'Edit task', create: 'Create a task', taskTitle: 'Title', description: 'Details', status: 'Status', priority: 'Priority',
    titlePlaceholder: 'What would you like to get done?', descriptionPlaceholder: 'Add context, a next step, or a useful note…',
    cancel: 'Cancel', save: 'Save task', saving: 'Saving…', remove: 'Delete task', confirmDelete: 'Delete this task',
    deleteQuestion: 'Delete this task? This removes it from the board.', keep: 'Keep task', close: 'Close editor',
    start: 'Start task', finish: 'Mark complete', reopen: 'Reopen task', synced: 'All changes saved', loading: 'Loading your board…',
    loadError: 'The board could not be loaded.', saveError: 'The change could not be saved.', retry: 'Try again', syncError: 'Sync needs attention',
    conflict: 'This board changed in another page. The latest board is loaded; review your edit and save again.',
    saved: 'Task saved', deleted: 'Task deleted', updated: 'Status updated', sampleHint: 'A persistent workspace, powered by your plugin.',
    taskCount: 'tasks', countLabel: 'Visible tasks', updatedAt: 'Updated', editorHint: 'Changes are saved when you choose Save task.',
  },
  zh: {
    title: '工作看板', eyebrow: '工作空间', subtitle: '理清想法，推进手头的事，让下一步更清晰。',
    example: '示例插件', add: '新建任务', refresh: '刷新看板', search: '搜索任务',
    searchPlaceholder: '查找任务…', all: '全部任务', planned: '待开始', in_progress: '进行中', done: '已完成',
    total: '看板任务', active: '正在推进', completed: '已经完成', progress: '完成进度',
    progressHint: '每一步，都算数。', emptyTitle: '留个位置，给下一个想法', emptyText: '新建一个任务，开始整理你的工作。',
    noMatches: '没有找到任务', noMatchesHint: '试试其他关键词，或清除筛选。', clear: '清除筛选',
    laneEmpty: '这里暂时没有任务', low: '低优先级', normal: '普通优先级', high: '高优先级',
    edit: '编辑任务', create: '新建任务', taskTitle: '标题', description: '详细说明', status: '状态', priority: '优先级',
    titlePlaceholder: '你希望完成什么？', descriptionPlaceholder: '补充背景、下一步，或一条有用的备注…',
    cancel: '取消', save: '保存任务', saving: '正在保存…', remove: '删除任务', confirmDelete: '确认删除',
    deleteQuestion: '删除这个任务？它将从看板中移除。', keep: '保留任务', close: '关闭编辑器',
    start: '开始任务', finish: '标记完成', reopen: '重新开始', synced: '所有更改已保存', loading: '正在加载看板…',
    loadError: '暂时无法加载看板。', saveError: '更改未能保存。', retry: '重试', syncError: '同步需要重试',
    conflict: '看板已在其他页面更新。已加载最新内容，请检查当前编辑后再次保存。',
    saved: '任务已保存', deleted: '任务已删除', updated: '状态已更新', sampleHint: '由插件驱动，持续保存你的工作。',
    taskCount: '个任务', countLabel: '当前显示的任务数', updatedAt: '更新于', editorHint: '点击“保存任务”后，修改才会保存。',
  },
}

export function translator(locale) {
  const dictionary = String(locale || '').toLowerCase().startsWith('zh') ? copy.zh : copy.en
  return (key) => dictionary[key] || copy.en[key] || key
}

export function readBoard(value) {
  if (!value || typeof value !== 'object' || value.schema_version !== 1
      || !Number.isSafeInteger(value.revision) || value.revision < 1
      || !Number.isSafeInteger(value.next_id) || value.next_id < 1
      || !Array.isArray(value.tasks) || value.tasks.length > 100) {
    throw new Error('The plugin returned an invalid board.')
  }
  const ids = new Set()
  for (const task of value.tasks) {
    if (!task || typeof task !== 'object' || typeof task.id !== 'string'
        || !/^task-[1-9][0-9]*$/.test(task.id) || ids.has(task.id)
        || Number(task.id.slice(5)) >= value.next_id
        || typeof task.title !== 'string' || !task.title.trim() || [...task.title].length > 160
        || typeof task.description !== 'string' || [...task.description].length > 2000
        || !STATUSES.includes(task.status) || !PRIORITIES.includes(task.priority)
        || !Number.isSafeInteger(task.updated_at_ms) || task.updated_at_ms < 0) {
      throw new Error('The plugin returned an invalid task.')
    }
    ids.add(task.id)
  }
  return value
}

export function projectBoard(board, query = '', status = 'all') {
  const search = query.trim().toLocaleLowerCase()
  const visible = board.tasks.filter((task) =>
    (status === 'all' || task.status === status)
    && (!search || (task.title + ' ' + task.description).toLocaleLowerCase().includes(search)))
  const counts = Object.fromEntries(STATUSES.map((key) => [key, board.tasks.filter((task) => task.status === key).length]))
  return {
    visible,
    counts,
    total: board.tasks.length,
    percentage: board.tasks.length ? Math.round(counts.done / board.tasks.length * 100) : 0,
    lanes: Object.fromEntries(STATUSES.map((key) => [key, visible.filter((task) => task.status === key)])),
  }
}
