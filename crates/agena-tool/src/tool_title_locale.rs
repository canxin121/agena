//! UI-only translations for composed tool headlines.
//!
//! `agena_tool` composes English headlines such as
//! `Execute command · cargo test · passed` and stores them with the durable
//! transcript facts. A client renders the same headline in its own language by
//! mapping the known vocabulary segment by segment: action phrases, status and
//! fact phrases, and count units. Dynamic values (paths, commands, ids, codes)
//! are never translated, and a locale without a vocabulary keeps the English
//! headline, so a missing translation never blanks a row.

/// Separator used by [`crate::compose_tool_title`] between headline parts.
const SEGMENT_SEPARATOR: &str = " · ";

/// Translate a composed tool headline into `locale`.
pub fn localize_tool_title(title: &str, locale: &str) -> String {
    let Some(table) = Vocabulary::for_locale(locale) else {
        return title.to_owned();
    };
    let localized = title
        .split(SEGMENT_SEPARATOR)
        .map(|segment| table.segment(segment.trim()))
        .collect::<Vec<_>>()
        .join(SEGMENT_SEPARATOR);
    localized
}

#[derive(Clone, Copy)]
struct Vocabulary {
    /// `(english, simplified chinese, traditional chinese)`.
    actions: &'static [(&'static str, &'static str, &'static str)],
    facts: &'static [(&'static str, &'static str, &'static str)],
    units: &'static [(&'static str, &'static str, &'static str)],
    simplified: bool,
}

impl Vocabulary {
    fn for_locale(locale: &str) -> Option<Self> {
        let normalized = locale.trim().replace('_', "-").to_ascii_lowercase();
        let language = normalized.split('-').next().unwrap_or_default();
        if language != "zh" {
            return None;
        }
        // Region and script decide between the two Chinese vocabularies; a
        // bare `zh` follows the repository default of simplified Chinese.
        let traditional = normalized.contains("hant")
            || normalized.contains("tw")
            || normalized.contains("hk")
            || normalized.contains("mo");
        Some(Self {
            actions: ACTIONS,
            facts: FACTS,
            units: UNITS,
            simplified: !traditional,
        })
    }

    /// Localize one ` · `-separated headline segment.
    fn segment(&self, segment: &str) -> String {
        if segment.is_empty() {
            return String::new();
        }
        if let Some(translated) = self.exact(self.actions, segment) {
            return translated;
        }
        if let Some(translated) = self.subject_action(segment) {
            return translated;
        }
        if let Some(translated) = self.exact(self.facts, segment) {
            return translated;
        }
        if let Some(translated) = self.exit_code(segment) {
            return translated;
        }
        if let Some(translated) = self.counted_unit(segment) {
            return translated;
        }
        segment.to_owned()
    }

    fn exact(
        &self,
        table: &'static [(&'static str, &'static str, &'static str)],
        segment: &str,
    ) -> Option<String> {
        table.iter().find_map(|(english, simplified, traditional)| {
            (*english == segment).then(|| {
                if self.simplified {
                    (*simplified).to_owned()
                } else {
                    (*traditional).to_owned()
                }
            })
        })
    }

    /// `Read README.md` keeps its subject and localizes the action.
    fn subject_action(&self, segment: &str) -> Option<String> {
        SUBJECT_ACTIONS.iter().find_map(|english| {
            let rest = segment.strip_prefix(english)?;
            let rest = rest.strip_prefix(' ')?;
            if rest.trim().is_empty() {
                return None;
            }
            let action = self.exact(self.actions, english)?;
            Some(format!("{action} {rest}"))
        })
    }

    /// `exit 3` keeps the code and localizes the label.
    fn exit_code(&self, segment: &str) -> Option<String> {
        let code = segment.strip_prefix("exit ")?.trim();
        if code.is_empty() || !code.chars().all(|character| character.is_ascii_digit()) {
            return None;
        }
        Some(if self.simplified {
            format!("退出码 {code}")
        } else {
            format!("退出碼 {code}")
        })
    }

    /// `12 lines`, `2/3 tools`, `36 matches` keep the count and localize the
    /// unit word.
    fn counted_unit(&self, segment: &str) -> Option<String> {
        for (unit, simplified, traditional) in self.units {
            let Some(count) = segment.strip_suffix(unit) else {
                continue;
            };
            if !is_count(count.trim_end()) {
                continue;
            }
            let unit = if self.simplified { simplified } else { traditional };
            return Some(format!("{}{unit}", count.trim_end()));
        }
        None
    }
}

