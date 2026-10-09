//! `agena.lsp` plugin: read-only observability of the configured
//! LSP servers plus model-visible navigation commands.

use std::sync::{Arc, RwLock};

use crate::LspConfig;
use serde::Serialize;

use crate::part::{
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
};
use crate::plugins::provided::router;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{HostClient, HostLspListServersResponse};
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};

fn lsp_settings_metadata() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            "",
            "LSP Plugin Config",
            "Shared defaults and per-server settings for the agena.lsp plugin.",
        ),
        (
            "/defaults",
            "Defaults",
            "Shared runtime settings applied to language servers unless a server overrides them.",
        ),
        (
            "/defaults/env",
            "Environment",
            "Environment variables injected into every configured language server process by default.",
        ),
        (
            "/defaults/root_markers",
            "Root Markers",
            "Marker files used to discover project roots when a server does not define its own routing markers.",
        ),
        (
            "/defaults/initialization_options",
            "Initialization Options",
            "Default JSON initialization options sent during server startup.",
        ),
        (
            "/servers",
            "Servers",
            "Named language server definitions keyed by server identifier.",
        ),
        (
            "/servers/*",
            "Server",
            "A single named language server definition.",
        ),
        (
            "/servers/*/process",
            "Process",
            "Executable command, arguments, and environment for this language server.",
        ),
        (
            "/servers/*/routing",
            "Routing",
            "File matching and root detection rules for this server.",
        ),
        (
            "/servers/*/session",
            "Session",
            "Per-server LSP session settings such as initialization options.",
        ),
        (
            "/servers/*/process/command",
            "Command",
            "Executable used to start the language server.",
        ),
        (
            "/servers/*/process/args",
            "Arguments",
            "Command-line arguments passed to the language server process.",
        ),
        (
            "/servers/*/process/env",
            "Environment",
            "Environment variables merged on top of the shared LSP defaults for this server.",
        ),
        (
            "/servers/*/routing/file_extensions",
            "File Extensions",
            "File extensions routed to this server. Leave empty to match all files.",
        ),
        (
            "/servers/*/routing/root_markers",
            "Root Markers",
            "Project-root markers used for this server. Leave empty to inherit the shared defaults.",
        ),
        (
            "/servers/*/session/initialization_options",
            "Initialization Options",
            "Server-specific JSON initialization options. Leave unset to inherit the shared defaults.",
        ),
    ]
}

pub(crate) struct LspPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

impl LspPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::internal("lsp plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::internal("lsp plugin invoked before init"))
    }

    fn invoke_routed_tool<T: serde::Serialize>(
        &self,
        tool_name: &str,
        args: T,
        session_id: i64,
        call_id: i64,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            tool_name,
            serde_json::to_value(args).map_err(|err| PluginError::invalid_params_error(&err))?,
            session_id,
            call_id,
        )
    }
}

#[derive(Debug, Serialize)]
struct LspServersOutput {
    servers: Vec<LspServerSummary>,
}

