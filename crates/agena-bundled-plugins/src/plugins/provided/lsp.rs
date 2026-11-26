//! Model tools for language server lifecycle, navigation, and diagnostics.

use std::sync::{Arc, RwLock};

use crate::LspConfig;
use serde::Serialize;

use crate::part::{
    LspDefinitionToolInput, LspDiagnosticsManyToolInput, LspDiagnosticsToolInput,
    LspHoverToolInput, LspPositionToolInput, LspReferencesToolInput, LspStartToolInput,
    LspStartToolOutput, LspStopToolInput, LspStopToolOutput, LspSymbolsToolInput,
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
            "Custom servers or replacements for named built-ins.",
        ),
        (
            "/servers/*",
            "Server",
            "A single named language server definition.",
        ),
        (
            "/servers/*/disabled",
            "Disabled",
            "Remove the named server from routing.",
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
            "/servers/*/routing/language_ids",
            "Language IDs",
            "Extension-to-language ID overrides for synchronization.",
        ),
        (
            "/servers/*/routing/prefer_outermost_root",
            "Prefer Outermost Root",
            "Prefer the outermost marker within the workspace when several ancestors match.",
        ),
        (
            "/servers/*/session/configuration",
            "Workspace Configuration",
            "Settings returned to the server when it requests workspace configuration.",
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
    display_name = "Language Servers",
    icon = crate::plugin_metadata::icon(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/plugin-icons/lsp.svg"))),
    authors = ["canxin121"],
    help = "Language servers start on explicit queries or lsp.start. Use symbols to explore a file, definition and references for precise navigation, and diagnostics_many to check several files. Use lsp.stop to release this session's instances early. Idle instances stop automatically.",
    tags(agena_plugin_host::sdk::ToolTag::Lsp, agena_plugin_host::sdk::ToolTag::Discovery),
    summary = "Manage language servers and query code through LSP.",
    translations(
        locale("zh-CN", summary = "启停语言服务器并通过 LSP 导航代码。", help = "显式查询或调用 lsp.start 时启动服务。用 lsp.stop 回收当前会话的进程。空闲实例会自动关闭。"),
        locale("zh-TW", summary = "管理語言伺服器，並透過 LSP 瀏覽程式碼。"),
        locale("ja-JP", summary = "言語サーバーを管理し、LSP でコードをナビゲートします。"),
        locale("ko-KR", summary = "언어 서버를 관리하고 LSP로 코드를 탐색합니다."),
        locale("fr-FR", summary = "Gérer les serveurs de langage et naviguer dans le code via LSP."),
        locale("de-DE", summary = "Sprachserver verwalten und per LSP im Code navigieren."),
        locale("es-ES", summary = "Gestiona los servidores de lenguaje y navega por el código mediante LSP."),
        locale("hi-IN", summary = "भाषा सर्वर प्रबंधित करें और LSP के माध्यम से कोड में नेविगेट करें।"),
        locale("ar-SA", summary = "أدر خوادم اللغة وتنقّل في الشيفرة عبر LSP."),
        locale("pt-BR", summary = "Gerencie servidores de linguagem e navegue pelo código via LSP.")
    ),
    settings = LspConfig,
    settings_default = default,
    settings_metadata = lsp_settings_metadata(),
)]
impl LspPlugin {
    #[hook(
        init,
        summary = "Initialize language services",
        help = "Connects the host APIs used to inspect language server diagnostics.",
        translations(
            locale(
                "zh-CN",
                summary = "初始化语言服务",
                help = "准备语言服务功能并连接所需的 Host 服务。"
            ),
            locale(
                "zh-TW",
                summary = "初始化語言服務",
                help = "準備語言服務功能並連接所需的 Host 服務。"
            ),
            locale(
                "fr-FR",
                summary = "Initialiser les services de langage",
                help = "Prépare les services de langage et connecte les services hôtes nécessaires."
            ),
            locale(
                "es-ES",
                summary = "Inicializar los servicios de lenguaje",
                help = "Prepara los servicios de lenguaje y conecta los servicios del host necesarios."
            ),
            locale(
                "pt-BR",
                summary = "Inicializar os serviços de linguagem",
                help = "Prepara os serviços de linguagem e conecta os serviços necessários do host."
            ),
            locale(
                "de-DE",
                summary = "Die Sprachdienste initialisieren",
                help = "Bereitet die Sprachdienste vor und verbindet die erforderlichen Host-Dienste."
            ),
            locale(
                "ja-JP",
                summary = "言語サービスを初期化",
                help = "必要なホストサービスに接続し、言語サービスを利用できるようにします。"
            ),
            locale(
                "ko-KR",
                summary = "언어 서비스 초기화",
                help = "필요한 호스트 서비스에 연결하고 언어 서비스 기능을 준비합니다."
            ),
            locale(
                "hi-IN",
                summary = "भाषा सेवाएँ प्रारंभ करें",
                help = "“भाषा सेवाएँ” के लिए ज़रूरी होस्ट सेवाओं से जुड़कर संबंधित कार्यक्षमता तैयार करता है।"
            ),
            locale(
                "ar-SA",
                summary = "تهيئة خدمات اللغة",
                help = "يُعِدّ خدمات اللغة ويربط خدمات المضيف اللازمة."
            )
        )
    )]
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
        help = "Lists built-in and custom routes. Checks command availability from the workspace root. command_available=null means lookup was inconclusive. running_roots lists live instances in the current session. Specific extension routes take priority over catch-all routes. Custom routes take priority over equally specific built-in routes.",
        translations(
            locale(
                "zh-CN",
                summary = "列出已配置的语言服务器及其可执行文件是否可用。",
                help = "列出内置和自定义路由，并从工作区根目录检查命令可用性。`command_available=null` 表示无法确定。`running_roots` 列出当前会话中正在运行的实例。扩展名专用路由优先于通配路由，同类路由中自定义配置优先。"
            ),
            locale(
                "zh-TW",
                summary = "列出已設定的語言伺服器及其執行檔可用狀態。",
                help = "從工作區根目錄依伺服器設定的 PATH 檢查命令，不會執行命令；`command_available=null` 表示無法判定是否存在。命令存在不代表伺服器一定能初始化。相對命令在不同專案根目錄可能解析到不同位置。`running_roots` 列出已初始化的執行個體。"
            ),
            locale(
                "ja-JP",
                summary = "設定済み言語サーバーと実行ファイルの有無を一覧表示します。",
                help = "ワークスペースのルートから、設定された PATH に基づいて各コマンドを確認します。実行はしません。`command_available=null` は判定できなかったことを示し、存在しても初期化成功を保証しません。相対コマンドの解決先はプロジェクトルートごとに異なる場合があります。`running_roots` は初期化済みのインスタンスです。"
            ),
            locale(
                "ko-KR",
                summary = "구성된 언어 서버와 실행 파일 사용 가능 여부를 나열합니다.",
                help = "작업 공간 루트에서 설정된 PATH를 기준으로 명령을 확인하며 실행하지는 않습니다. `command_available=null`은 확인할 수 없다는 뜻입니다. 명령이 존재해도 서버 초기화가 성공한다는 보장은 없습니다. 상대 경로 명령은 프로젝트 루트마다 다르게 해석될 수 있습니다. `running_roots`는 초기화된 인스턴스를 나타냅니다."
            ),
            locale(
                "fr-FR",
                summary = "Lister les serveurs de langage configurés et la disponibilité de leurs exécutables.",
                help = "Depuis la racine de l’espace de travail, vérifie chaque commande avec le PATH configuré sans l’exécuter. `command_available=null` signifie que la vérification n’a pas pu conclure ; la présence ne garantit pas l’initialisation. Les commandes relatives peuvent varier selon la racine du projet. `running_roots` répertorie les instances initialisées."
            ),
            locale(
                "de-DE",
                summary = "Konfigurierte Sprachserver und verfügbare Programme auflisten.",
                help = "Prüft jede Konfiguration vom Workspace-Stamm aus gegen den festgelegten PATH, ohne den Befehl auszuführen. `command_available=null` bedeutet, dass die Prüfung unklar blieb; ein vorhandener Befehl garantiert keine erfolgreiche Initialisierung. Relative Befehle können je nach Projektstamm anders aufgelöst werden. `running_roots` enthält initialisierte Instanzen."
            ),
            locale(
                "es-ES",
                summary = "Enumera los servidores de lenguaje configurados y si sus ejecutables están disponibles.",
                help = "Comprueba la disponibilidad de cada comando desde la raíz del espacio de trabajo usando el PATH configurado. `command_available=null` indica que la comprobación no fue concluyente. `running_roots` enumera las instancias activas de la sesión actual."
            ),
            locale(
                "hi-IN",
                summary = "कॉन्फ़िगर किए गए भाषा सर्वर और उनके executable की उपलब्धता दिखाएँ।",
                help = "कार्यस्थान की मूल डायरेक्टरी से कॉन्फ़िगर किए गए PATH के अनुसार कमांड की उपलब्धता जाँचता है। `command_available=null` का अर्थ है कि उपलब्धता तय नहीं हो सकी। `running_roots` मौजूदा सत्र में चल रहे सर्वर इंस्टेंस दिखाता है।"
            ),
            locale(
                "ar-SA",
                summary = "اعرض خوادم اللغة المُهيأة ومدى توفر ملفاتها التنفيذية.",
                help = "يتحقق من توفر كل أمر انطلاقًا من جذر مساحة العمل ووفق PATH المُهيأ. تعني `command_available=null` تعذّر تحديد التوفر. تعرض `running_roots` مثيلات الخوادم النشطة في الجلسة الحالية."
            ),
            locale(
                "pt-BR",
                summary = "Liste os servidores de linguagem configurados e a disponibilidade dos executáveis.",
                help = "Verifica a disponibilidade de cada comando a partir da raiz do espaço de trabalho usando o PATH configurado. `command_available=null` indica que a verificação foi inconclusiva. `running_roots` lista as instâncias ativas na sessão atual."
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
        tags(execute, lsp, filesystem, mutate),
        output(LspStartToolOutput),
        summary = "Start the language server selected by a file.",
        help = "Use file_path to select a language and project root. Call this to warm up the server before editing. Explicit LSP queries can also start it on demand. Reuse across calls requires persistent read access to the project root. Idle instances stop automatically. The result identifies the route and root, for example {\"server\":\"rust-analyzer\",\"root\":\".\"}.",
        translations(locale(
            "zh-CN",
            summary = "按文件启动语言服务器。",
            help = "根据 file_path 选择语言和项目根。显式查询也可按需启动服务。需要持续读取项目根的权限，才能跨调用保留进程。"
        ))
    )]
    async fn dispatch_start(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspStartToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_start", args, context.session_id, context.call_id)
    }

    #[tool(
        tags(lsp, mutate),
        output(LspStopToolOutput),
        summary = "Stop language servers in the current session.",
        help = "Omit server to stop all cached instances in the current session. Pass a server name from lsp.servers to stop its instances across this session's project roots. A later explicit start or query can open it again. The result counts running instances stopped, for example {\"server\":\"rust-analyzer\",\"stopped_instances\":1}.",
        translations(locale(
            "zh-CN",
            summary = "停止当前会话的语言服务器。",
            help = "省略 server 会停止当前会话的所有实例。传入 lsp.servers 中的名称可停止指定服务器。以后可显式启动或查询以重新打开。"
        ))
    )]
    async fn dispatch_stop(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspStopToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_stop", args, context.session_id, context.call_id)
    }

    #[tool(
        tags(query, lsp, filesystem, read_only),
        summary = "Resolve symbol definitions.",
        help = "Use the current source position in zero-based UTF-16. Pass expected_revision from a previous result to reject changed source. Use targets[].selection_range or targets[].range for follow-up queries.",
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
        help = "Use a zero-based UTF-16 source position. The result includes target ranges and indicates when some locations were omitted.",
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
        help = "Only current diagnostics confirm a clean synchronized version. A stale, unversioned, disconnected or timed-out result can be rechecked with lsp.diagnostics or the project check command.",
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

    #[tool(
        tags(query, lsp, filesystem, read_only),
        summary = "Find implementations of a symbol.",
        help = "Use the zero-based UTF-16 position of an interface, trait, or member. Results contain exact ranges when the server supports implementation queries.",
        translations(locale("zh-CN", summary = "查找接口、trait 或成员的实现。"))
    )]
    async fn dispatch_implementation(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspPositionToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool(
            "lsp_implementation",
            args,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(query, lsp, filesystem, read_only),
        summary = "List file symbols or search workspace symbols.",
        help = "Pass file_path to list functions, types and other symbols in a document. Also pass query to search the workspace of the language server selected by that file. Result ranges use zero-based UTF-16.",
        translations(locale("zh-CN", summary = "列出文件符号，或按名称搜索工作区符号。"))
    )]
    async fn dispatch_symbols(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspSymbolsToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_symbols", args, context.session_id, context.call_id)
    }

    #[tool(
        tags(query, lsp, filesystem, read_only),
        summary = "Check diagnostics for several files.",
        help = "Pass one to sixteen file_paths. Each file reports its version, diagnostic count, and state under a shared deadline.",
        translations(locale("zh-CN", summary = "一次检查多个文件的语言服务诊断。"))
    )]
    async fn dispatch_diagnostics_many(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspDiagnosticsManyToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool(
            "lsp_diagnostics_many",
            args,
            context.session_id,
            context.call_id,
        )
    }
}
