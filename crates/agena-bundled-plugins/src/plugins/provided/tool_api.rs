use std::sync::Arc;

use crate::plugins::provided::workflow::{
    ToolApiHelpInput, ToolApiListInput, ToolApiSearchInput, ToolApiTagsInput, ToolDiscoveryConfig,
    WorkflowPlanConfig, WorkflowPlugin, WorkflowPluginConfig,
};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput};

pub(crate) const TOOL_API_PLUGIN_ID: &str = "agena.tools";

pub(crate) struct ToolApiPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "tools",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Tool API discovery functions. The runtime resolves tools_call directly to its execution target.",
    translations(
        locale("zh-CN", summary = "发现工具和插件能力；运行时会将 tools_call 直接路由到对应执行目标。"),
        locale("zh-TW", summary = "探索工具與外掛功能；執行階段會將 tools_call 直接路由至對應的執行目標。"),
        locale("ja-JP", summary = "ツールやプラグインの機能を検索します。tools_call は実行時に対象のツールへ直接ルーティングされます。"),
        locale("ko-KR", summary = "도구와 플러그인 기능을 찾습니다. 런타임은 tools_call을 해당 실행 대상으로 직접 전달합니다."),
        locale("fr-FR", summary = "Découvrir les outils et capacités des extensions. À l’exécution, tools_call est directement routé vers sa cible."),
        locale("de-DE", summary = "Werkzeuge und Pluginfunktionen entdecken. tools_call wird zur Laufzeit direkt an das jeweilige Ausführungsziel geleitet."),
        locale("es-ES", summary = "Descubre herramientas y funciones de plugins. En tiempo de ejecución, tools_call se dirige directamente al destino correspondiente."),
        locale("hi-IN", summary = "टूल और प्लगइन क्षमताएँ खोजें। रनटाइम tools_call को सीधे उसके निष्पादन लक्ष्य तक भेजता है।"),
        locale("ar-SA", summary = "اكتشف الأدوات وإمكانات الإضافات. يوجّه وقت التشغيل tools_call مباشرةً إلى هدف التنفيذ المناسب."),
        locale("pt-BR", summary = "Descubra ferramentas e recursos de plugins. Em tempo de execução, tools_call é encaminhado diretamente ao destino de execução.")
    ),
    settings = ToolDiscoveryConfig,
    settings_default = default,
)]
impl ToolApiPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let tool_discovery = agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
            ctx.settings.clone(),
            "invalid tools config",
        )?;
        self.inner.initialize(
            ctx,
            WorkflowPluginConfig {
                tool_discovery,
                plan: WorkflowPlanConfig::default(),
            },
            host,
        )?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Enumerate current tools across one plugin or a batch of plugin targets.",
        translations(
            locale("zh-CN", summary = "列出一个或多个插件当前提供的工具。"),
            locale("zh-TW", summary = "列出一個或多個外掛目前提供的工具。"),
            locale(
                "ja-JP",
                summary = "1 つ以上のプラグインが現在提供しているツールを一覧表示します。"
            ),
            locale(
                "ko-KR",
                summary = "하나 이상의 플러그인에서 현재 제공하는 도구를 나열합니다."
            ),
            locale(
                "fr-FR",
                summary = "Afficher les outils actuellement proposés par un ou plusieurs plugins."
            ),
            locale(
                "de-DE",
                summary = "Die aktuell verfügbaren Werkzeuge eines oder mehrerer Plugins auflisten."
            ),
            locale(
                "es-ES",
                summary = "Enumera las herramientas disponibles de uno o varios plugins."
            ),
            locale("hi-IN", summary = "एक या अधिक प्लगइन में उपलब्ध टूल की सूची दें।"),
            locale(
                "ar-SA",
                summary = "اعرض الأدوات المتاحة حاليًا في إضافة واحدة أو عدة إضافات."
            ),
            locale(
                "pt-BR",
                summary = "Liste as ferramentas disponíveis em um ou mais plugins."
            )
        )
    )]
    async fn list(&self, input: &ToolApiListInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_list(input).await
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Search execution tools with one or many queries across one or many plugin targets.",
        translations(
            locale(
                "zh-CN",
                summary = "使用一个或多个关键词搜索一个或多个插件中的可执行工具。"
            ),
            locale(
                "zh-TW",
                summary = "使用一個或多個關鍵字搜尋一個或多個外掛中的可執行工具。"
            ),
            locale(
                "ja-JP",
                summary = "1 つ以上の検索語を使って、複数のプラグインから実行可能なツールを検索します。"
            ),
            locale(
                "ko-KR",
                summary = "하나 이상의 검색어로 여러 플러그인의 실행 도구를 검색합니다."
            ),
            locale(
                "fr-FR",
                summary = "Rechercher des outils exécutables dans un ou plusieurs plugins avec une ou plusieurs requêtes."
            ),
            locale(
                "de-DE",
                summary = "Ausführbare Werkzeuge in einem oder mehreren Plugins mit einer oder mehreren Suchanfragen finden."
            ),
            locale(
                "es-ES",
                summary = "Busca herramientas ejecutables en uno o varios plugins con una o varias consultas."
            ),
            locale(
                "hi-IN",
                summary = "एक या अधिक खोज प्रश्नों से एक या अधिक प्लगइन में निष्पादन योग्य टूल खोजें।"
            ),
            locale(
                "ar-SA",
                summary = "ابحث عن أدوات قابلة للتنفيذ في إضافة واحدة أو عدة إضافات باستخدام استعلام واحد أو أكثر."
            ),
            locale(
                "pt-BR",
                summary = "Pesquise ferramentas executáveis em um ou mais plugins usando uma ou mais consultas."
            )
        )
    )]
    async fn search(&self, input: &ToolApiSearchInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_search(input).await
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Get reusable schemas, examples, and usage notes for one Agena execution tool or a batch of tools.",
        translations(
            locale(
                "zh-CN",
                summary = "获取一个或多个 Agena 可执行工具的输入结构、示例和用法说明。"
            ),
            locale(
                "zh-TW",
                summary = "取得一個或多個 Agena 可執行工具的輸入結構、範例與使用說明。"
            ),
            locale(
                "ja-JP",
                summary = "Agena の実行ツール 1 つまたは複数について、入力形式、例、使い方を取得します。"
            ),
            locale(
                "ko-KR",
                summary = "Agena 실행 도구 하나 이상의 입력 형식, 예시, 사용 안내를 가져옵니다."
            ),
            locale(
                "fr-FR",
                summary = "Obtenir les schémas, exemples et conseils d’utilisation d’un ou plusieurs outils Agena."
            ),
            locale(
                "de-DE",
                summary = "Schemas, Beispiele und Hinweise zur Nutzung eines oder mehrerer Agena-Werkzeuge abrufen."
            ),
            locale(
                "es-ES",
                summary = "Obtén esquemas, ejemplos e indicaciones de uso de una o varias herramientas de Agena."
            ),
            locale(
                "hi-IN",
                summary = "एक या अधिक Agena निष्पादन टूल के स्कीमा, उदाहरण और उपयोग निर्देश प्राप्त करें।"
            ),
            locale(
                "ar-SA",
                summary = "احصل على المخططات والأمثلة وإرشادات الاستخدام لأداة تنفيذ واحدة أو عدة أدوات في Agena."
            ),
            locale(
                "pt-BR",
                summary = "Obtenha esquemas, exemplos e orientações de uso de uma ou mais ferramentas de execução do Agena."
            )
        )
    )]
    async fn help(&self, input: &ToolApiHelpInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_help(input).await
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "List tool tags across one plugin or a batch of plugin targets.",
        translations(
            locale("zh-CN", summary = "列出一个或多个插件中工具使用的标签。"),
            locale("zh-TW", summary = "列出一個或多個外掛中工具使用的標籤。"),
            locale(
                "ja-JP",
                summary = "1 つ以上のプラグインに含まれるツールのタグを一覧表示します。"
            ),
            locale(
                "ko-KR",
                summary = "하나 이상의 플러그인에 포함된 도구의 태그를 나열합니다."
            ),
            locale(
                "fr-FR",
                summary = "Lister les étiquettes des outils d’un ou plusieurs plugins."
            ),
            locale(
                "de-DE",
                summary = "Die Tags der Werkzeuge eines oder mehrerer Plugins auflisten."
            ),
            locale(
                "es-ES",
                summary = "Enumera las etiquetas de las herramientas de uno o varios plugins."
            ),
            locale("hi-IN", summary = "एक या अधिक प्लगइन के टूल टैग की सूची दें।"),
            locale("ar-SA", summary = "اعرض وسوم الأدوات في إضافة واحدة أو عدة إضافات."),
            locale(
                "pt-BR",
                summary = "Liste as tags das ferramentas de um ou mais plugins."
            )
        )
    )]
    async fn tags(&self, input: &ToolApiTagsInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_tags(input).await
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Enumerate one or many selected plugins with version, summary, tags, and tool count.",
        translations(
            locale(
                "zh-CN",
                summary = "列出一个或多个指定插件的版本、简介、标签和工具数量。"
            ),
            locale(
                "zh-TW",
                summary = "列出一個或多個指定外掛的版本、簡介、標籤與工具數量。"
            ),
            locale(
                "ja-JP",
                summary = "指定した 1 つ以上のプラグインについて、バージョン、概要、タグ、ツール数を一覧表示します。"
            ),
            locale(
                "ko-KR",
                summary = "선택한 하나 이상의 플러그인에서 버전, 설명, 태그, 도구 수를 나열합니다."
            ),
            locale(
                "fr-FR",
                summary = "Afficher la version, le résumé, les étiquettes et le nombre d’outils des plugins sélectionnés."
            ),
            locale(
                "de-DE",
                summary = "Version, Kurzbeschreibung, Tags und Werkzeuganzahl der ausgewählten Plugins anzeigen."
            ),
            locale(
                "es-ES",
                summary = "Enumera la versión, el resumen, las etiquetas y el número de herramientas de los plugins seleccionados."
            ),
            locale(
                "hi-IN",
                summary = "चुने गए एक या अधिक प्लगइन के संस्करण, सारांश, टैग और टूल की संख्या दिखाएँ।"
            ),
            locale(
                "ar-SA",
                summary = "اعرض إصدار الإضافات المحددة وملخصها ووسومها وعدد أدواتها."
            ),
            locale(
                "pt-BR",
                summary = "Liste a versão, o resumo, as tags e a quantidade de ferramentas dos plugins selecionados."
            )
        )
    )]
    async fn plugins_list(&self, input: &ToolApiListInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plugins_list(input).await
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Search loaded plugins with one or many queries and optional multi-plugin scope.",
        translations(
            locale(
                "zh-CN",
                summary = "使用一个或多个关键词搜索已加载的插件，可限定在多个插件范围内。"
            ),
            locale(
                "zh-TW",
                summary = "使用一個或多個關鍵字搜尋已載入的外掛，也可限定多個外掛範圍。"
            ),
            locale(
                "ja-JP",
                summary = "1 つ以上の検索語で読み込み済みのプラグインを検索し、必要に応じて対象を複数指定します。"
            ),
            locale(
                "ko-KR",
                summary = "하나 이상의 검색어로 로드된 플러그인을 검색하고 필요한 경우 여러 플러그인으로 범위를 제한합니다."
            ),
            locale(
                "fr-FR",
                summary = "Rechercher parmi les plugins chargés avec une ou plusieurs requêtes et, si besoin, plusieurs plugins ciblés."
            ),
            locale(
                "de-DE",
                summary = "Geladene Plugins mit einer oder mehreren Suchanfragen durchsuchen und optional auf mehrere Plugins eingrenzen."
            ),
            locale(
                "es-ES",
                summary = "Busca entre los plugins cargados con una o varias consultas y, si quieres, limita la búsqueda a varios plugins."
            ),
            locale(
                "hi-IN",
                summary = "एक या अधिक प्रश्नों से लोड किए गए प्लगइन खोजें और चाहें तो खोज को कई प्लगइन तक सीमित करें।"
            ),
            locale(
                "ar-SA",
                summary = "ابحث في الإضافات المحمّلة باستعلام واحد أو أكثر، مع إمكانية حصر البحث في عدة إضافات."
            ),
            locale(
                "pt-BR",
                summary = "Pesquise entre os plugins carregados com uma ou mais consultas e, se quiser, limite a busca a vários plugins."
            )
        )
    )]
    async fn plugins_search(&self, input: &ToolApiSearchInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plugins_search(input).await
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "List plugin tags across one plugin or a batch of plugin targets.",
        translations(
            locale("zh-CN", summary = "列出一个或多个插件声明的标签。"),
            locale("zh-TW", summary = "列出一個或多個外掛宣告的標籤。"),
            locale(
                "ja-JP",
                summary = "1 つ以上のプラグインが宣言しているタグを一覧表示します。"
            ),
            locale("ko-KR", summary = "하나 이상의 플러그인이 선언한 태그를 나열합니다."),
            locale(
                "fr-FR",
                summary = "Lister les étiquettes déclarées par un ou plusieurs plugins."
            ),
            locale(
                "de-DE",
                summary = "Die von einem oder mehreren Plugins deklarierten Tags auflisten."
            ),
            locale(
                "es-ES",
                summary = "Enumera las etiquetas declaradas por uno o varios plugins."
            ),
            locale("hi-IN", summary = "एक या अधिक प्लगइन द्वारा घोषित टैग की सूची दें।"),
            locale(
                "ar-SA",
                summary = "اعرض الوسوم التي تعلنها إضافة واحدة أو عدة إضافات."
            ),
            locale("pt-BR", summary = "Liste as tags declaradas por um ou mais plugins.")
        )
    )]
    async fn plugins_tags(&self, input: &ToolApiTagsInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plugins_tags(input).await
    }
}

#[cfg(test)]
mod tests {
    use super::ToolApiPlugin;
    use agena_plugin_host::sdk::{Plugin, ToolDefinition};

    fn definition_for(tool_name: &str) -> ToolDefinition {
        ToolApiPlugin::new()
            .manifest()
            .tools
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .unwrap_or_else(|| panic!("missing tools plugin tool `{tool_name}`"))
    }

    #[test]
    fn definitions_distinguish_tool_api_functions_from_execution_tools() {
        let help = definition_for("help");
        assert!(
            help.docs
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("one Agena execution tool"))
        );
        let help_tool_description = help
            .contract
            .input_schema
            .pointer("/properties/tool/description")
            .and_then(serde_json::Value::as_str)
            .expect("help tool-name description");
        assert!(help_tool_description.contains("exact execution-tool name"));
        assert!(help_tool_description.contains("non-empty array"));
        assert!(help_tool_description.contains("`tools_list` or `tools_search`"));
    }
}
