# Agena TUI 快捷键与按键参考

本文档记录 Agena TUI 当前实际实现的全部应用级快捷键、页面按键、通用列表导航和文本编辑键。聊天主界面保留 Vim/Composer 键位；二级页面采用可见控件和结构键，不再为页面功能分配普通字母快捷键。内容对应集中式 keymap，不以状态栏是否显示为准。

所有可聚焦分栏统一使用 `Tab` 向前、`Alt+Tab` 向后循环焦点；`BackTab`（通常是 `Shift+Tab`）也是向后切换的兼容键。只有恰好由左右两个可交互栏组成的简单页面，才额外允许 `←/→` 直接切到左栏或右栏。包含三处以上焦点、操作栏、工具栏、横向单元格或嵌套区域的复杂页面只用 Tab 系列键跨栏，`←/→` 留给当前栏内部的真实横向操作。`Esc` 返回，`↑/↓` 逐项导航，`Enter` 激活，`Space` 只用于多选。`PageUp/PageDown`、`Home/End` 以及同义字母键不再重复列表或焦点导航；唯一例外是特殊审核页中需要独立滚动长正文的 `PageUp/PageDown`。聊天主界面的 Transcript 和 Composer 也遵循同一套 Tab 正反向焦点切换规则，同时保留各自的 Vim/Composer 键位。

主要代码入口：

- `apps/agena-cli/src/tui_keymap/core.rs`：主页面、会话、Transcript、Composer 和通用弹窗。
- `apps/agena-cli/src/tui_keymap/usage.rs`：Usage Dashboard。
- `apps/agena-cli/src/tui_keymap/studio.rs`：Settings、Agent、Permission、Provider 和 Model Catalog。
- `apps/agena-cli/src/tui_keymap/plugin.rs`：Plugin Workbench。
- `apps/agena-cli/src/tui_keymap/composer.rs`：Composer 默认键位。
- `crates/agena-tui-components/src/keymap.rs`：通用列表、滚动和输入弹窗。
- `crates/agena-tui-components/src/editor.rs`：Shell/Emacs 风格文本编辑。

## 约定与处理优先级

- `Alt+Tab` 表示反向焦点切换；如果桌面环境截获该组合，可使用 `BackTab`，后者通常就是 `Shift+Tab`。
- `↑/↓/←/→` 分别表示方向键。
- 表格中的大写字符表示对应的大写按键，例如 `U`、`N`、`P`。
- 普通页面按键要求精确修饰键匹配。`Ctrl+K` 不会再被当成普通 `k`，`Alt+R` 不会再被当成普通 `r`。
- `Ctrl`、`Alt`、`Shift`、`Super`、`Hyper` 和 `Meta` 都参与修饰键匹配。
- `BackTab`、大写字符和 `?`、`+` 等需要 Shift 的符号仍兼容终端可能附带的 Shift 标志。

按键处理优先级为：

```text
历史输入窗口对 Ctrl+C 的局部关闭
→ 全局 Ctrl+C
→ 全局 Ctrl+H 上下文帮助
→ 当前 Overlay
→ 当前 Route
→ 主页面共享按键
→ 当前焦点页面
→ 通用文本编辑器
```

Composer 内部的优先级为：

```text
历史输入窗口
→ 文件提及建议
→ Slash 命令建议
→ Composer 内联条目
→ Composer 快捷键
→ 通用文本编辑器
```

## 全局按键

全局按键在页面、Route、Overlay 和编辑器之前处理。

| 按键 | 行为 |
|---|---|
| `Ctrl+C` 第一次 | 模型运行中时请求取消运行；否则提示再次按下退出 |
| `Ctrl+C` 第二次 | 在约 600ms 内再次按下，退出 TUI |
| `Ctrl+H` | 打开当前界面的专属帮助；帮助已打开时再次按下可关闭 |

