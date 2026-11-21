mod browser_tools;
mod downloads;
mod managed_preview;
mod sessions;
mod uploads;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_domain::{
    BackgroundActivity, BackgroundActivityKind, BackgroundActivityLogLine,
    BackgroundActivityLogRead, BackgroundActivityStatus,
};
use agena_plugin_host::sdk::attachment::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_plugin_host::sdk::{
    ActivitySourceAdapter, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, async_trait,
};
use agena_plugin_host::{PluginError, sdk::host_api::HostClient};
use agena_web::{managed_preview::ManagedPreviewTarget, prepare_fetch_url};
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};

use super::driver::BrowserDriver;
use super::runtime::{ChromeOptions, ChromeRuntime, ChromeSandbox};
use crate::network_policy;
use browser_tools::{
    BrowserContentInput, BrowserDiagnosticsInput, BrowserEvaluateInput, BrowserInspectInput,
};
use sessions::{
    BrowserActionInput, BrowserSessions, BrowserSnapshotInput, default_return_snapshot,
};
use uploads::BrowserUploadInput;

pub(crate) const BROWSER_PLUGIN_ID: &str = "agena.browser";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct BrowserConfig {
    /// Executable path for Chrome or Chromium. Auto-detect when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    executable_path: Option<String>,
    /// Maximum time to wait for Chrome to start.
    #[schemars(range(min = 1, max = 120))]
    startup_timeout_secs: u64,
    /// Inactivity before Chrome closes. Zero disables idle shutdown.
    idle_timeout_secs: u64,
    /// Include page messages in console diagnostics and activity logs.
    capture_console_text: bool,
    /// Additional synthetic DNS ranges owned by the local TUN proxy.
    fake_ip_ranges: Vec<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            executable_path: None,
            startup_timeout_secs: 10,
            idle_timeout_secs: 300,
            capture_console_text: false,
            fake_ip_ranges: Vec::new(),
        }
    }
}

impl BrowserConfig {
    fn chrome_options(&self) -> ChromeOptions {
        ChromeOptions::new(
            self.executable_path.as_deref(),
            self.startup_timeout_secs,
            self.idle_timeout_secs,
            ChromeSandbox::Required,
        )
    }

    fn parse(value: serde_json::Value) -> SdkResult<Self> {
        let config: Self = if value.is_null() {
            Self::default()
        } else {
            serde_json::from_value(value).map_err(|error| {
                plugin_internal_error_with_context("invalid browser plugin settings", &error)
            })?
        };
        config.chrome_options().validate()?;
        network_policy::validate_fake_ip_ranges(&config.fake_ip_ranges)?;
        Ok(config)
    }
}

fn browser_settings_metadata() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        ("", "Browser", "Settings for interactive Chrome pages."),
        (
            "/executable_path",
            "Executable Path",
            "Path to Chrome or Chromium. Auto-detect when omitted.",
        ),
        (
            "/startup_timeout_secs",
            "Startup Timeout (sec)",
            "Maximum time to wait for Chrome to start.",
        ),
        (
            "/idle_timeout_secs",
            "Idle Timeout (sec)",
            "Seconds of inactivity before Chrome closes. Zero disables idle shutdown.",
        ),
        (
            "/capture_console_text",
            "Capture Console Text",
            "Include page messages in diagnostics and activity logs.",
        ),
        (
            "/fake_ip_ranges",
            "Synthetic DNS Ranges",
            "Additional Fake-IP ranges owned by the local TUN proxy.",
        ),
    ]
}

pub(crate) struct BrowserPlugin {
    state: OnceLock<BrowserPluginState>,
    browser_state: Arc<BrowserSessions>,
}

/// Managed Chrome sessions shared by successive hosts with the same Browser
/// configuration. Tool permissions still come from each call's own host.
#[derive(Clone, Default)]
pub struct BrowserSessionPool(Arc<BrowserSessions>);

impl BrowserSessionPool {
    pub fn new(activity_host: Arc<dyn HostClient>) -> Self {
        Self(Arc::new(BrowserSessions::with_activity_host(activity_host)))
    }

    pub(crate) fn plugin(&self) -> BrowserPlugin {
        BrowserPlugin {
            state: OnceLock::new(),
            browser_state: self.0.clone(),
        }
    }
}

struct BrowserPluginState {
    config: BrowserConfig,
    workspace_root: PathBuf,
    host: Arc<dyn HostClient>,
    generation_lease: std::sync::Mutex<Option<Arc<()>>>,
}

impl Default for BrowserPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "browser",
    version = env!("CARGO_PKG_VERSION"),
    display_name = "Browser",
    icon = crate::plugin_metadata::icon(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/plugin-icons/inline_ui.svg"))),
    authors = ["canxin121"],
    summary = "Interact with managed Chrome pages and workspace previews.",
    help = "Start with browser.list to discover owned pages and previews, or browser.open for an HTTP(S) URL. Use browser.snapshot for controls, browser.content or browser.inspect for page data, and browser.diagnostics to investigate failures. Pass the browser session_id from browser.open or browser.list. When the page requires human verification, sign-in, or another step the user must perform, request help with browser.request_takeover and continue after handback.",
    tags(agena_plugin_host::sdk::ToolTag::Network, agena_plugin_host::sdk::ToolTag::Interactive),
    translations(locale(
        "zh-CN",
        summary = "操作受管 Chrome 页面和工作区预览。",
        help = "用 browser.list 查找当前会话的页面和预览，或用 browser.open 打开网页。用页面的 session_id 继续操作。browser.snapshot 查看控件，browser.content 读取页面内容，browser.evaluate 执行 JavaScript，browser.diagnostics 查看网络请求和异常。如果页面需要人工验证、登录或其他必须由用户完成的操作，调用 browser.request_takeover 请求接管，等用户交还浏览器后继续。"
    )),
    settings = BrowserConfig,
    settings_default = BrowserConfig::default(),
    settings_metadata = browser_settings_metadata(),
)]
impl BrowserPlugin {
    pub(crate) fn new() -> Self {
        BrowserSessionPool::default().plugin()
    }

