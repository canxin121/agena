# Agena 网页版纯键盘操作修复与验收

本次工作已补齐首轮审查列出的 14 类应用侧缺口，并修复帮助弹窗不能滚动的问题。普通模式具有完整的消息导航入口，布局、文件移动和图片平移具有等价键盘操作；菜单、树、列表、标签、分隔条和弹窗采用一致的焦点约定。

修改位于独立分支 `fix/web-keyboard-accessibility`，worktree 为 `/Volumes/Rc20/Projects/agena-wt/web-keyboard-accessibility`，初始代码基线为 `efaac47c9e0075e0b1a636fdc281bc13cbb7c4f7`。修复已 rebase 至最新主分支 `7506556c3084bdaac66150b05350ae7a1eaac2d2`，保留期间主分支增加的 5 个提交，并重新完成下述集成验证。本报告的“已验证”指明下述组件与浏览器范围；涉及真实后端写入、外部页面或操作系统快捷键时，见文末边界说明。

## 帮助弹窗滚动

[`Dialog.vue`](../packages/agena-web/src/components/ui/Dialog.vue) 原先把 `bodyScroll` 声明为可选 Boolean。Vue 将没有传入的 Boolean 转换为 `false`，正文因此落入 `overflow-hidden`，长内容被裁切。现在显式设置 `bodyScroll: true`，普通弹窗正文默认滚动，并提供可聚焦、有名称的正文区域和可见焦点。标题、关闭按钮保持固定；编辑器类弹窗仍能显式使用 `bodyScroll=false` 由子组件管理滚动。

首轮复现时，1280 × 850 视口中的帮助正文高 1615 像素，可视部分为 729 像素，修复前 End 后 `scrollTop` 仍为 0；修复后能滚至 886。最终浏览器回归重新检查了滚轮、Home、End、Tab 圈定、Escape 返回来源，以及 390 × 640 和 844 × 390 两种小视口。快捷键说明可换行。

## 原 14 类缺漏的处理结果