/// Whether a fragment is a plain count: digits with `/`, `.`, `%`, `–` or a
/// trailing `+`.
fn is_count(fragment: &str) -> bool {
    let fragment = fragment.trim();
    if fragment.is_empty() {
        return false;
    }
    let mut digits = 0usize;
    for character in fragment.chars() {
        match character {
            '0'..='9' => digits += 1,
            '/' | '.' | '%' | '-' | '+' | '\u{2013}' | '~' => {}
            _ => return false,
        }
    }
    digits > 0
}

/// Action phrases that are composed as `Action subject` instead of
/// `Action · subject`; mirrors `action_is_noun_phrase_with_subject`.
const SUBJECT_ACTIONS: &[&str] = &[
    "Read",
    "Write",
    "Create",
    "Update",
    "Delete",
    "Open page",
    "Fetch page",
    "View image",
    "Inspect file",
    "Inspect symbol",
    "Read memory",
    "Save memory",
    "Delete memory",
    "Update settings",
    "Reset settings",
    "Edit text",
    "Edit image",
    "Rename session",
];

/// Action phrases produced by `tool_action_label`.
const ACTIONS: &[(&str, &str, &str)] = &[
    ("Read", "读取", "讀取"),
    ("Read files", "读取多个文件", "讀取多個檔案"),
    ("Write", "写入", "寫入"),
    ("Apply patch", "应用补丁", "套用修補"),
    ("Find files", "查找文件", "尋找檔案"),
    ("Search files", "搜索文件", "搜尋檔案"),
    ("Replace text", "替换文本", "取代文字"),
    ("Inspect file", "检查文件", "檢查檔案"),
    ("Search AST", "搜索 AST", "搜尋 AST"),
    ("Inspect syntax tree", "检查语法树", "檢查語法樹"),
    ("Rewrite AST", "重写 AST", "重寫 AST"),
    ("Read document", "读取文档", "讀取文件"),
    ("Read content", "读取内容", "讀取內容"),
    ("Run command", "运行命令", "執行命令"),
    ("Execute command", "执行命令", "執行命令"),
    ("Spawn background command", "启动后台命令", "啟動背景命令"),
    ("Watch command output", "监听命令输出", "監看命令輸出"),
    ("Open interactive terminal", "打开交互终端", "開啟互動終端"),
    ("Read terminal output", "读取终端输出", "讀取終端輸出"),
    ("List processes", "列出进程", "列出程序"),
    ("Show process logs", "查看进程日志", "檢視程序日誌"),
    ("Stop process", "停止进程", "停止程序"),
    ("Interact with terminal", "与终端交互", "與終端互動"),
    ("Resize terminal", "调整终端大小", "調整終端大小"),
    ("Signal terminal", "向终端发送信号", "傳送訊號至終端"),
    ("Start monitor", "启动监视", "啟動監看"),
    ("Stop monitor", "停止监视", "停止監看"),
    ("Search tools", "搜索工具", "搜尋工具"),
    ("List tools", "列出工具", "列出工具"),
    ("Read tool help", "读取工具说明", "讀取工具說明"),
    ("List tool tags", "列出工具标签", "列出工具標籤"),
    ("Call tool", "调用工具", "呼叫工具"),
    ("List plugins", "列出插件", "列出外掛"),
    ("Search plugins", "搜索插件", "搜尋外掛"),
    ("List plugin tags", "列出插件标签", "列出外掛標籤"),
    ("Ask user", "询问用户", "詢問使用者"),
    ("Send notification", "发送通知", "傳送通知"),
    ("Fetch page", "抓取页面", "抓取頁面"),
    ("Read pages", "读取多个页面", "讀取多個頁面"),
    ("Continue reading page", "继续读取页面", "繼續讀取頁面"),
    ("Search crawled pages", "搜索已抓取页面", "搜尋已抓取頁面"),
    ("Read memory", "读取记忆", "讀取記憶"),
    ("Save memory", "保存记忆", "儲存記憶"),
    ("Delete memory", "删除记忆", "刪除記憶"),
    ("Update settings", "更新设置", "更新設定"),
    ("Reset settings", "重置设置", "重設設定"),
    ("Edit text", "编辑文本", "編輯文字"),
    ("Edit image", "编辑图片", "編輯圖片"),
    ("Rename session", "重命名会话", "重新命名工作階段"),
    ("Open page", "打开页面", "開啟頁面"),
    ("View image", "查看图片", "檢視圖片"),
    ("Inspect symbol", "检查符号", "檢查符號"),
    ("Create", "创建", "建立"),
    ("Update", "更新", "更新"),
    ("Delete", "删除", "刪除"),
];