    #[hook(
        init,
        summary = "Initialize interactive browser tools",
        help = "Load Chrome settings and bind the browser tools to the workspace host.",
        translations(
            locale(
                "zh-CN",
                summary = "初始化浏览器工具",
                help = "读取 Chrome 设置并连接工作区宿主服务。"
            ),
            locale(
                "zh-TW",
                summary = "初始化瀏覽器工具",
                help = "讀取 Chrome 設定並連接工作區主機服務。"
            ),
            locale(
                "fr-FR",
                summary = "Initialiser les outils du navigateur",
                help = "Charge les paramètres de Chrome et relie les outils au service hôte de l’espace de travail."
            ),
            locale(
                "es-ES",
                summary = "Inicializar las herramientas del navegador",
                help = "Carga la configuración de Chrome y conecta las herramientas al servicio del espacio de trabajo."
            ),
            locale(
                "pt-BR",
                summary = "Inicializar as ferramentas do navegador",
                help = "Carrega as configurações do Chrome e conecta as ferramentas ao serviço do espaço de trabalho."
            ),
            locale(
                "de-DE",
                summary = "Browserwerkzeuge initialisieren",
                help = "Lädt die Chrome-Einstellungen und verbindet die Werkzeuge mit dem Hostdienst des Arbeitsbereichs."
            ),
            locale(
                "ja-JP",
                summary = "ブラウザーツールを初期化",
                help = "Chrome の設定を読み込み、ツールをワークスペースのホストサービスに接続します。"
            ),
            locale(
                "ko-KR",
                summary = "브라우저 도구 초기화",
                help = "Chrome 설정을 읽고 도구를 작업 공간의 호스트 서비스에 연결합니다."
            ),
            locale(
                "hi-IN",
                summary = "ब्राउज़र टूल शुरू करें",
                help = "Chrome की सेटिंग लोड करके टूल को कार्यक्षेत्र की होस्ट सेवा से जोड़ता है।"
            ),
            locale(
                "ar-SA",
                summary = "تهيئة أدوات المتصفح",
                help = "يحمّل إعدادات Chrome ويربط الأدوات بخدمة المضيف في مساحة العمل."
            )
        )
    )]
    async fn init(
        &self,
        ctx: agena_plugin_host::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        let state = BrowserPluginState {
            config: BrowserConfig::parse(ctx.settings)?,
            workspace_root: ctx.workspace_root,
            host,
            generation_lease: std::sync::Mutex::new(Some(self.browser_state.attach_plugin())),
        };
        self.state
            .set(state)
            .map_err(|_| PluginError::internal("browser plugin initialized more than once"))?;
        let mut manifest = agena_plugin_host::sdk::Plugin::manifest(self);
        crate::plugin_tool_docs::localize_builtin_tool_docs(&mut manifest);
        Ok(agena_plugin_host::sdk::InitOutcome::ack(manifest))
    }

    fn state(&self) -> SdkResult<&BrowserPluginState> {
        self.state
            .get()
            .ok_or_else(|| PluginError::internal("browser plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&BrowserConfig> {
        Ok(&self.state()?.config)
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        Ok(&self.state()?.workspace_root)
    }

    #[hook(
        shutdown,
        summary = "Close browser resources",
        help = "Closes managed Chrome pages and the interactive browser process.",
        translations(
            locale(
                "zh-CN",
                summary = "关闭浏览器资源",
                help = "插件停止时关闭受管 Chrome 页面和交互式浏览器进程。"
            ),
            locale(
                "zh-TW",
                summary = "關閉瀏覽器資源",
                help = "外掛停止時關閉受管 Chrome 頁面及互動式瀏覽器程序。"
            ),
            locale(
                "fr-FR",
                summary = "Fermer les ressources du navigateur",
                help = "Ferme les pages Chrome gérées et le processus du navigateur interactif à l’arrêt du plugin."
            ),
            locale(
                "es-ES",
                summary = "Cerrar los recursos del navegador",
                help = "Cierra las páginas gestionadas de Chrome y el proceso del navegador interactivo al detener el plugin."
            ),
            locale(
                "pt-BR",
                summary = "Fechar recursos do navegador",
                help = "Fecha as páginas gerenciadas do Chrome e o processo do navegador interativo ao encerrar o plugin."
            ),
            locale(
                "de-DE",
                summary = "Browserressourcen schließen",
                help = "Schließt beim Beenden des Plugins die verwalteten Chrome-Seiten und den interaktiven Browserprozess."
            ),
            locale(
                "ja-JP",
                summary = "ブラウザーのリソースを終了",
                help = "プラグイン停止時に管理対象の Chrome ページと対話型ブラウザープロセスを終了します。"
            ),
            locale(
                "ko-KR",
                summary = "브라우저 리소스 종료",
                help = "플러그인을 중지할 때 관리되는 Chrome 페이지와 대화형 브라우저 프로세스를 종료합니다."
            ),
            locale(
                "hi-IN",
                summary = "ब्राउज़र संसाधन बंद करें",
                help = "प्लगइन बंद होने पर प्रबंधित Chrome पेज और इंटरैक्टिव ब्राउज़र प्रक्रिया बंद करता है।"
            ),
            locale(
                "ar-SA",
                summary = "إغلاق موارد المتصفح",
                help = "يغلق صفحات Chrome المُدارة وعملية المتصفح التفاعلي عند إيقاف الإضافة."
            )
        )
    )]
    async fn shutdown(&self) -> SdkResult<()> {
        if let Some(state) = self.state.get() {
            let lease = state
                .generation_lease
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            if lease.and_then(Arc::into_inner).is_some() {
                return self
                    .browser_state
                    .shutdown(self.browser_state.activity_host(&state.host).as_ref())
                    .await;
            }
        }
        Ok(())
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "open",
        summary = "Open a public page or an owned running preview in a managed interactive browser session.",
        help = "Supply exactly one of url or preview_id. browser.list discovers owned preview IDs. preview_id grants only the registered run and exact isolated origin and still requires network permission for the real upstream URL; stop/restart revokes it. Managed previews restrict external fetch/WebSocket connections, frames, workers and popups; blocked requests are a browser policy result, not evidence the application is broken.",
        translations(
            locale("ar-SA", summary = "افتح صفحة في جلسة متصفح تفاعلية مُدارة."),
            locale(
                "de-DE",
                summary = "Öffnet eine Seite in einer verwalteten interaktiven Browser-Sitzung."
            ),
            locale(
                "es-ES",
                summary = "Abre una página en una sesión de navegador interactiva gestionada."
            ),
            locale(
                "fr-FR",
                summary = "Ouvre une page dans une session de navigateur interactive gérée."
            ),
            locale("hi-IN", summary = "प्रबंधित इंटरैक्टिव ब्राउज़र सत्र में पेज खोलता है।"),
            locale(
                "ja-JP",
                summary = "管理された対話型ブラウザーセッションでページを開きます。"
            ),
            locale(
                "ko-KR",
                summary = "관리되는 대화형 브라우저 세션에서 페이지를 엽니다."
            ),
            locale(
                "pt-BR",
                summary = "Abre uma página em uma sessão de navegador interativa gerenciada."
            ),
            locale("zh-CN", summary = "在受管交互式浏览器会话中打开页面。"),
            locale("zh-TW", summary = "在受管理的互動式瀏覽器工作階段中開啟頁面。")
        )
    )]
    async fn browser_open(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserOpenInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.open_browser(context, input).await
    }

    #[tool(
        tags(interactive),
        name = "request_takeover",
        summary = "Pause the browser task so the user can operate the current page in Agena Web.",
        help = "Open the page first. Request takeover for sign-in, verification or other steps the user needs to perform. State what the user should do in reason. Ask the user to enter credentials only in the browser. When status is returned, take a fresh snapshot of the returned session_id. If browser.list reports waiting_for_user, wait for the user before doing more browser work.",
        translations(locale(
            "zh-CN",
            summary = "暂停浏览器任务，请用户在 Agena Web 接管当前页面。",
            help = "先打开页面，再说明需要用户完成的登录、验证或其他操作。请用户直接在浏览器输入凭据。返回状态为 returned 时，使用返回的 session_id 重新获取页面快照。browser.list 返回 waiting_for_user 时，等待用户交还控制权。"
        ))
    )]
    async fn browser_request_takeover(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserTakeoverInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.request_takeover(context, &input.session_id, &input.reason)
            .await
    }

    #[tool(
        tags(network, interactive, query, discovery, read_only),
        name = "list",
        summary = "List open page targets in the managed interactive browser.",
        translations(
            locale(
                "ar-SA",
                summary = "اعرض أهداف الصفحات المفتوحة في المتصفح التفاعلي المُدار."
            ),
            locale(
                "de-DE",
                summary = "Listet offene Seitenziele im verwalteten interaktiven Browser auf."
            ),
            locale(
                "es-ES",
                summary = "Lista los destinos de página abiertos en el navegador interactivo gestionado."
            ),
            locale(
                "fr-FR",
                summary = "Liste les cibles de page ouvertes dans le navigateur interactif géré."
            ),
            locale(
                "hi-IN",
                summary = "प्रबंधित इंटरैक्टिव ब्राउज़र में खुले पेज लक्ष्यों की सूची बनाता है।"
            ),
            locale(
                "ja-JP",
                summary = "管理された対話型ブラウザーで開いているページターゲットを一覧表示します。"
            ),
            locale(
                "ko-KR",
                summary = "관리되는 대화형 브라우저에서 열린 페이지 대상을 나열합니다."
            ),
            locale(
                "pt-BR",
                summary = "Lista os destinos de página abertos no navegador interativo gerenciado."
            ),
            locale("zh-CN", summary = "列出受管交互式浏览器中打开的页面目标。"),
            locale("zh-TW", summary = "列出受管理互動式瀏覽器中開啟的頁面目標。")
        )
    )]
    async fn browser_list(
        &self,
        context: &ToolInvokeContext<'_>,
        _input: &BrowserListInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.list_browsers(context).await
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "close",
        summary = "Close one page target in the managed interactive browser.",
        translations(
            locale("ar-SA", summary = "أغلق هدف صفحة واحدًا في المتصفح التفاعلي المُدار."),
            locale(
                "de-DE",
                summary = "Schließt ein Seitenziel im verwalteten interaktiven Browser."
            ),
            locale(
                "es-ES",
                summary = "Cierra un destino de página en el navegador interactivo gestionado."
            ),
            locale(
                "fr-FR",
                summary = "Ferme une cible de page dans le navigateur interactif géré."
            ),
            locale("hi-IN", summary = "प्रबंधित इंटरैक्टिव ब्राउज़र में एक पेज लक्ष्य बंद करता है।"),
            locale(
                "ja-JP",
                summary = "管理された対話型ブラウザーのページターゲットを1つ閉じます。"
            ),
            locale(
                "ko-KR",
                summary = "관리되는 대화형 브라우저에서 페이지 대상 하나를 닫습니다."
            ),
            locale(
                "pt-BR",
                summary = "Fecha um destino de página no navegador interativo gerenciado."
            ),
            locale("zh-CN", summary = "关闭受管交互式浏览器中的一个页面目标。"),
            locale("zh-TW", summary = "關閉受管理互動式瀏覽器中的一個頁面目標。")
        )
    )]
    async fn browser_close(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserSessionInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.close_browser(context, &input.session_id).await
    }
    #[tool(
        tags(network, interactive, mutate),
        name = "shutdown",
        summary = "Close browser pages owned by the current Agena session without affecting other callers.",
        help = "Caller-scoped shutdown closes owned pages only. The shared Chrome process and other sessions are not stopped. Global browser shutdown is reserved for trusted host lifecycle control.",
        translations(
            locale(
                "ar-SA",
                summary = "أغلق صفحات المتصفح التي تملكها جلسة Agena الحالية دون التأثير على المتصلين الآخرين."
            ),
            locale(
                "de-DE",
                summary = "Schließt Browserseiten der aktuellen Agena-Sitzung, ohne andere Aufrufer zu beeinflussen."
            ),
            locale(
                "es-ES",
                summary = "Cierra las páginas del navegador de la sesión actual de Agena sin afectar a otros usuarios."
            ),
            locale(
                "fr-FR",
                summary = "Ferme les pages du navigateur de la session Agena en cours sans affecter les autres appelants."
            ),
            locale(
                "hi-IN",
                summary = "वर्तमान Agena सत्र के स्वामित्व वाले ब्राउज़र पेज बंद करता है, अन्य कॉलर प्रभावित नहीं होते।"
            ),
            locale(
                "ja-JP",
                summary = "現在の Agena セッションが所有するブラウザーのページを閉じます。他の呼び出し元には影響しません。"
            ),
            locale(
                "ko-KR",
                summary = "현재 Agena 세션이 소유한 브라우저 페이지를 닫습니다. 다른 호출자에는 영향을 주지 않습니다."
            ),
            locale(
                "pt-BR",
                summary = "Fecha as páginas do navegador pertencentes à sessão atual do Agena sem afetar outros chamadores."
            ),
            locale(
                "zh-CN",
                summary = "关闭当前 Agena 会话拥有的浏览器页面，不影响其他调用方。"
            ),
            locale(
                "zh-TW",
                summary = "關閉目前 Agena 工作階段擁有的瀏覽器頁面，不影響其他呼叫方。"
            )
        )
    )]
    async fn browser_shutdown(
        &self,
        context: &ToolInvokeContext<'_>,
        _input: &BrowserListInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.close_owned_browsers(context).await
    }

    #[tool(
        tags(network, interactive, query, read_only),
        name = "snapshot",
        summary = "Read the browser accessibility tree and get element refs for browser actions.",
        help = "Read snapshot.tree for page text and controls. For a node marked [ref=e7], use @e7 as a selector in browser.click, browser.type or browser.action. If the tree is truncated, pass selector to inspect a specific CSS region, interactive to focus on controls, or max_depth for a shallower tree. Use refs from the latest snapshot in the same session.",
        translations(
            locale(
                "ar-SA",
                summary = "افحص النص المرئي والعناصر التفاعلية في جلسة متصفح."
            ),
            locale(
                "de-DE",
                summary = "Prüft sichtbaren Text und interaktive Elemente in einer Browser-Sitzung."
            ),
            locale(
                "es-ES",
                summary = "Inspecciona el texto visible y los elementos interactivos de una sesión de navegador."
            ),
            locale(
                "fr-FR",
                summary = "Inspecte le texte visible et les éléments interactifs d’une session de navigateur."
            ),
            locale(
                "hi-IN",
                summary = "ब्राउज़र सत्र में दिखाई देने वाला पाठ और इंटरैक्टिव तत्व देखता है।"
            ),
            locale(
                "ja-JP",
                summary = "ブラウザーセッションの表示テキストと操作可能な要素を確認します。"
            ),
            locale(
                "ko-KR",
                summary = "브라우저 세션에서 표시되는 텍스트와 상호작용 요소를 확인합니다."
            ),
            locale(
                "pt-BR",
                summary = "Inspeciona o texto visível e os elementos interativos de uma sessão de navegador."
            ),
            locale(
                "zh-CN",
                summary = "读取浏览器无障碍树，获取可用于后续操作的元素引用。"
            ),
            locale(
                "zh-TW",
                summary = "讀取瀏覽器無障礙樹，取得可用於後續操作的元素參照。"
            )
        )
    )]
    async fn browser_snapshot(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserSnapshotInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.inspect_browser(context, input).await
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "evaluate",
        summary = "Execute JavaScript in an owned browser page or frame and return a bounded structured result.",
        help = "Pass an expression such as document.title, an immediately invoked function, or a Promise. Use browser.frames for frame_id. The script runs in the page and can change it. Use browser.content or browser.inspect for standard reads. Check status for exceptions, timeout and context changes. Inspect page state before retrying a script with side effects.",
        translations(locale(
            "zh-CN",
            summary = "在受管页面或 frame 中执行 JavaScript 并返回有大小限制的结构化结果。"
        ))
    )]
    async fn browser_evaluate(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserEvaluateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.evaluate_in_browser(context, input).await
    }

    #[tool(
        tags(network, interactive, query, read_only),
        name = "content",
        summary = "Read visible text, current DOM HTML, or the captured main-document HTTP response.",
        help = "Use text or dom_html with an optional selector and frame_id to read the current page. Use response_html for the main document's captured HTTP response. Continue the same immutable capture using session_id, content_id and next_offset as offset. Check truncated for the capture size limit and status for an unavailable response.",
        translations(locale(
            "zh-CN",
            summary = "读取页面可见文本、当前 DOM HTML 或捕获的主文档 HTTP 响应。"
        ))
    )]
    async fn browser_content(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserContentInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.read_browser_content(context, input).await
    }

    #[tool(
        tags(network, interactive, query, read_only),
        name = "inspect",
        summary = "Read element properties and match count for a CSS selector or a snapshot ref.",
        help = "Returns text, form value, visible, enabled, checked, requested attributes, and bounds of the first match. Bounds use CSS pixels relative to the element's own frame viewport. Use match_count to detect missing or ambiguous CSS matches.",
        translations(locale(
            "zh-CN",
            summary = "读取 CSS 或快照引用匹配的元素属性、状态、坐标与匹配数量。"
        ))
    )]
    async fn browser_inspect(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserInspectInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.inspect_browser_element(context, input).await
    }

    #[tool(
        tags(network, interactive, query, discovery, read_only),
        name = "frames",
        summary = "List frames in an owned browser page, including frame IDs, ancestry, and URLs.",
        help = "Pass frame_id to browser.snapshot, browser.content, browser.inspect, browser.evaluate or an element operation. Refresh the list if a frame was removed or its execution context is still loading.",
        translations(locale("zh-CN", summary = "列出受管页面的 frame ID、父级关系和 URL。"))
    )]
    async fn browser_frames(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserSessionInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.list_browser_frames(context, &input.session_id).await
    }

    #[tool(
        tags(network, interactive, query, read_only),
        name = "diagnostics",
        summary = "Read a browser page's recent network requests, selected response body, console, or exceptions.",
        help = "Use query.kind network to locate requests, request with request_id for details and an optional text body, or logs for console and exceptions. Pass next_seq as since_seq to continue. Console message text follows the capture_console_text plugin setting. Inspect body.status for unavailable and non-text responses.",
        translations(locale(
            "zh-CN",
            summary = "查看当前页面近期的网络请求、指定响应体、控制台日志或异常。"
        ))
    )]
    async fn browser_diagnostics(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserDiagnosticsInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.read_browser_diagnostics(context, input).await
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "click",
        summary = "Click a browser element by @ref from the latest snapshot or by CSS selector.",
        translations(
            locale("ar-SA", summary = "انقر عنصر متصفح محددًا بـ CSS أو مرجع أحدث لقطة."),
            locale(
                "de-DE",
                summary = "Klickt ein Browserelement an, das per CSS oder neuester Snapshot-Referenz gewählt wurde."
            ),
            locale(
                "es-ES",
                summary = "Hace clic en un elemento del navegador seleccionado por CSS o la referencia de la última instantánea."
            ),
            locale(
                "fr-FR",
                summary = "Clique sur un élément du navigateur sélectionné par CSS ou la référence du dernier instantané."
            ),
            locale(
                "hi-IN",
                summary = "CSS या नवीनतम स्नैपशॉट संदर्भ से चुने गए ब्राउज़र तत्व पर क्लिक करता है।"
            ),
            locale(
                "ja-JP",
                summary = "CSS または最新スナップショットの参照で選んだブラウザー要素をクリックします。"
            ),
            locale(
                "ko-KR",
                summary = "CSS 또는 최신 스냅샷 참조로 선택한 브라우저 요소를 클릭합니다."
            ),
            locale(
                "pt-BR",
                summary = "Clica em um elemento do navegador selecionado por CSS ou pela referência do instantâneo mais recente."
            ),
            locale("zh-CN", summary = "点击由 CSS 或最新快照引用选定的浏览器元素。"),
            locale("zh-TW", summary = "點擊由 CSS 或最新快照參照選定的瀏覽器元素。")
        )
    )]
    async fn browser_click(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserClickInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let access = self.browser_access(context, &input.session_id).await?;
        self.run_browser_command(
            &access,
            "click",
            serde_json::json!({
                "action": "click", "selector": input.selector, "frameId": input.frame_id,
            }),
            input.timeout_ms,
            input.return_snapshot,
        )
        .await
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "type",
        summary = "Fill an editable browser element by @ref or CSS selector, optionally pressing Enter.",
        help = "After submitting with press_enter, inspect the resulting page. Use browser.snapshot when return_snapshot is false. When the page requires verification or sign-in, read tools_help for browser.request_takeover and invoke it for this page before ending the task.",
        translations(
            locale(
                "ar-SA",
                summary = "املأ حقل إدخال في المتصفح محددًا بـ CSS أو مرجع أحدث لقطة، مع إمكانية الضغط على Enter."
            ),
            locale(
                "de-DE",
                summary = "Füllt ein Browser-Eingabefeld, das per CSS oder neuester Snapshot-Referenz gewählt wurde, optional mit Enter."
            ),
            locale(
                "es-ES",
                summary = "Rellena un campo del navegador seleccionado por CSS o la referencia de la última instantánea, opcionalmente pulsando Enter."
            ),
            locale(
                "fr-FR",
                summary = "Remplit un champ du navigateur sélectionné par CSS ou la référence du dernier instantané, avec Entrée en option."
            ),
            locale(
                "hi-IN",
                summary = "CSS या नवीनतम स्नैपशॉट संदर्भ से चुने गए ब्राउज़र इनपुट को भरता है, वैकल्पिक रूप से Enter दबाता है।"
            ),
            locale(
                "ja-JP",
                summary = "CSS または最新スナップショットの参照で選んだブラウザーの入力欄を埋め、必要に応じて Enter を押します。"
            ),
            locale(
                "ko-KR",
                summary = "CSS 또는 최신 스냅샷 참조로 선택한 브라우저 입력란을 채우고 선택적으로 Enter를 누릅니다."
            ),
            locale(
                "pt-BR",
                summary = "Preenche um campo do navegador selecionado por CSS ou pela referência do instantâneo mais recente, opcionalmente pressionando Enter."
            ),
            locale(
                "zh-CN",
                summary = "填写由 CSS 或最新快照引用选定的浏览器输入框，可选择按回车。",
                help = "按回车提交后查看页面结果。return_snapshot 为 false 时可调用 browser.snapshot 查看。遇到需要用户完成的验证或登录，先用 tools_help 查看 browser.request_takeover，再为当前页面发起接管请求。"
            ),
            locale(
                "zh-TW",
                summary = "填寫由 CSS 或最新快照參照選定的瀏覽器輸入框，可選擇按下 Enter。"
            )
        )
    )]
    async fn browser_type(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserTypeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let access = self.browser_access(context, &input.session_id).await?;
        self.run_browser_command(&access, "fill", serde_json::json!({
            "action": "fill", "selector": input.selector, "value": input.text, "pressEnter": input.press_enter,
            "frameId": input.frame_id,
        }), input.timeout_ms, input.return_snapshot).await
    }

    #[tool(
        tags(network, interactive, query, read_only),
        name = "wait",
        summary = "Wait for an element state, visible text, a URL pattern, or a page load condition.",
        help = "Supply selector, text, url, or load_state. With selector, use state visible, hidden, attached, or detached. The default wait is domcontentloaded. A frame_id scopes the condition to that frame, and networkidle applies to the whole page.",
        translations(
            locale("ar-SA", summary = "انتظر جاهزية الصفحة أو محدد CSS أو نصًا مرئيًا."),
            locale(
                "de-DE",
                summary = "Wartet auf Seitenbereitschaft, einen CSS-Selektor oder sichtbaren Text."
            ),
            locale(
                "es-ES",
                summary = "Espera a que la página esté lista, a un selector CSS o a un texto visible."
            ),
            locale(
                "fr-FR",
                summary = "Attend que la page soit prête, un sélecteur CSS ou un texte visible."
            ),
            locale(
                "hi-IN",
                summary = "पेज तैयार होने, CSS चयनकर्ता या दिखाई देने वाले पाठ की प्रतीक्षा करता है।"
            ),
            locale(
                "ja-JP",
                summary = "ページの準備完了、CSS セレクター、または表示テキストを待ちます。"
            ),
            locale(
                "ko-KR",
                summary = "페이지 준비, CSS 선택자 또는 표시 텍스트를 기다립니다."
            ),
            locale(
                "pt-BR",
                summary = "Aguarda a página ficar pronta, um seletor CSS ou um texto visível."
            ),
            locale("zh-CN", summary = "等待页面就绪、CSS 选择器或可见文本。"),
            locale("zh-TW", summary = "等待頁面就緒、CSS 選擇器或可見文字。")
        )
    )]
    async fn browser_wait(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserWaitInput,
    ) -> SdkResult<ToolInvokeOutput> {
        if [
            input.selector.is_some(),
            input.text.is_some(),
            input.url.is_some(),
            input.load_state.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
            > 1
            || input.state.is_some() && input.selector.is_none()
        {
            return Err(PluginError::invalid_params(
                "Supply at most one of selector, text, url or load_state. state requires selector.",
            ));
        }
        let access = self.browser_access(context, &input.session_id).await?;
        let mut command = serde_json::json!({"action": "wait", "timeout": input.timeout_ms, "frameId": input.frame_id});
        if let Some(selector) = &input.selector {
            command["selector"] = serde_json::json!(selector);
            command["state"] =
                serde_json::json!(input.state.unwrap_or(BrowserElementState::Visible));
        } else if let Some(text) = &input.text {
            command["text"] = serde_json::json!(text);
        } else if let Some(url) = &input.url {
            command["url"] = serde_json::json!(url);
        } else {
            command["loadState"] = serde_json::json!(
                input
                    .load_state
                    .unwrap_or(BrowserLoadState::DomContentLoaded)
            );
        }
        self.run_browser_command(
            &access,
            "wait",
            command,
            input.timeout_ms,
            input.return_snapshot,
        )
        .await
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "action",
        summary = "Navigate, press keys, scroll, hover, double-click, focus, drag, operate form controls, or resolve a dialog.",
        help = "Use an owned session_id from browser.open or browser.list and select an action.kind. Element actions accept an optional frame_id from browser.frames. Take a fresh browser.snapshot when a ref becomes stale or an action outcome is unknown. A completed action can include an observation error; verify its result before retrying.",
        translations(locale(
            "zh-CN",
            summary = "在当前浏览器页面导航、使用历史记录、按键、滚动、操作表单控件或处理对话框。"
        ))
    )]
    async fn browser_action(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserActionInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.control_browser(context, input).await
    }

    #[tool(
        tags(network, interactive, mutate, filesystem),
        name = "screenshot",
        summary = "Capture a browser screenshot and return it as an image attachment.",
        translations(
            locale("ar-SA", summary = "التقط لقطة شاشة للمتصفح وأعِدها كمرفق صورة."),
            locale(
                "de-DE",
                summary = "Erstellt einen Browser-Screenshot und gibt ihn als Bildanhang zurück."
            ),
            locale(
                "es-ES",
                summary = "Captura una captura de pantalla del navegador y la devuelve como adjunto de imagen."
            ),
            locale(
                "fr-FR",
                summary = "Capture une capture d’écran du navigateur et la renvoie comme pièce jointe image."
            ),
            locale(
                "hi-IN",
                summary = "ब्राउज़र स्क्रीनशॉट लेता है और उसे छवि अनुलग्नक के रूप में लौटाता है।"
            ),
            locale(
                "ja-JP",
                summary = "ブラウザーのスクリーンショットを取得し、画像添付として返します。"
            ),
            locale(
                "ko-KR",
                summary = "브라우저 스크린샷을 캡처해 이미지 첨부로 반환합니다."
            ),
            locale(
                "pt-BR",
                summary = "Captura uma captura de tela do navegador e a retorna como anexo de imagem."
            ),
            locale("zh-CN", summary = "捕获浏览器截图并将其作为图片附件返回。"),
            locale("zh-TW", summary = "擷取瀏覽器截圖並將其以圖片附件形式傳回。")
        )
    )]
    async fn browser_screenshot(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserScreenshotInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let access = self.browser_access(context, &input.session_id).await?;
        let screenshot = access
            .page
            .driver
            .command(
                "Page.captureScreenshot",
                serde_json::json!({
                    "format": "png", "captureBeyondViewport": input.full_page,
                }),
            )
            .await?;
        access.page.verify_committed_url().await?;
        let encoded = screenshot["data"]
            .as_str()
            .ok_or_else(|| PluginError::internal("Chrome returned no screenshot data"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| {
                plugin_internal_error_with_context("invalid screenshot data", &error)
            })?;
        let path = match input.path.as_deref() {
            Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
            Some(_) if context.is_global() => {
                return Err(PluginError::invalid_params(
                    "global sessions require an explicit full absolute path",
                ));
            }
            Some(path) => self.workspace_root()?.join(path),
            None => browser_artifacts_root(
                context,
                if context.is_global() {
                    Path::new("")
                } else {
                    self.workspace_root()?
                },
            )
            .join("browser")
            .join(format!("{}.png", uuid::Uuid::new_v4())),
        };
        if input.path.is_some() {
            self.state()?
                .host
                .require_path_permission(
                    agena_plugin_host::sdk::host_api::PathPermissionQuery::write(
                        path.to_string_lossy(),
                    ),
                )
                .await?;
        }
        let size_bytes = bytes.len() as u64;
        crate::artifact_file::persist_replace_or_create(path.clone(), bytes, "browser screenshot")
            .await?;
        let attachment = AttachmentItem {
            kind: AttachmentKind::Image,
            mime: "image/png".to_string(),
            source: AttachmentSource::LocalPath {
                path: path.to_string_lossy().to_string(),
            },
            filename: path
                .file_name()
                .and_then(|value| value.to_str())
                .map(ToOwned::to_owned),
            title: Some(format!("Browser screenshot {}", input.session_id)),
            size_bytes: Some(size_bytes),
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        };
        Ok(ToolInvokeOutput::from_parts(
            "browser screenshot",
            format!("image/png · {size_bytes} bytes"),
            format!("Saved browser screenshot to '{}'.", path.display()),
            Some(serde_json::json!({
                "session_id": input.session_id,
                "path": path,
                "size_bytes": size_bytes,
            })),
            std::collections::BTreeMap::from([
                ("agena.effect".to_string(), "file_changes".to_string()),
                ("path".to_string(), path.to_string_lossy().to_string()),
            ]),
            vec![attachment],
        ))
    }

    #[tool(
        tags(network, interactive, mutate, filesystem),
        name = "download",
        summary = "Collect a browser download triggered by an HTTP(S) URL or by clicking a page control.",
        help = "Supply exactly one of url or selector. Use selector for an export button or a browser-generated file. Chrome download events confirm completion before the tool returns a managed file attachment.",
        translations(
            locale(
                "ar-SA",
                summary = "نزّل عنوان HTTP(S) واحدًا عبر جلسة متصفح مُدارة وأعِد ملفًا محليًا."
            ),
            locale(
                "de-DE",
                summary = "Lädt eine HTTP(S)-URL über eine verwaltete Browser-Sitzung herunter und liefert ein lokales Artefakt."
            ),
            locale(
                "es-ES",
                summary = "Descarga una URL HTTP(S) a través de una sesión de navegador gestionada y devuelve un archivo local."
            ),
            locale(
                "fr-FR",
                summary = "Télécharge une URL HTTP(S) via une session de navigateur gérée et renvoie un artefact local."
            ),
            locale(
                "hi-IN",
                summary = "प्रबंधित ब्राउज़र सत्र के माध्यम से एक HTTP(S) URL डाउनलोड करता है और स्थानीय फ़ाइल लौटाता है।"
            ),
            locale(
                "ja-JP",
                summary = "管理されたブラウザーセッション経由で HTTP(S) URL を1件ダウンロードし、ローカル成果物を返します。"
            ),
            locale(
                "ko-KR",
                summary = "관리되는 브라우저 세션을 통해 HTTP(S) URL 하나를 다운로드하고 로컬 산출물을 반환합니다."
            ),
            locale(
                "pt-BR",
                summary = "Baixa uma URL HTTP(S) por meio de uma sessão de navegador gerenciada e retorna um artefato local."
            ),
            locale(
                "zh-CN",
                summary = "通过受管浏览器会话下载一个 HTTP(S) URL，并返回本地文件。"
            ),
            locale(
                "zh-TW",
                summary = "透過受管理瀏覽器工作階段下載一個 HTTP(S) URL，並傳回本機產物。"
            )
        )
    )]
    async fn browser_download(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserDownloadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let access = self.browser_access(context, &input.session_id).await?;
        let preview = access.page.preview.clone();
        const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;
        let trigger = match (&input.url, &input.selector, &input.frame_id) {
            (Some(url), None, None) => {
                let url = prepare_fetch_url(url).map_err(PluginError::invalid_params)?;
                self.validate_browser_destination(preview.as_ref(), &url)
                    .await?;
                downloads::DownloadTrigger::Navigate(url.to_string())
            }
            (None, Some(selector), frame_id) => downloads::DownloadTrigger::Click {
                selector: selector.clone(),
                frame_id: frame_id.clone(),
            },
            _ => {
                return Err(PluginError::invalid_params(
                    "Supply exactly one of url and selector. frame_id requires selector.",
                ));
            }
        };
        let download_dir = browser_artifacts_root(
            context,
            if context.is_global() {
                Path::new("")
            } else {
                self.workspace_root()?
            },
        )
        .join("browser/downloads")
        .join(uuid::Uuid::new_v4().simple().to_string());
        let result = downloads::transfer(
            access,
            trigger,
            download_dir,
            MAX_DOWNLOAD_BYTES,
            Duration::from_millis(input.timeout_ms),
            self.browser_state.download_gate.clone(),
        )
        .await?;
        if let Some(preview) = &preview {
            preview.validate().await.map_err(managed_preview::denied)?;
        }
        let filename = result.filename.clone().or_else(|| {
            result
                .path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
        });
        let attachment = AttachmentItem {
            kind: AttachmentKind::detect("", filename.as_deref()),
            mime: "application/octet-stream".into(),
            source: AttachmentSource::LocalPath {
                path: result.path.display().to_string(),
            },
            filename: filename.clone(),
            title: Some("Browser download".into()),
            size_bytes: Some(result.size),
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        };
        Ok(ToolInvokeOutput::from_parts(
            "browser download",
            format!("{} bytes · completed GUID {}", result.size, result.guid),
            format!(
                "Saved completed browser download at {}.\n{}",
                result.path.display(),
                result.warnings.join("\n")
            ),
            Some(
                serde_json::json!({"session_id":input.session_id,"url":result.url,"filename":filename,"path":result.path,"size_bytes":result.size,"download_guid":result.guid,"state":"completed","cleanup_warnings":result.warnings}),
            ),
            Default::default(),
            vec![attachment],
        ))
    }

    #[tool(
        tags(network, interactive, mutate, filesystem),
        name = "upload",
        summary = "Set a browser file input using authorized workspace files or local attachments.",
        help = "Pass a CSS selector or snapshot ref for the file input and a list of local file paths. Workspace-relative paths resolve inside the current workspace. Select an element, then use the normal page action to submit the form if needed.",
        translations(locale(
            "zh-CN",
            summary = "将有读取权限的工作区文件或本地附件选入页面的文件输入框。"
        ))
    )]
    async fn browser_upload(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserUploadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.upload_browser_files(context, input).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("url", "preview_id"), non_empty_if_present("url", "preview_id"))]