例外：历史输入窗口打开时，精确的 `Ctrl+C` 会先关闭历史窗口并恢复原始 Composer 草稿，不会触发全局中断。

主页面没有 `q` 退出；当前键盘退出方式是连续两次 `Ctrl+C`。

## 主会话页面共享按键

`Tab` / `Alt+Tab` 在 Transcript 和 Composer 之间正向／反向循环焦点，在 Composer 的历史搜索、建议列表或内联条目正在接管键盘时除外。下表其余按键只在焦点不位于 Composer 时生效。

| 按键 | 行为 |
|---|---|
| `Tab` / `Alt+Tab` | 下一个／上一个主页面焦点区域 |
| `/` | 将焦点切到 Transcript，打开向下搜索 |
| `?` | 将焦点切到 Transcript，打开向上搜索 |
| `n` | 有 Transcript 搜索时跳到下一个匹配；没有搜索时无操作 |
| `N` | 有 Transcript 搜索时跳到上一个匹配；没有搜索时无操作 |
| `Ctrl+N` | 创建新会话 |
| `r` | 继续当前被阻塞或待处理的会话 |
| `U` | 打开 Usage Dashboard |

`?` 不再打开 Help。`Ctrl+H` 在所有 Route、Overlay、编辑器和聊天焦点中打开上下文帮助，`/help` 也会打开当前界面的同一帮助。`Ctrl+F` 和 `/find` 不再提供搜索功能。

## Sessions 会话列表

仅在 Sessions Pane 获得焦点时生效。

| 按键 | 行为 |
|---|---|
| `1` | 显示全部会话 |
| `2` | 只显示根会话 |
| `3` | 显示当前会话子树 |
| `m` | 循环切换会话视图模式 |
| `↑` / `k` | 上一条会话，必要时惰性加载更多 |
| `↓` / `j` | 下一条会话，必要时惰性加载更多 |
| `PageUp` | 向上移动 10 条 |
| `PageDown` | 向下移动 10 条并按需加载 |
| `Home` | 第一条会话 |
| `End` | 当前已加载列表的最后一条，并按需加载 |
| `Enter` | 打开选中会话并进入 Transcript |

## Transcript VIEW 模式

### 模式、选择和折叠

| 按键 | 行为 |
|---|---|
| `i` | 进入 Composer INSERT 模式 |
| `Enter` | 展开或收起当前选中的可折叠节点 |
| `←` / `h` | 选择上一个消息或内容块 |
| `→` / `l` | 选择下一个消息或内容块 |

`h/l` 是按块选择，不是横向滚动。

### 滚动

| 按键 | 行为 |
|---|---|
| `↑` / `k` | 向上滚动一行并更新块选择 |
| `↓` / `j` | 向下滚动一行并更新块选择 |
| `PageUp` / `Ctrl+B` | 向上翻一页 |
| `PageDown` / `Space` | 向下翻一页 |
| `Shift+Space` | 向上翻一页 |
| `Ctrl+U` | 向上翻半页 |
| `Ctrl+D` | 向下翻半页 |
| `Home` / `g` | Transcript 顶部，并按需加载旧消息 |
| `End` / `G` | Transcript 底部 |

### 数字移动前缀

| 按键 | 行为 |
|---|---|
| `1`–`9` | 开始或追加移动次数 |
| `0` | 已有数字前缀时追加 `0` |
| 数字后接 `h/j/k/l` | 按指定次数移动 |

例如 `5j`、`10k`、`3l`。数字前缀只用于 Transcript 移动。

### 复制

| 按键 | 行为 |
|---|---|
| `y` | 复制当前选中的 Transcript 节点 |
| `Y` | 复制当前可见 Transcript |
| `C` | 复制当前已加载的全部 Transcript |
| `c` | 复制最后一条 assistant 消息 |

## Composer INSERT 模式

### 发送、Steer 和排队