| 原缺漏 | 已实现的行为 | 验证与实现位置 |
| --- | --- | --- |
| 工作区布局依赖拖放 | 每个标签有原生动作按钮，Shift+F10 / ContextMenu 也能打开；可复制到右侧分组、移到新分组或其他组、调整标签与分组顺序、合并分组、用当前内容替换其他标签。移走最后一个标签导致来源组件卸载时，焦点转到目标标签 | 浏览器完成排序、分屏、移动后卸载来源、拆分/合并和替换；[`WorkspaceEditorGroupPane.vue`](../packages/agena-web/src/layout/WorkspaceEditorGroupPane.vue)、[`ui.ts`](../packages/agena-web/src/stores/ui.ts) |
| 文件移动缺少单项键盘入口 | 文件/目录动作菜单增加“移动到…”，复用已有批量移动流程；目标路径可键入，Enter 只在路径输入框提交，Cancel 上的 Enter 正常取消。失败保持弹窗并聚焦路径；部分成功只保留失败目标，刷新失败也不会重试已移动的路径 | 浏览器覆盖单项菜单、错误反馈、取消和返回原文件行；实际文件重命名/移动 API 未在夹具中执行；[`FilesExplorerPane.vue`](../packages/agena-web/src/pages/files/components/FilesExplorerPane.vue)、[`FilesMoveDialog.vue`](../packages/agena-web/src/pages/files/components/FilesMoveDialog.vue)、[`FilesPage.vue`](../packages/agena-web/src/pages/FilesPage.vue) |
| 桌面附件面板焦点不完整 | 打开进入面板，提供命名 dialog 和浮层保护；焦点移到其他控件时关闭并保留新焦点；嵌套项目选择、图片预览与附件 Escape 分层关闭，恢复各自来源 | 浏览器覆盖进入、嵌套表单、图片与附件连续关闭、移焦离开；[`AttachmentsPanel.vue`](../packages/agena-web/src/components/chat/AttachmentsPanel.vue) |
| 放大图片无法键盘平移 | Shift+方向键平移；另有四个命名方向按钮。普通左右键继续换图，缩放与重置保留。边界由布局尺寸乘缩放值计算，避免缩放动画中读取过渡尺寸使紧接着的平移无效 | 浏览器检查四角均可到达、继续按键不越界、原生按钮和平移后的焦点返回；[`ImageViewerDialog.vue`](../packages/agena-web/src/components/ImageViewerDialog.vue) |
| 关闭 Vim 后缺消息级操作 | 普通模式工具栏提供上一条/下一条、复制、选择正文、搜索、计划和返回输入框。Alt+Shift+上下键及 Home/End 在用户和助手消息间移动；聚焦消息后可直接使用上下/Home/End、Ctrl/Cmd+C 和 Escape。到首条可加载更早记录，异步完成后避免抢走已移动的焦点 | 浏览器使用真实 MessageItem，覆盖助手消息、旧记录加载、复制回调、DOM 文本选择、搜索入口及输入框返回；[`TranscriptKeyboardToolbar.vue`](../packages/agena-web/src/components/chat/TranscriptKeyboardToolbar.vue)、[`ChatPageView.vue`](../packages/agena-web/src/pages/chat/ChatPageView.vue) |
| iframe 内缺应用退出路径 | 预览提供显式“进入预览操作”。可访问的同源主文档监听 F6 / Shift+F6 并返回入口，切换响应式/填充模式会清理并重建监听。跨源主文档移出顺序 Tab，并提供键盘打开独立标签页的等价入口 | 浏览器覆盖同源进入/退出、模式切换、跨源独立标签页；外部网页边界见下文；[`previewKeyboard.ts`](../packages/agena-web/src/features/workspacePreview/model/previewKeyboard.ts) |
| 文件树缺层级导航 | 单一主动作 Tab 停靠点；上下、首尾、翻页、名称定位、左右展开/折叠和父子移动，F2 重命名，Shift+F10 动作菜单。多选时 Shift+上下沿用现有范围选择；目录展开使用独立动作，不再被多选点击逻辑截走 | 浏览器覆盖父子移动、名称定位、重命名成功/失败/取消、多选与展开；[`keyboardCollection.ts`](../packages/agena-web/src/lib/keyboardCollection.ts) |
| 会话和目录长列表效率不足 | 目录、会话、子线程以及收藏/置顶/最近/运行列表使用共用导航；支持首尾、名称定位、层级展开和动作菜单。仅当前行保留次级动作的 Tab 入口；删除或折叠后保留可用焦点 | 浏览器覆盖目录、父子会话、动作入口与折叠；四类页脚通过共用指令接入；实现位于 `layout/chatSidebar/components/` |
| 设置树缺层级导航 | 统一的左右展开/父子移动、上下/首尾/名称定位，树项目暴露层级、展开和选中状态 | 浏览器覆盖展开、进入子项、激活、返回父项和按名称定位；[`SettingsSidebarNavList.vue`](../packages/agena-web/src/components/settings/sidebar/SettingsSidebarNavList.vue) |
| Git 长列表缺统一移动和多选 | 工作树、暂存、未跟踪与历史使用共用列表导航；Shift+上下复用已有范围选择处理，Enter/Space 继续执行原生主动作，当前行次级按钮保留 Tab 入口；删除和还原仍经过确认 | 浏览器覆盖工作树范围选择、名称/首尾定位、历史激活；真实 Git 写操作未执行；实现位于 `components/git/` |
| 组合控件语义不一致 | Picker 触发器的 haspopup 与实际 dialog 一致；命令搜索、提示词历史都有 combobox/listbox/option 及当前结果关联；真正的 Composer 关联命令结果。工作区使用 tablist/tab/tabpanel 与 aria-selected；图标按钮从已有 tooltip/title 补充名称 | 浏览器检查真实 Composer 的 active descendant、菜单结果与标签焦点；读屏软件的播报仍需实体辅助技术验证 |
| 预览尺寸手柄没有键盘动作 | 宽/高手柄为命名 separator，暴露当前值与上下界；方向键每次 1 像素，Shift 每次 10 像素，Home/End 到上下界。角手柄支持两轴并通知当前尺寸；已有宽高输入框保留 | 纯函数测试检查轴归属、边界和 IME/修饰键；浏览器操作全部三个手柄；[`WorkspacePreviewDockPanel.vue`](../packages/agena-web/src/features/workspacePreview/components/WorkspacePreviewDockPanel.vue) |
| 页面切换和隐藏输入框失焦 | 延迟焦点等待可见目标和异步窗格加载，用户继续 Tab/点击时取消请求；同页 query 更新保留输入焦点。输入框快捷键优先当前窗格，再激活已有聊天；没有会话时进入会话选择器并保留工作区 | 浏览器覆盖延迟出现、用户取消、路由目的地、同页 query、隐藏聊天激活和空会话回退；[`deferredKeyboardFocus.ts`](../packages/agena-web/src/lib/deferredKeyboardFocus.ts)、[`installKeyboardShortcuts.ts`](../packages/agena-web/src/app/runtime/installKeyboardShortcuts.ts) |
| 浏览器保留键冲突且不能改绑 | 保留既有应用入口，提供 Alt+Shift 替代；帮助中可录制、停用、恢复单项或全部应用快捷键。设置立即生效并按浏览器本地保存；拒绝重复绑定、普通打字、Tab/Escape/F6 和 AltGr 类组合，提示已知浏览器冲突；存储失败不覆盖旧设置 | 单测覆盖解析、冲突、坏数据、存储异常；浏览器覆盖冲突反馈、取消录制、即时生效、新页面加载和全部重置；[`KeyboardShortcutEditor.vue`](../packages/agena-web/src/components/KeyboardShortcutEditor.vue)、[`keyboardPreferences.ts`](../packages/agena-web/src/app/runtime/keyboardPreferences.ts) |