#[derive(Debug, Serialize)]
struct LspServerSummary {
    name: String,
    command: String,
    args: Vec<String>,
    file_extensions: Vec<String>,
    executable: Option<String>,
    command_available: Option<bool>,
    root_markers: Vec<String>,
    running_roots: Vec<String>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "lsp",
    version = env!("CARGO_PKG_VERSION"),
    summary = "LSP read-only observability and navigation tools.",
    translations(
        locale("zh-CN", summary = "通过 LSP 只读查看语言服务状态并导航代码。"),
        locale("zh-TW", summary = "透過 LSP 唯讀檢視語言伺服器狀態並瀏覽程式碼。"),
        locale("ja-JP", summary = "LSP を使って言語サーバーの状態を読み取り、コードをナビゲートします。"),
        locale("ko-KR", summary = "LSP로 언어 서버 상태를 읽고 코드 위치를 탐색합니다."),
        locale("fr-FR", summary = "Observer les serveurs de langage en lecture seule et naviguer dans le code via LSP."),
        locale("de-DE", summary = "Sprachserver per LSP schreibgeschützt prüfen und im Code navigieren."),
        locale("es-ES", summary = "Consulta en modo de solo lectura los servidores de lenguaje y navega por el código mediante LSP."),
        locale("hi-IN", summary = "LSP के माध्यम से भाषा सर्वर की स्थिति केवल पढ़ें और कोड में नेविगेट करें।"),
        locale("ar-SA", summary = "افحص خوادم اللغة للقراءة فقط وانتقل في الشيفرة عبر LSP."),
        locale("pt-BR", summary = "Consulte servidores de linguagem em modo somente leitura e navegue pelo código via LSP.")
    ),
    settings = LspConfig,
    settings_default = default,
    settings_metadata = lsp_settings_metadata(),
)]
impl LspPlugin {
    #[hook(init)]
    async fn init(
        &self,
        _ctx: agena_plugin_host::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::internal("lsp plugin host lock poisoned"))? = Some(host);
        Ok(agena_plugin_host::sdk::InitOutcome::ack(
            agena_plugin_host::sdk::Plugin::manifest(self),
        ))
    }

    #[tool(
        tags(query, lsp, discovery, read_only),
        summary = "List configured language servers and executable availability.",
        help = "Checks each command against its configured PATH from the workspace root without executing it; command_available=null means lookup was inconclusive. Presence does not guarantee successful initialization. Relative commands can resolve differently in individual project roots. running_roots lists initialized instances. Extension-specific servers take priority over catch-all servers; ties use lexical server name.",
        translations(
            locale(
                "zh-CN",
                summary = "列出已配置的语言服务器及其可执行文件是否可用。",
                help = "从工作区根目录按服务器配置的 PATH 检查命令，不会执行命令；`command_available=null` 表示无法确定是否存在。命令可用不代表服务器一定能初始化。相对命令在不同项目根目录下可能解析到不同位置。`running_roots` 列出已初始化的实例。针对特定扩展名的服务器优先于通用服务器；优先级相同时按服务器名称排序。"
            ),
            locale(
                "zh-TW",
                summary = "列出已設定的語言伺服器及其執行檔可用狀態。",
                help = "從工作區根目錄依伺服器設定的 PATH 檢查命令，不會執行命令；`command_available=null` 表示無法判定是否存在。命令存在不代表伺服器一定能初始化。相對命令在不同專案根目錄可能解析到不同位置。`running_roots` 列出已初始化的執行個體。副檔名專用伺服器優先於通用伺服器；同級時依伺服器名稱排序。"
            ),
            locale(
                "ja-JP",
                summary = "設定済み言語サーバーと実行ファイルの有無を一覧表示します。",
                help = "ワークスペースのルートから、設定された PATH に基づいて各コマンドを確認します。実行はしません。`command_available=null` は判定できなかったことを示し、存在しても初期化成功を保証しません。相対コマンドの解決先はプロジェクトルートごとに異なる場合があります。`running_roots` は初期化済みのインスタンスです。拡張子専用サーバーは汎用サーバーより優先され、同順位はサーバー名順です。"
            ),
            locale(
                "ko-KR",
                summary = "구성된 언어 서버와 실행 파일 사용 가능 여부를 나열합니다.",
                help = "작업 공간 루트에서 설정된 PATH를 기준으로 명령을 확인하며 실행하지는 않습니다. `command_available=null`은 확인할 수 없다는 뜻입니다. 명령이 존재해도 서버 초기화가 성공한다는 보장은 없습니다. 상대 경로 명령은 프로젝트 루트마다 다르게 해석될 수 있습니다. `running_roots`는 초기화된 인스턴스를 나타냅니다. 확장자 전용 서버가 범용 서버보다 우선하며, 우선순위가 같으면 서버 이름순입니다."
            ),
            locale(
                "fr-FR",
                summary = "Lister les serveurs de langage configurés et la disponibilité de leurs exécutables.",
                help = "Depuis la racine de l’espace de travail, vérifie chaque commande avec le PATH configuré sans l’exécuter. `command_available=null` signifie que la vérification n’a pas pu conclure ; la présence ne garantit pas l’initialisation. Les commandes relatives peuvent varier selon la racine du projet. `running_roots` répertorie les instances initialisées. Les serveurs propres à une extension sont prioritaires sur les serveurs génériques ; à égalité, le nom du serveur départage."
            ),
            locale(
                "de-DE",
                summary = "Konfigurierte Sprachserver und verfügbare Programme auflisten.",
                help = "Prüft jede Konfiguration vom Workspace-Stamm aus gegen den festgelegten PATH, ohne den Befehl auszuführen. `command_available=null` bedeutet, dass die Prüfung unklar blieb; ein vorhandener Befehl garantiert keine erfolgreiche Initialisierung. Relative Befehle können je nach Projektstamm anders aufgelöst werden. `running_roots` enthält initialisierte Instanzen. Erweiterungsspezifische Server haben Vorrang vor allgemeinen; bei Gleichstand entscheidet der Servername alphabetisch."
            ),
            locale(
                "es-ES",
                summary = "Enumera los servidores de lenguaje configurados y si sus ejecutables están disponibles.",
                help = "Comprueba cada comando desde la raíz del espacio de trabajo usando el PATH configurado, sin ejecutarlo. `command_available=null` indica que no se pudo determinar; que exista no garantiza que el servidor se inicialice. Los comandos relativos pueden resolverse de forma distinta según la raíz del proyecto. `running_roots` enumera las instancias inicializadas. Los servidores específicos de una extensión tienen prioridad sobre los genéricos; en caso de empate, se ordenan por nombre."
            ),
            locale(
                "hi-IN",
                summary = "कॉन्फ़िगर किए गए भाषा सर्वर और उनके executable की उपलब्धता दिखाएँ।",
                help = "कार्यस्थान की मूल डायरेक्टरी से कॉन्फ़िगर किए गए PATH के अनुसार हर कमांड जाँचता है, चलाता नहीं। `command_available=null` का अर्थ है कि उपलब्धता तय नहीं हो सकी; मौजूद होने से initialization सफल होना सुनिश्चित नहीं होता। अलग-अलग प्रोजेक्ट रूट में सापेक्ष कमांड अलग तरह से मिल सकते हैं। `running_roots` शुरू किए गए इंस्टेंस दिखाता है। एक्सटेंशन-विशिष्ट सर्वर सामान्य सर्वर से पहले चुने जाते हैं; बराबरी पर सर्वर नाम क्रम लागू होता है।"
            ),
            locale(
                "ar-SA",
                summary = "اعرض خوادم اللغة المُهيأة ومدى توفر ملفاتها التنفيذية.",
                help = "يفحص كل أمر انطلاقًا من جذر مساحة العمل ووفق PATH المُهيأ، من دون تشغيله. تعني `command_available=null` تعذر الجزم بالتوفر، كما أن وجود الأمر لا يضمن نجاح التهيئة. قد تُحل الأوامر النسبية إلى مواقع مختلفة بحسب جذر المشروع. تعرض `running_roots` النسخ التي تمت تهيئتها. تتقدم الخوادم الخاصة بامتداد على الخوادم العامة، وعند التعادل يُرتب الاسم معجميًا."
            ),
            locale(
                "pt-BR",
                summary = "Liste os servidores de linguagem configurados e a disponibilidade dos executáveis.",
                help = "Verifica cada comando a partir da raiz do espaço de trabalho usando o PATH configurado, sem executá-lo. `command_available=null` significa que não foi possível concluir; encontrar o comando não garante a inicialização. Comandos relativos podem apontar para locais diferentes conforme a raiz do projeto. `running_roots` lista as instâncias inicializadas. Servidores específicos de extensão têm prioridade sobre os genéricos; em caso de empate, vale a ordem alfabética do nome."
            )
        )
    )]
    async fn dispatch_servers(&self) -> SdkResult<ToolInvokeOutput> {
        let HostLspListServersResponse { servers } = self.host()?.lsp_list_servers().await?;
        let summary = LspServersOutput {
            servers: servers
                .into_iter()
                .map(|server| LspServerSummary {
                    name: server.name,
                    command: server.command,
                    args: server.args,
                    file_extensions: server.file_extensions,
                    executable: server.executable,
                    command_available: server.command_available,
                    root_markers: server.root_markers,
                    running_roots: server.running_roots,
                })
                .collect(),
        };
        let body = serde_json::to_string_pretty(&summary)
            .map_err(|err| PluginError::internal_error(&err))?;
        Ok(ToolInvokeOutput {
            title: "Language servers".to_string(),
            summary: format!("{} configured servers", summary.servers.len()),
            output_text: body,
            payload: Some(
                serde_json::to_value(&summary)
                    .map_err(|error| PluginError::internal_error(&error))?,
            ),
            metadata: Default::default(),
            attachments: Vec::new(),
        })
    }

    #[tool(
        tags(query, lsp, filesystem, read_only),
        summary = "Resolve symbol definitions.",
        translations(
            locale("zh-CN", summary = "查找符号定义。"),
            locale("zh-TW", summary = "尋找符號定義。"),
            locale("ja-JP", summary = "シンボルの定義を検索します。"),
            locale("ko-KR", summary = "심볼 정의를 찾습니다."),
            locale("fr-FR", summary = "Trouver les définitions d’un symbole."),
            locale("de-DE", summary = "Symboldefinitionen finden."),
            locale("es-ES", summary = "Busca las definiciones de un símbolo."),
            locale("hi-IN", summary = "सिंबल की परिभाषाएँ खोजें।"),
            locale("ar-SA", summary = "اعثر على تعريفات الرمز."),
            locale("pt-BR", summary = "Encontre as definições de um símbolo.")
        )
    )]
    async fn dispatch_definition(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspDefinitionToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_definition", args, context.session_id, context.call_id)
    }

    #[tool(
        tags(query, lsp, filesystem, read_only),
        summary = "Find symbol references.",
        translations(
            locale("zh-CN", summary = "查找符号引用。"),
            locale("zh-TW", summary = "尋找符號參照。"),
            locale("ja-JP", summary = "シンボルの参照箇所を検索します。"),
            locale("ko-KR", summary = "심볼 참조를 찾습니다."),
            locale("fr-FR", summary = "Trouver les références à un symbole."),
            locale("de-DE", summary = "Verweise auf ein Symbol finden."),
            locale("es-ES", summary = "Busca referencias a un símbolo."),
            locale("hi-IN", summary = "सिंबल के संदर्भ खोजें।"),
            locale("ar-SA", summary = "اعثر على مراجع الرمز."),
            locale("pt-BR", summary = "Encontre referências a um símbolo.")
        )
    )]
    async fn dispatch_references(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspReferencesToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_references", args, context.session_id, context.call_id)
    }

    #[tool(
        tags(query, lsp, filesystem, read_only),
        summary = "Fetch hover text.",
        translations(
            locale("zh-CN", summary = "获取符号悬停说明。"),
            locale("zh-TW", summary = "取得符號懸停說明。"),
            locale("ja-JP", summary = "シンボルのホバー情報を取得します。"),
            locale("ko-KR", summary = "심볼의 호버 정보를 가져옵니다."),
            locale(
                "fr-FR",
                summary = "Récupérer les informations au survol d’un symbole."
            ),
            locale("de-DE", summary = "Hover-Informationen zu einem Symbol abrufen."),
            locale("es-ES", summary = "Obtiene la información emergente de un símbolo."),
            locale("hi-IN", summary = "सिंबल की होवर जानकारी प्राप्त करें।"),
            locale("ar-SA", summary = "اجلب معلومات الرمز عند تمرير المؤشر."),
            locale("pt-BR", summary = "Obtenha as informações de hover de um símbolo.")
        )
    )]
    async fn dispatch_hover(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspHoverToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_hover", args, context.session_id, context.call_id)
    }

    #[tool(
        tags(query, lsp, filesystem, read_only),
        summary = "Fetch file diagnostics.",
        translations(
            locale("zh-CN", summary = "获取文件诊断信息。"),
            locale("zh-TW", summary = "取得檔案診斷資訊。"),
            locale("ja-JP", summary = "ファイルの診断情報を取得します。"),
            locale("ko-KR", summary = "파일 진단 정보를 가져옵니다."),
            locale("fr-FR", summary = "Récupérer les diagnostics d’un fichier."),
            locale("de-DE", summary = "Dateidiagnosen abrufen."),
            locale("es-ES", summary = "Obtiene los diagnósticos de un archivo."),
            locale("hi-IN", summary = "फ़ाइल के डायग्नोस्टिक्स प्राप्त करें।"),
            locale("ar-SA", summary = "اجلب تشخيصات الملف."),
            locale("pt-BR", summary = "Obtenha os diagnósticos de um arquivo.")
        )
    )]
    async fn dispatch_diagnostics(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspDiagnosticsToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_diagnostics", args, context.session_id, context.call_id)
    }
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::{
        MAX_JSON_ESCAPE_BYTES, MAX_JSON_ESCAPE_DEPTH, Plugin, SettingsNode, SettingsNodeKind,
    };

    use super::LspPlugin;

    fn node_at<'a>(node: &'a SettingsNode, path: &str) -> Option<&'a SettingsNode> {
        if node.path == path {
            return Some(node);
        }
        match &node.kind {
            SettingsNodeKind::Object { fields } => {
                fields.iter().find_map(|field| node_at(field, path))
            }
            SettingsNodeKind::List { item } => node_at(item, path),
            SettingsNodeKind::Record { value } => node_at(value, path),
            SettingsNodeKind::TaggedVariant { variants, .. } => variants
                .iter()
                .flat_map(|variant| variant.fields.iter())
                .find_map(|field| node_at(field, path)),
            _ => None,
        }
    }

    #[test]
    fn manifest_uses_typed_settings_and_bounded_json_initialization_options() {
        let manifest = LspPlugin::new().manifest();
        let settings = manifest.settings.expect("typed LSP settings contract");
        settings.validate().expect("valid LSP settings contract");
        assert_eq!(settings.root.title, "LSP Plugin Config");
        for path in [
            "/defaults/initialization_options",
            "/servers/*/session/initialization_options",
        ] {
            let node = node_at(&settings.root, path).expect("LSP initialization options node");
            assert!(matches!(
                node.kind,
                SettingsNodeKind::Json {
                    max_bytes: MAX_JSON_ESCAPE_BYTES,
                    max_depth: MAX_JSON_ESCAPE_DEPTH,
                }
            ));
        }
    }
}