| 按键 | 行为 |
|---|---|
| `Enter` | 空闲时发送；运行中时加入本地待发送队列 |
| `Ctrl+Enter` | 空闲时发送；运行中时尝试 steer，失败后排队 |
| `Shift+Enter` / `Alt+Enter` / `Ctrl+J` | 插入换行 |
| `Esc` | 离开 Composer，返回 Transcript VIEW 模式 |

Slash 命令始终直接在本地执行，不进入消息队列。粘贴检测期间的内部 Enter 会作为换行处理。

### 历史、队列和输入状态

| 按键 | 行为 |
|---|---|
| `Ctrl+R` / `Alt+↑` | 打开历史输入窗口 |
| `Ctrl+↑` | 从待发送队列取回一条消息进行编辑 |
| `↑` | 多行编辑器向上一行，不再打开历史或取回队列 |
| `Ctrl+L` | 清空当前 Composer 输入 |

### 附件和外部工具

| 按键 | 行为 |
|---|---|
| `F2` | 进入或退出 Composer 内联条目选择 |
| `F3` / `Ctrl+O` / `Alt+O` | 打开文件附件选择器 |
| `F4` / `Alt+E` | 使用外部编辑器编辑 Composer |
| `F6` / `Alt+I` | 从剪贴板附加图片 |
| `Alt+U` | 打开待处理的用户输入请求 |
| `Alt+A` | 打开待处理的权限请求 |

Composer 默认键位存放在 `ComposerKeyBindings` 中。当前 `TuiConfig::load()` 尚未从用户配置读取自定义键位，因此以上是实际生效的默认值。

## Composer 内联条目选择

| 按键 | 行为 |
|---|---|
| `Esc` | 退出条目选择 |
| `BackTab` / `←` / `h` | 上一个条目 |
| `Tab` / `→` / `l` | 下一个条目 |
| `Delete` / `Backspace` / `d` | 删除选中条目 |
| `Enter` / `o` | 打开选中条目 |

文件附件会打开对应路径；大段粘贴没有文件路径，不能作为文件打开。

## 历史输入悬浮窗口

历史记录按从新到旧排列，并按需分页加载。

| 按键 | 行为 |
|---|---|
| `Esc` / `Ctrl+C` | 关闭历史窗口并恢复打开前的原始草稿 |
| `Enter` | 接受当前历史输入 |
| `Ctrl+R` / `↑` / `Alt+↑` | 选择更旧的一条，必要时加载下一页 |
| `↓` / `Alt+↓` | 选择更新的一条；越过最新端时关闭并恢复原草稿 |
| `Ctrl+S` | 选择更新的一条，但保持窗口打开 |
| 普通文本编辑键 | 编辑历史搜索词 |

## 文件提及和 Slash 命令建议

| 按键 | 行为 |
|---|---|
| `↑` / `Ctrl+P` | 上一个建议 |
| `↓` / `Ctrl+N` | 下一个建议 |
| `Esc` | 关闭建议 |
| `Tab` | 填入选中建议 |
| `Enter` | 接受选中建议 |

文件提及的 Enter 会附加文件。Slash 命令的 Enter 会补全并立即提交；Tab 只补全。

## Transcript 搜索输入框

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭搜索输入框 |
| `Enter` | 应用搜索词并按打开时的方向跳转 |
| 普通文本编辑键 | 编辑搜索词 |

关闭搜索框后使用 `n/N` 沿当前搜索方向或反方向跳转。

## 上下文 Help

Help 不再是汇总整个 TUI 的长文本页面。每个界面都有自己的帮助卡片，只展示当前界面的用途、快捷键、按键和必要的工作流提示。例如，在 Transcript 中只显示消息导航、折叠、搜索和复制；在 Provider Studio 中只显示面板切换、选择和可见操作；在文本编辑器中只显示编辑与提交方式。