#[serde(deny_unknown_fields)]
struct BrowserOpenInput {
    /// Public HTTP(S) URL. Supply exactly one of url and preview_id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    /// Running preview owned by this workspace and Agena session. Use browser.list
    /// to discover IDs. Private URLs are never accepted through the url field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview_id: Option<String>,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("session_id"), non_empty("session_id"))]
#[serde(deny_unknown_fields)]
struct BrowserSessionInput {
    session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("session_id", "reason"), non_empty("session_id", "reason"))]
#[serde(deny_unknown_fields)]
struct BrowserTakeoverInput {
    /// Page ID returned by browser.open or browser.list.
    #[arg(max_chars = 128)]
    session_id: String,
    /// What the user needs to do in the browser.
    #[arg(max_chars = 300)]
    reason: String,
}

#[derive(
    Debug, Clone, Default, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput,
)]
#[serde(deny_unknown_fields)]
struct BrowserListInput {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector", "frame_id"),
    non_empty("session_id", "selector"),
    non_empty_if_present("frame_id")
)]
#[serde(deny_unknown_fields)]
struct BrowserClickInput {
    session_id: String,
    /// Use @eN from the latest browser snapshot or a CSS selector.
    #[arg(max_chars = 4096)]
    selector: String,
    /// Frame ID from browser.frames for a CSS selector. Refs can identify their own frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
    /// Return the updated accessibility snapshot. Disable when the next action already has a selector.
    #[serde(default = "default_return_snapshot")]
    return_snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector", "frame_id"),
    non_empty("session_id", "selector"),
    non_empty_if_present("frame_id")
)]
#[serde(deny_unknown_fields)]
struct BrowserTypeInput {
    session_id: String,
    /// Use @eN from the latest browser snapshot or a CSS selector.
    #[arg(max_chars = 4096)]
    selector: String,
    /// Frame ID from browser.frames for a CSS selector. Refs can identify their own frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    #[arg(max_chars = 65536)]
    text: String,
    /// Press the real Enter key after filling the field.
    #[serde(default)]
    press_enter: bool,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
    /// Return the updated accessibility snapshot. Disable when the next action already has a selector.
    #[serde(default = "default_return_snapshot")]
    return_snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector", "text", "url", "frame_id"),
    non_empty("session_id"),
    non_empty_if_present("selector", "text", "url", "frame_id")
)]
#[serde(deny_unknown_fields)]
struct BrowserWaitInput {
    session_id: String,
    /// CSS selector or @eN ref from the latest snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    text: Option<String>,
    /// Substring or wildcard pattern for the page or frame URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    url: Option<String>,
    /// Element state to wait for. Use with selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state: Option<BrowserElementState>,
    /// Page readiness condition. Network idle applies to the whole page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    load_state: Option<BrowserLoadState>,
    /// Frame ID from browser.frames. Defaults to the main document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
    /// Return the updated accessibility snapshot. Disable when the next action already has a selector.
    #[serde(default = "default_return_snapshot")]
    return_snapshot: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BrowserElementState {
    Visible,
    Hidden,
    Attached,
    Detached,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum BrowserLoadState {
    DomContentLoaded,
    Load,
    NetworkIdle,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("session_id", "path"), non_empty("session_id"))]
#[serde(deny_unknown_fields)]
struct BrowserScreenshotInput {
    session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default)]
    full_page: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "url", "selector", "frame_id"),
    non_empty("session_id"),
    non_empty_if_present("url", "selector", "frame_id")
)]
#[serde(deny_unknown_fields)]
struct BrowserDownloadInput {
    /// Existing managed browser page to use for the navigation. Its browser
    /// profile (for example, authenticated cookies) remains intact.
    session_id: String,
    /// Navigate to this HTTP(S) download URL. Supply url or selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 8192)]
    url: Option<String>,
    /// Click this CSS selector or snapshot @ref after Chrome starts listening for a download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    selector: Option<String>,
    /// Optional frame for selector. Discover it with browser.frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
}

const fn default_browser_action_timeout_ms() -> u64 {
    30_000
}

fn browser_artifacts_root(context: &ToolInvokeContext<'_>, project: &Path) -> PathBuf {
    let layout = agena_storage_layout::StorageLayout::current();
    if context.is_global() {
        layout.session_artifacts_dir(context.session_id)
    } else {
        layout.artifacts_dir(project)
    }
}

fn plugin_internal_error_with_context(
    context: &str,
    error: &(dyn std::error::Error + 'static),
) -> PluginError {
    PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
        context, error,
    ))
}

fn log_secondary_plugin_failure(operation: &str, error: &PluginError) {
    tracing::warn!(
        operation,
        diagnostic = %error.diagnostic_message(),
        "secondary browser plugin operation failed"
    );
}
