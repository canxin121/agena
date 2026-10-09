//! Definition for bounded canonical content reads dispatched by ToolExecutor.

use crate::part::ContentReadToolInput;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};

pub(crate) struct ContentPlugin;

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena", name = "content", version = env!("CARGO_PKG_VERSION"),
    summary = "Read canonical resource-backed text and tool output with resumable byte-bounded pages.",
    translations(
        locale("zh-CN", summary = "读取规范内容资源中的文本和工具输出，并可按字节游标续读。"),
        locale("zh-TW", summary = "讀取標準內容資源中的文字與工具輸出，並可依位元組游標續讀。"),
        locale("ja-JP", summary = "正規コンテンツリソースのテキストやツール出力を、バイト単位のカーソルで再開しながら読み取ります。"),
        locale("ko-KR", summary = "표준 콘텐츠 리소스의 텍스트와 도구 출력을 바이트 커서로 이어서 읽습니다."),
        locale("fr-FR", summary = "Lire le texte et les sorties d’outils des ressources canoniques, avec reprise par curseur et limite en octets."),
        locale("de-DE", summary = "Text und Werkzeugausgaben aus kanonischen Inhaltsressourcen mit Bytecursor fortsetzbar lesen."),
        locale("es-ES", summary = "Lee texto y resultados de herramientas de recursos canónicos y permite continuar desde un cursor de bytes."),
        locale("hi-IN", summary = "मानक सामग्री संसाधनों से पाठ और टूल आउटपुट पढ़ें; बाइट कर्सर से आगे पढ़ना जारी रख सकते हैं।"),
        locale("ar-SA", summary = "اقرأ النص ومخرجات الأدوات من موارد المحتوى المعتمدة، مع إمكانية المتابعة بمؤشر للبايتات."),
        locale("pt-BR", summary = "Leia textos e saídas de ferramentas de recursos canônicos, com retomada por cursor e limite de bytes.")
    )
)]
impl ContentPlugin {
    #[tool(
        tags(query, read_only),
        summary = "Read retained source text from a content resource in this session.",
        translations(
            locale("zh-CN", summary = "读取当前会话中内容资源保留的原始文本。"),
            locale("zh-TW", summary = "讀取目前工作階段中內容資源保留的原始文字。"),
            locale(
                "ja-JP",
                summary = "現在のセッションにあるコンテンツリソースの保持テキストを読み取ります。"
            ),
            locale(
                "ko-KR",
                summary = "현재 세션의 콘텐츠 리소스에 보존된 원문을 읽습니다."
            ),
            locale(
                "fr-FR",
                summary = "Lire le texte conservé dans une ressource de contenu de cette session."
            ),
            locale(
                "de-DE",
                summary = "Den gespeicherten Quelltext einer Inhaltsressource dieser Sitzung lesen."
            ),
            locale(
                "es-ES",
                summary = "Lee el texto conservado en un recurso de contenido de esta sesión."
            ),
            locale("hi-IN", summary = "इस सत्र के सामग्री संसाधन में सुरक्षित मूल पाठ पढ़ें।"),
            locale("ar-SA", summary = "اقرأ النص المحفوظ في مورد محتوى ضمن هذه الجلسة."),
            locale(
                "pt-BR",
                summary = "Leia o texto retido em um recurso de conteúdo desta sessão."
            )
        ),
        help = "Use resource_id from a Part or tool result field. The first read omits epoch/after/offset. Continue with next_position.after.epoch as epoch, next_position.after.sequence as after and next_position.offset as offset; offsets safely resume inside a single large UTF-8 record. max_bytes defaults to 4096 (4–4096 raw text bytes); slices preserve whitespace and stdout/stderr. has_more means more retained records, gap signals retention loss, and capture state is independent of process success. Non-text document/terminal records advance the position without fabricated text. This is a diagnostic read, not a wait or polling mechanism."
    )]
    async fn invoke_read(
        &self,
        context: &ToolInvokeContext<'_>,
        input: ContentReadToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let input = serde_json::to_value(input)
            .map_err(|error| agena_plugin_host::PluginError::invalid_params_error(&error))?;
        super::router::invoke_tool("content", input, context.session_id, context.call_id)
    }
}