| 按键 | 行为 |
|---|---|
| `Ctrl+H` | 在任意界面打开或关闭当前上下文的 Help |
| `Esc` | 关闭 Help |
| `↑` | 向上滚动一行 |
| `↓` | 向下滚动一行 |

Help 以居中的圆角窗口显示，顶部提供当前界面名称与简介，按键按 Navigation、Actions、Editing、Workflow、Search 和 Selection 等卡片分组。窄终端会自动切换为上下堆叠的按键说明。Help 打开后会拦截普通页面输入，不会意外触发背后的界面操作。

## 通用可搜索列表

Choice、文件附件、路径浏览、会话搜索、Picker、模型选择和 Timeline 等页面继承以下基础键位：

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭当前列表或选择器 |
| `↑` | 上一项 |
| `↓` | 下一项 |
| `Home` / `End` | 移动搜索输入光标到开头／结尾 |
| 普通文本编辑键 | 编辑搜索内容 |

搜索输入激活时只使用 `↑/↓` 导航列表。`PageUp/PageDown` 和 `Ctrl+Home/Ctrl+End` 不再提供重复的列表跳转；`Home/End` 仅保留文本光标语义。所有普通字符都保留给搜索文本输入。

## Choice 通用选项弹窗

除通用可搜索列表键位外：

| 按键 | 行为 |
|---|---|
| `Enter` | 确认选中项 |

## 文件附件选择器

除通用可搜索列表键位外：

| 按键 | 行为 |
|---|---|
| `Enter` | 附加选中的文件 |

## 路径浏览器

除通用可搜索列表键位外：

| 按键 | 行为 |
|---|---|
| `Enter` | 目录行进入目录；`../` 行返回父目录；文件或自定义路径行接受当前路径 |

## 会话搜索和会话切换页

除通用可搜索列表键位外：

| 按键 | 行为 |
|---|---|
| `Enter` | 打开选中会话并进入 Composer |

在当前页第一项继续按 `↑` 会惰性加载上一页；在最后一项继续按 `↓` 会惰性加载下一页。分页不占用额外快捷键。

## 通用 Picker

包括 Agent、Provider 和 Permission Rule 等 Picker。

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `↑` / `↓` | 上一项／下一项 |
| `Enter` | 接受或打开选中项 |

Agent、Provider 和 Permission Rule Picker 把“新建”显示为真实列表项；规则删除通过进入规则页后选择可见的 Revoke 操作完成。

## Session Model 模型选择页

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `↑` / `↓` | 上一个／下一个模型 |
| `Enter` | 应用选中模型 override |
| 普通文本编辑键 | 搜索模型 |

## Timeline 事件时间线

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `↑` / `↓` | 上一个／下一个事件 |
| `Enter` | 跳到事件关联的消息 |
| 普通文本编辑键 | 搜索事件 |

## 确认弹窗

| 按键 | 行为 |
|---|---|
| `Esc` | 取消 |
| `Enter` | 确认 |

## 单行和多行编辑弹窗

会话重命名、Agent 创建、设置值和搜索等单行输入：

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭或取消 |
| `Enter` | 提交 |
| 通用文本编辑键 | 编辑内容 |

Agent、Permission、Provider 和 Plugin 配置中的多行编辑器：

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭编辑器 |
| `Ctrl+S` | 保存或提交 |
| 通用文本编辑键 | 编辑内容 |

编辑器打开时优先处理按键，外层页面快捷键暂时不执行。

## 权限请求弹窗

| 按键 | 行为 |
|---|---|
| `Esc` | 返回上一级；最外层时关闭弹窗 |
| `↑` / `↓` | 上一个／下一个选项 |
| `Enter` | 激活当前选项 |

“详情”现在是 Allow、Deny 和 Edit Rule 同级的可见选项，不再使用隐藏的 `i` 键。详细信息页使用 `Esc` 返回。