首轮审查中关于文件移动的描述过宽：批量移动流程原本已经存在，此次补的是单行菜单入口、焦点和失败处理。聊天搜索和计划也原本有原生按钮，此次补的是普通消息导航工具栏和统一键盘路径。

## 共用行为及补测发现

全局应用快捷键集中处理，仅消费映射到的数字 1–5，移除了 AppHeader 的重复监听。已经处理的事件、输入法候选确认和按住产生的 repeat 不再触发新的应用动作。终端保留 Ctrl 序列，应用提供 Alt+Shift 入口。折叠侧栏、隐藏窗格和窄布局中被覆盖的主内容使用 inert。

F6 / Shift+F6 在可见导航、侧栏、标签栏和工作区内容之间循环；focusin 同步当前窗格，区域入口遵守列表当前行的 Tab 停靠点。分屏及侧栏分隔条具有可见焦点、separator 角色、数值与方向键操作。

通用选项菜单支持上下/Home/End、跳过禁用项、循环、搜索后 Enter 和自动滚入当前项。InlineSearchAdd 支持非首项选择、连续添加返回输入框、Escape 与离焦关闭。提示词历史和命令建议离开后会关闭；listbox 显式使用 tabindex=-1，防止 Chrome 自动给可滚动容器添加额外 Tab 停靠点。

弹窗关闭优先回到存活的来源。来源行被删除时，回到相邻行、空列表或存活区域；从动作菜单打开表单时，会记住菜单原始来源，避免表单关闭后试图返回已卸载的菜单项。Radix 临时给背景加上的 aria-hidden 不会被误判为来源消失。另修复了确认弹层首次聚焦时动态挂载锚点导致按钮重建失焦的问题，并使用 Radix 实际支持的 element 属性传入虚拟锚点。

Git 等共用区块标题原本把操作按钮放在展开按钮内部。现在展开按钮与次级操作为同级原生控件，展开状态可读，Enter/Space 可以分别操作。帮助及新键盘提示同步到 en-US、zh-CN、es-ES、fr-FR、pt-BR、hi-IN、ar-SA 七种语言。

## 操作约定

| 目标 | 键盘路径 |
| --- | --- |
| 控件间移动和激活 | Tab / Shift+Tab；原生按钮使用 Enter / Space |
| 主要区域切换 | F6 / Shift+F6 |
| 文件、会话、设置、Git 列表 | 上下、Home/End、PageUp/PageDown；输入名称定位；树用左右键 |
| 当前行次级动作 | Tab；有动作菜单的行可用 Shift+F10 / ContextMenu；文件 F2 重命名 |
| 工作区标签 | 左右、Home/End；Delete 关闭；Shift+F10 或动作按钮组织布局 |
| 帮助正文滚动 | Tab 进入正文，再使用方向键、PageUp/PageDown、Home/End |
| 普通消息导航 | 工具栏，或 Alt+Shift+上下/Home/End；消息聚焦后上下/Home/End、Ctrl/Cmd+C、Escape |
| 图片 | 加减缩放、0 重置、普通左右换图、Shift+方向键平移，或方向按钮 |
| 预览 | 入口按钮进入同源主文档，F6 返回；跨源内容使用独立标签页入口 |
| 布局尺寸 | 聚焦 separator 后使用方向键、Shift 大步长、Home/End |
| 退出当前交互 | Escape 由最上层拥有；关闭返回来源，Tab 离开非模态建议面板时正常前进 |