/// Status and fact phrases composed into a completed headline.
const FACTS: &[(&str, &str, &str)] = &[
    ("passed", "通过", "通過"),
    ("failed", "失败", "失敗"),
    ("cancelled", "已取消", "已取消"),
    ("declined", "已拒绝", "已拒絕"),
    ("timed out", "超时", "逾時"),
    ("completed", "已完成", "已完成"),
    ("running", "运行中", "執行中"),
    ("queued", "排队中", "排隊中"),
    ("pending", "等待中", "等待中"),
    ("permission denied", "权限被拒绝", "權限被拒"),
    ("response received", "已收到回复", "已收到回覆"),
    ("image response received", "已收到图片回复", "已收到圖片回覆"),
    ("more available", "还有更多", "還有更多"),
    ("truncated", "已截断", "已截斷"),
    ("no output", "无输出", "無輸出"),
    ("No output.", "无输出。", "無輸出。"),
    ("search was partial or truncated", "搜索不完整或已截断", "搜尋不完整或已截斷"),
    ("1 file changed", "1 个文件已更改", "1 個檔案已變更"),
];

/// Count units appended to a numeric prefix (`12 lines`, `2/3 tools`).
const UNITS: &[(&str, &str, &str)] = &[
    (" lines", "行", "行"),
    (" items", "项", "項"),
    (" matches", "处匹配", "處符合"),
    (" files", "个文件", "個檔案"),
    (" tools", "个工具", "個工具"),
    (" chunks", "个分块", "個分塊"),
    (" events", "个事件", "個事件"),
    (" paths", "个路径", "個路徑"),
    (" results", "个结果", "個結果"),
    (" packages", "个包", "個套件"),
    (" steps", "个步骤", "個步驟"),
    (" tasks", "个任务", "個任務"),
    (" sources", "个来源", "個來源"),
    (" sessions", "个会话", "個工作階段"),
    (" servers", "个服务器", "個伺服器"),
    (" tools available", "个工具可用", "個工具可用"),
];

#[cfg(test)]
mod tests {
    use super::localize_tool_title;

    #[test]
    fn simplified_chinese_localizes_actions_facts_and_units() {
        assert_eq!(
            localize_tool_title("Execute command · cargo test · passed", "zh-CN"),
            "执行命令 · cargo test · 通过"
        );
        assert_eq!(
            localize_tool_title(
                "Search tools · memory save durable knowledge · 2/3 tools · more available",
                "zh-CN"
            ),
            "搜索工具 · memory save durable knowledge · 2/3个工具 · 还有更多"
        );
        assert_eq!(
            localize_tool_title("Read README.md · 12 lines", "zh_CN"),
            "读取 README.md · 12行"
        );
        assert_eq!(
            localize_tool_title("Execute command · npm test · exit 3", "zh-CN"),
            "执行命令 · npm test · 退出码 3"
        );
    }

    #[test]
    fn traditional_chinese_uses_its_own_vocabulary() {
        assert_eq!(
            localize_tool_title("Execute command · cargo test · passed", "zh-TW"),
            "執行命令 · cargo test · 通過"
        );
        assert_eq!(
            localize_tool_title("Read README.md · 12 lines", "zh-Hant"),
            "讀取 README.md · 12行"
        );
    }

    #[test]
    fn unsupported_locales_and_dynamic_subjects_stay_english() {
        assert_eq!(
            localize_tool_title("Execute command · cargo test · passed", "de-DE"),
            "Execute command · cargo test · passed"
        );
        // Subjects keep their own text; only the vocabulary is translated.
        assert_eq!(
            localize_tool_title("Read /tmp/notes.md · 12 lines", "zh-CN"),
            "读取 /tmp/notes.md · 12行"
        );
        // An unknown segment keeps its own text instead of being dropped.
        assert_eq!(
            localize_tool_title("Execute command · cargo test · custom fact", "zh-CN"),
            "执行命令 · cargo test · custom fact"
        );
    }
}