## 用户输入请求：问题页

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭弹窗 |
| `Enter` | 提交当前问题答案 |
| `Ctrl+X` | 取消整个请求 |
| `↑` / `↓` | 上一个／下一个选项 |
| `Tab` / `Alt+Tab` | 下一个／上一个问题或 Review，并在页面间循环 |
| `Space` | 切换当前选项 |
| `e` | 编辑自定义答案 |
| `Delete` | 清空当前答案 |

自定义答案编辑状态：

| 按键 | 行为 |
|---|---|
| `Esc` | 退出自定义编辑 |
| `Enter` | 接受自定义内容 |
| `Ctrl+X` | 取消整个用户输入请求 |
| 通用文本编辑键 | 编辑内容 |

`Ctrl+D` 在普通问题状态和自定义编辑状态中都只保留给文本编辑语义，不再取消整个请求。

## 用户输入请求：Review 页

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `Enter` | 提交全部回答 |
| `Ctrl+X` | 取消请求 |
| `Tab` / `Alt+Tab` | 下一个／上一个问题或 Review，并在页面间循环 |
| `↑` / `↓` | 上一个／下一个问题 |
| `e` | 返回编辑选中的问题 |
| `Delete` | 清空选中问题答案 |

特殊审核决策页面复用该上下文：`↑/↓` 选择决策，`PageUp/PageDown` 仅用于滚动独立的长正文，`Enter` 提交决策。这里的翻页键与选择导航并非同一功能，因此予以保留。

## Usage Dashboard

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `Tab` / `Alt+Tab` | 在内容区及 Period、View、Provider、Model、Subagents、Sort、Refresh 可见控件之间向前／向后循环焦点 |
| `Enter` | 修改当前控件；内容区位于 Sessions 视图时打开选中会话 |
| `↑` / `↓` | 内容区上一行／下一行 |

统计周期、视图、过滤器、Subagents、排序和刷新都显示为可聚焦控件。原来的 `1-5`、`p/P`、`m/M`、`a`、`s`、`r`、`q` 和 Vim 字母别名均已移除。

## Settings Studio

`/settings` 是唯一的配置入口，固定包含 Models & Providers、Agents、Permissions、Plugins & Tools、Runtime & Session、Interface、Diagnostics 七个顶层分区。Permission Studio 和 Plugin Workbench 都从对应分区进入，不再提供独立的 `/permissions` 或 `/plugins` 命令。

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `←` / `→` | 直接切换到 Navigation／Items |
| `Tab` / `Alt+Tab` | 在 Navigation 和 Items 之间向前／向后循环焦点 |
| `↑` / `↓` | 当前区域上一项／下一项 |
| `Enter` | 激活或编辑选中设置 |

## Agent Studio

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `↑` / `↓` | 上一项／下一项 |
| `Enter` | 激活或编辑选中字段 |

权限编辑和打开配置来源已经是 Agent 字段列表底部的可见操作行，不再绑定 `p/o`；刷新在页面打开或保存后自动进行。

## Permission Studio

| 按键 | 行为 |
|---|---|
| `Esc` | Actions → Content → Navigation → 关闭页面 |
| `Tab` / `Alt+Tab` | 在 Navigation、Content 和底部可见 Actions 操作栏之间向前／向后循环焦点 |
| `←` / `→` | Actions 获得焦点时选择 Add、Edit、Rename、Duplicate 或 Delete |
| `↑` / `↓` | 当前列表上一项／下一项 |
| `Enter` | 重新应用当前导航，或激活当前内容项／可见操作按钮；不会跨栏切换焦点 |

原来的 `a/e/n/y/d/r` 命令已移除。操作栏只在当前 Permission 分区支持这些操作时显示，并始终作用于 Content 中保持选中的条目。

## Permission Rule Studio

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `↑` / `↓` | 上一项／下一项 |
| `Enter` | 激活或编辑当前规则字段 |

Browse Workspace、Browse Target、Save 和 Revoke 都是字段列表中的可见操作行，因此不再需要 `b/r`。