默认 Alt+Shift 入口为 H 帮助、S 设置、L 会话侧栏、I 输入框、N 新会话、T 主题、1–5 主页面。所有入口及当前自定义结果均可在帮助中查看。快捷键编辑只改变应用级绑定，消息、树、Vim、编辑器和终端内部按键遵守各自控件的约定。

## 自动化与复现

在 `packages/agena-web` 运行：

```sh
bun run dev --host 127.0.0.1 --port 5175 --strictPort
```

保持源码稳定，在另一终端运行两组浏览器回归；完成后执行测试和构建，避免 Vite 热更新或测试依赖缓存重建干扰浏览器中的焦点状态：

```sh
uv run --with playwright python scripts/verify-keyboard-navigation.py --base http://127.0.0.1:5175
uv run --with playwright python scripts/verify-keyboard-workflows.py --base http://127.0.0.1:5175
bun test
bun run build
git diff --check
```

最终浏览器检查为基础导航 39 项、功能与异常路径 65 项，共 104 项断言，通过后没有未捕获组件错误，也没有后端写请求。39 项中包括 9 次 Tab 圈定检查，104 项不能解读为 104 条真实业务端到端流程。检查使用隔离的 headless Chrome，加载真实生产 Vue 组件、UI store 和快捷键运行时；服务端业务回调使用内存数据，预览网络请求被截获。

最新主分支集成后的 `bun run build` 通过，包含导入规则、国际化完整性、TypeScript 与生产打包；`git diff --check` 通过。完整前端测试为 **863 通过、6 失败，共 869 项、235 个文件**。快捷键解析与偏好/预览尺寸的两个新增测试文件共 8 项通过。上述 104 项浏览器断言也已在 rebase 后重新全部通过。

6 个失败都已在未修改的最新 `master`（`7506556c3`）上独立复现；对应 6 个文件的基线结果为 **52 通过、6 失败，共 58 项**，与集成后的失败用例一致。本次键盘修复没有新增测试失败，完整测试也不能描述为全部通过。

| 失败文件 | 已复现的主分支问题 |
| --- | --- |
| `performanceFollowupRendering.test.ts` | Git diff 请求队列用例的 `requests[index].pending` 为 undefined |
| `attachmentRowVocabularyContract.test.ts` | 附件标题源码契约断言不匹配 |
| `messageListRuntimePartContract.test.ts` | 消息列表源码契约仍要求已移除的临时 part 状态 |
| `workspaceChanges.test.ts` | 刷新失败用例期望清空列表，与保留已加载数据的行为不匹配 |
| `transcriptPartPresentation.test.ts` | 权限投影用例期望 pending 状态，实际为 completed |
| `realtimeConsistency.test.ts` | 相同 revision 的重新读取未保留用例期望的 live part 对象 |

最新集成结果、日志和截图保存在 `.tmp-artifacts/keyboard-navigation/integration/`：`verification.json`、`workflow-verification.json`、`frontend-tests.log`、`build.log`、`baseline-tests.log`，以及桌面、小屏、横屏帮助和快捷键编辑器截图。原始分支的验证记录仍保留在其上级目录。

## 验证边界

真实后端需要登录。本次没有读取认证信息或绕过登录，也没有创建真实会话、发送消息、移动用户文件、修改服务端设置或执行 Git 写命令。浏览器夹具验证的是实际组件的键盘事件、焦点、可见状态与回调接线，不能替代带真实后端成功/失败结果的业务端到端验收。

外部网页受浏览器同源策略控制。应用可为可访问的同源主文档提供 F6 返回，但无法承诺接管任意跨源页面、外部页面自己的焦点陷阱、内嵌子框架或系统快捷键。跨源主预览提供独立标签页等价操作，返回依赖浏览器自身的标签页切换/关闭操作。headless Chrome 的页面按键检查也不能证明 macOS、Windows、Linux 与 Safari、Firefox、实体键盘布局/输入法的所有系统级冲突都已消除。

控件角色、名称和关联已补齐并检查 DOM；本轮没有使用 VoiceOver、NVDA 或其他读屏软件。上述边界属于后端及实体平台验收范围，应用中原先列出的 14 类键盘入口缺口已落实为代码。
