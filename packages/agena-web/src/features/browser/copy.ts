import { useI18n } from 'vue-i18n'

const copy = {
  browser: ['浏览器', 'Browser'],
  requested: ['AI 请求你接管浏览器', 'AI asked you to take over the browser'],
  open: ['打开浏览器', 'Open browser'],
  take: ['接管浏览器', 'Take control'],
  giveBack: ['交还给 AI', 'Return to AI'],
  discard: ['关闭这些页面', 'Close these pages'],
  cancelRequest: ['取消请求', 'Cancel request'],
  tabs: ['浏览器标签页', 'Browser tabs'],
  preview: ['浏览器画面', 'Browser viewport'],
  signInHere: ['需要登录时，请在浏览器画面中输入。', 'Enter sign-in details in the browser viewport.'],
  previewOnly: ['仅预览', 'Preview only'],
  youControl: ['你正在操作，AI 的浏览器访问已暂停', 'You have control. AI browser access is paused.'],
  otherView: ['浏览器正在另一个视图中被接管', 'Another view has control of the browser.'],
  empty: ['这个会话还没有打开浏览器页面。', 'This session has no open browser pages.'],
  linkedPageClosed: ['这条消息打开的页面已关闭。', 'The page opened by this message is closed.'],
  loading: ['正在连接浏览器…', 'Connecting to the browser…'],
  keyboard: ['键盘输入', 'Keyboard input'],
  keyboardHint: [
    '点击页面开始输入。按 Esc 退出键盘输入。',
    'Click the page to type. Press Esc to leave keyboard input.',
  ],
  dialog: ['网页对话框', 'Page dialog'],
  confirm: ['确定', 'OK'],
  cancel: ['取消', 'Cancel'],
  resume: ['继续任务', 'Continue task'],
  failed: ['浏览器连接出错。', 'The browser connection failed.'],
} as const

export function useBrowserCopy() {
  const { locale } = useI18n()
  return (key: keyof typeof copy) => copy[key][locale.value.startsWith('zh') ? 0 : 1]
}