## Provider Studio 主页面

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `Tab` / `Alt+Tab` | 循环到下一个／上一个焦点区域 |
| `Space` | 切换当前 Adapter 或 Model 选中状态 |
| `↑` / `↓` | 上一项／下一项 |
| `Enter` | 编辑字段、打开模型，或激活字段列表中的可见操作行 |

创建 Provider 位于 Provider Picker 的真实列表项中。启动/继续认证、删除 Provider、发现模型、添加模型、删除选中 Adapter/Model、保存 Adapter 和保存 Provider 都是 Fields 面板中的可见操作行。原来的 `n/o/p/r/+/Delete/D/s/a/A/c/m` 页面命令已移除。

## Provider Detail 页面

| 按键 | 行为 |
|---|---|
| `Esc` | 返回 Provider Studio |
| `↑` / `↓` | 上一字段／下一字段 |
| `Enter` | 编辑选中字段 |

认证动作显示在 Provider 主页面，不在 Detail 页面重复绑定。

## Provider Model 页面

| 按键 | 行为 |
|---|---|
| `Esc` | 返回 Provider Studio |
| `↑` / `↓` | 上一字段／下一字段 |
| `Enter` | 编辑字段，或激活列表末尾的 Save Model / Delete Model 操作行 |

## Model Catalog

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 |
| `Tab` / `Alt+Tab` | 在模型列表和 Search、Refresh、Previous page、Next page 可见操作之间向前／向后循环焦点 |
| `Enter` | 激活当前可见操作 |
| `↑` / `↓` | 上一个／下一个模型 |

Search 操作打开搜索编辑器；编辑器中 `Esc` 关闭，`Enter` 提交查询，其他文本编辑键修改查询。原来的 `/`、`R` 和 `h/l` 已移除。

## Plugin Workbench：插件列表

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭 Plugin Workbench |
| `Tab` / `Alt+Tab` | 在搜索/列表与 Transport、Config、Refresh 可见控件之间向前／向后循环焦点 |
| `Enter` | 列表中打开选中 Plugin；控制栏中修改过滤器或刷新 |
| `↑` / `↓` | 上一个／下一个插件 |
| `Home` / `End` | 移动插件搜索输入光标到开头／结尾 |
| 其他普通文本编辑键 | 编辑插件搜索词 |

Transport、Config 和 Refresh 都显示为可聚焦控件，原来的 `Ctrl+R`、`Alt+T` 和 `Alt+C` 已移除。

## Plugin Detail 非 Config Tab

| 按键 | 行为 |
|---|---|
| `Esc` | 返回插件列表 |
| `Tab` / `Alt+Tab` | 循环到下一个／上一个 Detail Tab |
| `↑` / `↓` | 当前详情向上／向下滚动一行 |

## Plugin Detail：Config Tab

| 按键 | 行为 |
|---|---|
| `Esc` | 返回插件列表 |
| `Tab` / `Alt+Tab` | 循环到下一个／上一个 Config 焦点区域 |
| `Enter` | Toolbar 中执行 Validate、Reset All、Diff、Save 或 Restart；Editor 中激活选中单元格；Structure 中保持当前焦点 |
| `↑` / `↓` | 上一项／下一项 |
| `←` / `→` | 在当前 Toolbar 或 Editor 内选择上一个／下一个操作或单元格；不跨焦点区域 |

所有顶层操作都在 Toolbar 中可见；字段级类型、添加、重命名、删除和重置通过移动到对应 Type/Action/State 单元格后按 Enter 打开。Diff 打开时使用 Esc 关闭。原来的 `s/v/i/x/D/R/r/a/t/e` 和 Ctrl 组合均已移除。

## Plugin Config Actions 菜单

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭动作菜单 |
| `↑` / `↓` | 上一个／下一个动作 |
| `Enter` | 执行当前动作 |

## Plugin Config Selection 选择器

