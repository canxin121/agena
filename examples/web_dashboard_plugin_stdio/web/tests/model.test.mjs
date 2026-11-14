import test from 'node:test'
import assert from 'node:assert/strict'
import { readBoard, projectBoard, translator } from '../dist/model.js'

const fixture = () => ({
  schema_version: 1, revision: 4, next_id: 4,
  tasks: [
    { id: 'task-1', title: 'Design review', description: 'Mobile layout', status: 'planned', priority: 'high', updated_at_ms: 1000 },
    { id: 'task-2', title: 'Ship docs', description: '', status: 'in_progress', priority: 'normal', updated_at_ms: 2000 },
    { id: 'task-3', title: 'Read feedback', description: 'Review notes', status: 'done', priority: 'low', updated_at_ms: 3000 },
  ],
})

test('filtering cards preserves whole-board progress and lane identity', () => {
  const view = projectBoard(readBoard(fixture()), ' REVIEW ', 'all')
  assert.deepEqual(view.visible.map((task) => task.id), ['task-1', 'task-3'])
  assert.equal(view.total, 3)
  assert.equal(view.counts.in_progress, 1)
  assert.equal(view.percentage, 33)
  assert.deepEqual(view.lanes.done.map((task) => task.id), ['task-3'])
  assert.equal(projectBoard(fixture(), 'review', 'in_progress').visible.length, 0)
})

test('empty boards retain a finite progress value', () => {
  const value = fixture()
  value.tasks = []
  const view = projectBoard(readBoard(value))
  assert.equal(view.percentage, 0)
  assert.equal(view.visible.length, 0)
})

test('duplicate identifiers, unsupported schemas and unsafe revisions are refused', () => {
  const duplicate = fixture()
  duplicate.tasks[1].id = duplicate.tasks[0].id
  assert.throws(() => readBoard(duplicate), /invalid task/)
  assert.throws(() => readBoard({ ...fixture(), schema_version: 2 }), /invalid board/)
  assert.throws(() => readBoard({ ...fixture(), revision: Number.MAX_SAFE_INTEGER + 1 }), /invalid board/)
  const status = fixture()
  status.tasks[0].status = 'unknown'
  assert.throws(() => readBoard(status), /invalid task/)
})

test('user text remains data and Chinese locale selection is explicit', () => {
  const board = fixture()
  board.tasks[0].title = '<img src=x onerror=alert(1)>'
  assert.equal(readBoard(board).tasks[0].title, '<img src=x onerror=alert(1)>')
  assert.equal(translator('zh-CN')('save'), '保存任务')
  assert.equal(translator('en-US')('save'), 'Save task')
})
