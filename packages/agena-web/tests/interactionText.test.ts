import assert from 'node:assert/strict'
import test from 'node:test'
import { createI18n } from 'vue-i18n'
import enUS from '../src/i18n/messages/en-US'
import zhCN from '../src/i18n/messages/zh-CN'
import { interactionText } from '../src/pages/chat/interactionText'

const translator = createI18n({ legacy: false, locale: 'zh-CN', messages: { 'en-US': enUS, 'zh-CN': zhCN } }).global

test('built-in review choices and descriptions follow the interface locale', () => {
  assert.equal(interactionText('review', 'Approve with autorun on', translator.t), '批准并开启自动执行')
  assert.equal(interactionText('review', 'Keep in planning', translator.t), '继续规划')
  assert.equal(interactionText('review', 'Approve New Plan', translator.t), '审批新计划')
  assert.equal(
    interactionText('review', 'Approve the plan and mark it completed.', translator.t),
    '批准计划，并将其标记为已完成。',
  )
  translator.locale.value = 'en-US'
  assert.equal(interactionText('review', 'Approve with autorun on', translator.t), 'Approve with autorun on')
  translator.locale.value = 'zh-CN'
})

test('plan transition questions translate every supported target phase', () => {
  for (const [phase, localized] of Object.entries(zhCN.chat.planReview.phases)) {
    const question = `Choose whether this plan should move to ${phase}, or type feedback for the agent to revise the plan.`
    assert.equal(
      interactionText('review', question, translator.t),
      `请选择是否将此计划设为“${localized}”，或输入反馈，让助手修改计划。`,
    )
  }
})

test('Ask content, custom review text and unknown protocol strings remain verbatim', () => {
  for (const value of ['Approve', 'Keep in planning', 'Custom', 'Type your own answer']) {
    assert.equal(interactionText('ask_user', value, translator.t), value)
  }
  for (const value of ['Please revise step two.', '批准', 'toString', '__proto__', 'New choice from a plugin']) {
    assert.equal(interactionText('review', value, translator.t), value)
  }
})