| 按键 | 行为 |
|---|---|
| `Esc` | 关闭选择器 |
| `↑` / `↓` | 上一项／下一项 |
| `Space` | 多选模式下切换当前项 |
| `Enter` | 确认选择 |

## Plugin Config Drilldown

| 按键 | 行为 |
|---|---|
| `Esc` | 返回上一级 Drilldown |
| `↑` / `↓` | 上一行／下一行 |
| `←` / `→` | 上一个／下一个单元格 |
| `Enter` | 激活当前单元格，包括编辑、类型、添加和动作菜单 |

## 通用文本编辑器

这些键适用于 Composer、搜索框、单行输入框和各种 Studio 编辑器。外层页面先占用的键不会继续传入编辑器。

### 光标移动

| 按键 | 行为 |
|---|---|
| `←` / `Ctrl+B` | 向左一个字符 |
| `→` / `Ctrl+F` | 向右一个字符 |
| `Ctrl+←` / `Alt+←` / `Alt+B` | 向左一个单词 |
| `Ctrl+→` / `Alt+→` / `Alt+F` | 向右一个单词 |
| `Home` | 当前行开头 |
| `End` | 当前行结尾 |
| `Ctrl+A` | 当前行开头；已在行首时可到上一行行首 |
| `Ctrl+E` | 当前行结尾；已在行尾时可到下一行行尾 |
| `↑` / `Ctrl+P` | 多行编辑器中向上一行 |
| `↓` / `Ctrl+N` | 多行编辑器中向下一行 |

### 删除和 Kill/Yank

| 按键 | 行为 |
|---|---|
| `Backspace` | 删除前一个字符；`Ctrl+H` 保留给上下文 Help |
| `Delete` | 删除当前字符；文本末尾时退格删除 |
| `Ctrl+D` | 删除当前字符 |
| `Alt+Backspace` / `Ctrl+Alt+H` / `Ctrl+W` | 删除前一个单词 |
| `Alt+Delete` | 删除后一个单词 |
| `Ctrl+U` | 删除到当前行开头 |
| `Ctrl+K` | 删除到当前行结尾 |
| `Ctrl+Y` | 粘贴编辑器内部最近一次 Kill 的内容 |

编辑器要求精确修饰键。`Ctrl+Shift+Left` 不会被当成 `Ctrl+Left`；`Ctrl+Super+Enter` 不会被当成 Composer 的 `Ctrl+Enter`。AltGr 输入仍按普通字符处理。

## 主页面已经移除的快捷键

以下按键不再是主页面全局命令：

| 按键 | 已移除的旧行为 |
|---|---|
| `q` | 退出 TUI |
| `s` / `Alt+S` | 打开会话切换器 |
| `b` | 打开会话分支历史 |
| `R` | 重命名当前会话 |
| `t` | 打开时间线 |
| `P` | 打开插件工作台 |
| `[` / `]` | 打开父会话／子会话选择器 |
| `e` | 导出对话 |
| `v` | 使用外部分页器打开对话 |
| `u` | 打开待处理用户输入 |
| `Alt+P` | 打开命令面板 |
| `Ctrl+F` | 打开 Find |
| `/find` | Find 命令 |

这些按键不再作为聊天主页面的全局命令；二级页面也不复用普通字母作为隐藏命令。需要输入文本时，普通字符会继续交给当前编辑器或搜索框。

## 维护要求

新增、删除或修改 TUI 按键时，应同时完成以下事项：

1. 在对应 `tui_keymap` 模块中修改物理按键到 `KeyAction` 的映射。
2. 页面 handler 只处理语义化 `KeyAction`，不要重新引入散落的 `KeyCode` 判断。
3. 通用列表、滚动或编辑器行为应修改 `agena-tui-components` 的集中式 keymap。
4. 更新状态栏、Help 多语言提示和本文档。
5. 为修饰键、页面上下文和按键冲突增加防回归测试。
