//! `agena.settings` plugin: inspect and edit Agena's layered `agena.json` files.

use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock},
};

use agena_macros::ToolInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::config::{
    ConfigError, ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsGetInput,
    ConfigSettingsLayer, ConfigSettingsListInput, ConfigSettingsPatchInput,
    ConfigSettingsPathInput, ConfigSettingsReadResponse, ConfigSettingsSetInput,
    ConfigSettingsSource, ConfigSettingsValidateResponse, delete_layered_file_setting,
    list_file_settings, list_json_path, patch_layered_file_settings, read_file_setting,
    set_layered_file_setting, validate_layered_file_settings,
};
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{HostClient, HostConfigReloadRequestResponse};
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeOutput, ToolTag};

pub(crate) const SETTINGS_PLUGIN_ID: &str = "agena.settings";

pub(crate) struct SettingsPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
    config: OnceLock<SettingsPluginConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SettingsScope {
    Config,
    Meta,
}

/// Selects the persisted config file to read or mutate. `global` is the
/// user-level `~/agena/agena.json`; `workspace` is
/// `<workspace_root>/.agena/agena.json`.
type SettingsLayer = ConfigSettingsLayer;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct SettingsPluginConfig {
    reads: SettingsReadDefaultsConfig,
    edits: SettingsEditDefaultsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct SettingsReadDefaultsConfig {
    source: ConfigSettingsSource,
    file_layer: SettingsLayer,
    effective_scope: SettingsScope,
    list_recursive: bool,
}

impl Default for SettingsReadDefaultsConfig {
    fn default() -> Self {
        Self {
            source: ConfigSettingsSource::Effective,
            file_layer: SettingsLayer::Global,
            effective_scope: SettingsScope::Config,
            list_recursive: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct SettingsEditDefaultsConfig {
    file_layer: SettingsLayer,
    validate_by_default: bool,
    reload_after_write: bool,
}

impl Default for SettingsEditDefaultsConfig {
    fn default() -> Self {
        Self {
            file_layer: SettingsLayer::Global,
            validate_by_default: true,
            reload_after_write: true,
        }
    }
}

fn settings_plugin_metadata() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            "",
            "Settings Plugin Config",
            "Default read and edit behavior for the agena.settings plugin.",
        ),
        (
            "/reads",
            "Reads",
            "Defaults applied when reading or listing settings through settings.",
        ),
        (
            "/reads/source",
            "Default Source",
            "Which settings source is used when get/list calls omit source.",
        ),
        (
            "/reads/file_layer",
            "Default Read Layer",
            "Which persisted config file is read when source=file and layer is omitted.",
        ),
        (
            "/reads/effective_scope",
            "Effective Scope",
            "Which branch of the effective settings snapshot is read when source=effective and scope is omitted.",
        ),
        (
            "/reads/list_recursive",
            "List Recursively",
            "Whether list calls recurse into nested structures when recursive is omitted.",
        ),
        (
            "/edits",
            "Edits",
            "Defaults applied when mutating agena.json through set, delete, or patch.",
        ),
        (
            "/edits/file_layer",
            "Default Edit Layer",
            "Which persisted config file is edited when layer is omitted.",
        ),
        (
            "/edits/validate_by_default",
            "Validate by Default",
            "Runs config validation unless the caller explicitly disables it.",
        ),
        (
            "/edits/reload_after_write",
            "Reload After Write",
            "Reloads the active runtime configuration after a successful write unless the caller overrides it.",
        ),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[input(trim("path"))]
#[serde(default, deny_unknown_fields)]
struct SettingsGetToolInput {
    path: Option<String>,
    scope: Option<SettingsScope>,
    source: Option<ConfigSettingsSource>,
    layer: Option<SettingsLayer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[input(trim("path"))]
#[serde(default, deny_unknown_fields)]
struct SettingsListToolInput {
    path: Option<String>,
    scope: Option<SettingsScope>,
    source: Option<ConfigSettingsSource>,
    layer: Option<SettingsLayer>,
    recursive: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[input(trim("path"))]
#[serde(default, deny_unknown_fields)]
struct SettingsInspectToolInput {
    path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[serde(default, deny_unknown_fields)]
struct SettingsValidateToolInput {
    layer: Option<SettingsLayer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("path"), non_empty("path"))]
#[serde(deny_unknown_fields)]
struct SettingsSetToolInput {
    #[serde(default)]
    expected_revision: Option<String>,
    path: String,
    value: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<SettingsLayer>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reload: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("path"), non_empty("path"))]
#[serde(deny_unknown_fields)]
struct SettingsDeleteToolInput {
    #[serde(default)]
    expected_revision: Option<String>,
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<SettingsLayer>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reload: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("path"))]
#[serde(deny_unknown_fields)]
struct SettingsPatchToolInput {
    #[serde(default)]
    expected_revision: Option<String>,
    #[serde(default)]
    path: Option<String>,
    changes: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<SettingsLayer>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reload: Option<bool>,
}

#[derive(Debug, Clone)]
struct SettingsConfigMeta {
    global_path: PathBuf,
    global_found: bool,
    workspace_path: PathBuf,
    workspace_found: bool,
    applied_layers: JsonValue,
}

impl SettingsConfigMeta {
    fn from_value(meta: &JsonValue) -> SdkResult<Self> {
        Ok(Self {
            global_path: required_meta_path(meta, "config_path")?,
            global_found: meta
                .get("config_found")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            workspace_path: required_meta_path(meta, "project_config_path")?,
            workspace_found: meta
                .get("project_config_found")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            applied_layers: meta
                .get("applied_layers")
                .cloned()
                .unwrap_or_else(|| JsonValue::Array(Vec::new())),
        })
    }

    fn file(&self, layer: SettingsLayer) -> (&PathBuf, bool) {
        match layer {
            SettingsLayer::Global => (&self.global_path, self.global_found),
            SettingsLayer::Workspace => (&self.workspace_path, self.workspace_found),
        }
    }
}

#[derive(Debug, Serialize)]
struct SettingsInspectFileValue {
    layer: SettingsLayer,
    config_path: PathBuf,
    config_found: bool,
    path: Option<String>,
    defined: bool,
    value: JsonValue,
}

#[derive(Debug, Serialize)]
struct SettingsInspectResponse {
    path: Option<String>,
    global: SettingsInspectFileValue,
    workspace: SettingsInspectFileValue,
    effective: JsonValue,
    applied_layers: JsonValue,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "settings",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Inspect and edit Agena's global and workspace agena.json settings.",
    translations(
        locale("zh-CN", summary = "查看并编辑 Agena 的全局和工作区 agena.json 设置。"),
        locale("zh-TW", summary = "檢視並編輯 Agena 的全域與工作區 agena.json 設定。"),
        locale("ja-JP", summary = "Agena のグローバルおよびワークスペースの agena.json 設定を確認・編集します。"),
        locale("ko-KR", summary = "Agena의 전역 및 작업 공간 agena.json 설정을 확인하고 편집합니다."),
        locale("fr-FR", summary = "Consulter et modifier les paramètres agena.json globaux et de l’espace de travail."),
        locale("de-DE", summary = "Globale und arbeitsbereichsbezogene agena.json-Einstellungen von Agena prüfen und bearbeiten."),
        locale("es-ES", summary = "Consulta y edita la configuración global y del espacio de trabajo en agena.json."),
        locale("hi-IN", summary = "Agena की वैश्विक और वर्कस्पेस agena.json सेटिंग देखें और संपादित करें।"),
        locale("ar-SA", summary = "افحص إعدادات agena.json العامة والخاصة بمساحة العمل في Agena وعدّلها."),
        locale("pt-BR", summary = "Consulte e edite as configurações globais e do espaço de trabalho no agena.json do Agena.")
    ),
    settings = SettingsPluginConfig,
    settings_default = default,
    settings_metadata = settings_plugin_metadata(),
)]
impl SettingsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            config: OnceLock::new(),
        }
    }

    #[hook(init)]
    async fn init(
        &self,
        ctx: agena_plugin_host::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        let config = agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
            ctx.settings,
            "invalid settings plugin config",
        )?;
        self.config
            .set(config)
            .map_err(|_| PluginError::internal("settings plugin config already initialized"))?;
        *self
            .host
            .write()
            .map_err(|_| PluginError::internal("settings plugin host lock poisoned"))? = Some(host);
        Ok(agena_plugin_host::sdk::InitOutcome::ack(
            agena_plugin_host::sdk::Plugin::manifest(self),
        ))
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::internal("settings plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::internal("settings plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&SettingsPluginConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("settings plugin invoked before init"))
    }

    fn read_source(
        &self,
        requested: Option<ConfigSettingsSource>,
    ) -> SdkResult<ConfigSettingsSource> {
        Ok(requested.unwrap_or(self.config()?.reads.source))
    }

    fn read_layer(&self, requested: Option<SettingsLayer>) -> SdkResult<SettingsLayer> {
        Ok(requested.unwrap_or(self.config()?.reads.file_layer))
    }

    fn edit_layer(&self, requested: Option<SettingsLayer>) -> SdkResult<SettingsLayer> {
        Ok(requested.unwrap_or(self.config()?.edits.file_layer))
    }

    fn effective_scope(
        &self,
        requested: Option<SettingsScope>,
        source: ConfigSettingsSource,
    ) -> SdkResult<Option<SettingsScope>> {
        Ok(match source {
            ConfigSettingsSource::Effective => {
                Some(requested.unwrap_or(self.config()?.reads.effective_scope))
            }
            ConfigSettingsSource::File => requested,
        })
    }

    fn edit_options(
        &self,
        dry_run: bool,
        validate: Option<bool>,
        reload: Option<bool>,
    ) -> SdkResult<ConfigSettingsEditOptions> {
        let config = self.config()?;
        Ok(ConfigSettingsEditOptions {
            expected_revision: None,
            dry_run,
            validate: validate.unwrap_or(config.edits.validate_by_default),
            reload: reload.unwrap_or(config.edits.reload_after_write),
        })
    }

    async fn config_meta(&self) -> SdkResult<SettingsConfigMeta> {
        let host = self.host()?;
        let meta = host.read_config(Some("meta".to_string())).await?;
        SettingsConfigMeta::from_value(&meta)
    }

    async fn effective_config_value(&self, path: Option<&str>) -> SdkResult<JsonValue> {
        let host = self.host()?;
        host.read_config(effective_host_path(path)).await
    }

    #[tool(
        summary = "Read one settings path.",
        help = "Use `source=file` with `layer=global|workspace` for persisted values. Effective reads merge both files plus environment and CLI layers; prefer explicit `scope=config|meta` with a relative path.",
        translations(
            locale(
                "zh-CN",
                summary = "读取一个设置路径。",
                help = "读取持久化值时使用 `source=file` 和 `layer=global|workspace`。effective 读取会合并两个配置文件以及环境变量和命令行层；请优先通过相对路径明确指定 `scope=config|meta`。"
            ),
            locale(
                "zh-TW",
                summary = "讀取一個設定路徑。",
                help = "讀取持久化值時使用 `source=file` 與 `layer=global|workspace`。effective 讀取會合併兩個設定檔，以及環境變數和命令列層；請優先使用相對路徑明確指定 `scope=config|meta`。"
            ),
            locale(
                "ja-JP",
                summary = "設定パスを 1 つ読み取ります。",
                help = "永続化された値には `source=file` と `layer=global|workspace` を指定します。effective は両方の設定ファイルに環境変数と CLI の値を重ねて返します。相対パスで `scope=config|meta` を明示してください。"
            ),
            locale(
                "ko-KR",
                summary = "설정 경로 하나를 읽습니다.",
                help = "저장된 값을 읽으려면 `source=file`, `layer=global|workspace`를 사용하세요. effective 조회는 두 설정 파일과 환경 변수·CLI 계층을 병합합니다. 상대 경로와 함께 `scope=config|meta`를 명시하는 편이 좋습니다."
            ),
            locale(
                "fr-FR",
                summary = "Lire un chemin de configuration.",
                help = "Pour les valeurs enregistrées, utilisez `source=file` avec `layer=global|workspace`. Une lecture effective fusionne les deux fichiers, l’environnement et la ligne de commande. Précisez de préférence `scope=config|meta` avec un chemin relatif."
            ),
            locale(
                "de-DE",
                summary = "Einen Einstellungspfad lesen.",
                help = "Für gespeicherte Werte verwenden Sie `source=file` mit `layer=global|workspace`. Effektive Werte führen beide Dateien sowie Umgebungs- und CLI-Einstellungen zusammen. Geben Sie vorzugsweise `scope=config|meta` und einen relativen Pfad an."
            ),
            locale(
                "es-ES",
                summary = "Lee una ruta de configuración.",
                help = "Para valores guardados, usa `source=file` con `layer=global|workspace`. La lectura effective combina ambos archivos y las capas de entorno y CLI. Es preferible indicar `scope=config|meta` con una ruta relativa."
            ),
            locale(
                "hi-IN",
                summary = "एक सेटिंग पथ पढ़ें।",
                help = "सहेजे गए मान के लिए `source=file` और `layer=global|workspace` दें। effective रीड दोनों फ़ाइलों के साथ environment और CLI स्तरों को मिलाता है। सापेक्ष पथ के साथ `scope=config|meta` स्पष्ट रूप से देना बेहतर है।"
            ),
            locale(
                "ar-SA",
                summary = "اقرأ مسار إعداد واحدًا.",
                help = "للقيم المحفوظة استخدم `source=file` مع `layer=global|workspace`. تدمج القراءة الفعلية الملفين وطبقتي البيئة وسطر الأوامر. يُفضّل تحديد `scope=config|meta` صراحةً مع مسار نسبي."
            ),
            locale(
                "pt-BR",
                summary = "Leia um caminho de configuração.",
                help = "Para valores persistidos, use `source=file` com `layer=global|workspace`. A leitura effective combina os dois arquivos e as camadas de ambiente e CLI. Prefira indicar `scope=config|meta` explicitamente com um caminho relativo."
            )
        ),
        tags(
            ToolTag::Query,
            ToolTag::Discovery,
            ToolTag::Filesystem,
            settings_tag(),
            settings_read_tag()
        )
    )]
    async fn get(&self, input: SettingsGetToolInput) -> SdkResult<ToolInvokeOutput> {
        let meta = self.config_meta().await?;
        let source = self.read_source(input.source)?;
        let layer = self.read_layer(input.layer)?;
        let scope = self.effective_scope(input.scope, source)?;
        let response = match source {
            ConfigSettingsSource::File => {
                if scope == Some(SettingsScope::Meta) {
                    return Err(PluginError::invalid_params(
                        "settings get with source=file does not support scope=meta",
                    ));
                }
                let (config_path, _) = meta.file(layer);
                read_file_setting(
                    config_path.clone(),
                    ConfigSettingsGetInput {
                        target: ConfigSettingsPathInput {
                            path: input.path.clone(),
                        },
                        source,
                    },
                )
                .map_err(map_err)?
            }
            ConfigSettingsSource::Effective => {
                let (config_path, config_found) = meta.file(SettingsLayer::Global);
                ConfigSettingsReadResponse {
                    revision: None,
                    value: self
                        .effective_config_value(
                            resolve_effective_settings_path(scope, input.path.as_deref())?
                                .as_deref(),
                        )
                        .await?,
                    config_path: config_path.clone(),
                    config_found,
                    source,
                    path: input.path,
                }
            }
        };
        output_with_layer(
            "Settings value",
            "Read settings value.",
            &response,
            (source == ConfigSettingsSource::File).then_some(layer),
        )
    }

    #[tool(
        summary = "List settings paths.",
        translations(
            locale("zh-CN", summary = "列出设置路径。"),
            locale("zh-TW", summary = "列出設定路徑。"),
            locale("ja-JP", summary = "設定パスを一覧表示します。"),
            locale("ko-KR", summary = "설정 경로를 나열합니다."),
            locale("fr-FR", summary = "Lister les chemins de configuration."),
            locale("de-DE", summary = "Einstellungspfade auflisten."),
            locale("es-ES", summary = "Enumera las rutas de configuración."),
            locale("hi-IN", summary = "सेटिंग पथ की सूची दें।"),
            locale("ar-SA", summary = "اعرض مسارات الإعدادات."),
            locale("pt-BR", summary = "Liste os caminhos de configuração.")
        ),
        tags(
            ToolTag::Query,
            ToolTag::Discovery,
            ToolTag::Filesystem,
            settings_tag(),
            settings_read_tag()
        )
    )]
    async fn list(&self, input: SettingsListToolInput) -> SdkResult<ToolInvokeOutput> {
        let meta = self.config_meta().await?;
        let source = self.read_source(input.source)?;
        let layer = self.read_layer(input.layer)?;
        let scope = self.effective_scope(input.scope, source)?;
        let recursive = input
            .recursive
            .unwrap_or(self.config()?.reads.list_recursive);
        let response = match source {
            ConfigSettingsSource::File => {
                if scope == Some(SettingsScope::Meta) {
                    return Err(PluginError::invalid_params(
                        "settings list with source=file does not support scope=meta",
                    ));
                }
                let (config_path, _) = meta.file(layer);
                list_file_settings(
                    config_path.clone(),
                    ConfigSettingsListInput {
                        target: ConfigSettingsPathInput {
                            path: input.path.clone(),
                        },
                        source,
                        recursive,
                    },
                )
                .map_err(map_err)?
            }
            ConfigSettingsSource::Effective => {
                let value = self.host()?.read_config(None).await?;
                let items = list_json_path(
                    &value,
                    resolve_effective_settings_path(scope, input.path.as_deref())?.as_deref(),
                    recursive,
                )
                .map_err(map_err)?;
                let (config_path, config_found) = meta.file(SettingsLayer::Global);
                crate::config::ConfigSettingsListResponse {
                    revision: None,
                    config_path: config_path.clone(),
                    config_found,
                    source,
                    path: input.path,
                    items,
                }
            }
        };
        let count = response.items.len();
        output_with_layer(
            "Settings items",
            format!(
                "Listed {count} settings item{}.",
                if count == 1 { "" } else { "s" }
            ),
            &response,
            (source == ConfigSettingsSource::File).then_some(layer),
        )
    }

    #[tool(
        summary = "Inspect a setting across every config layer.",
        help = "Returns the persisted global value, persisted workspace value, effective merged value, source file paths, and applied-layer metadata. Secret values are always redacted.",
        translations(
            locale(
                "zh-CN",
                summary = "查看某项设置在所有配置层中的取值。",
                help = "返回已保存的全局值、工作区值、合并后的生效值、来源文件路径和应用层元数据。秘密值始终会脱敏。"
            ),
            locale(
                "zh-TW",
                summary = "檢視某項設定在所有設定層中的值。",
                help = "回傳已儲存的全域值、工作區值、合併後的生效值、來源檔案路徑與套用層中繼資料。機密值一律會遮蔽。"
            ),
            locale(
                "ja-JP",
                summary = "設定値をすべての構成レイヤーで確認します。",
                help = "保存済みのグローバル値、ワークスペース値、統合後の有効値、参照元ファイルのパス、適用レイヤーの情報を返します。秘密値は常に伏せられます。"
            ),
            locale(
                "ko-KR",
                summary = "모든 설정 계층에서 설정 값을 확인합니다.",
                help = "저장된 전역 값과 작업 공간 값, 병합된 유효 값, 원본 파일 경로, 적용된 계층 정보를 반환합니다. 비밀 값은 항상 가려집니다."
            ),
            locale(
                "fr-FR",
                summary = "Examiner une valeur dans toutes les couches de configuration.",
                help = "Renvoie les valeurs globales et d’espace de travail enregistrées, la valeur effective fusionnée, les chemins des fichiers source et les métadonnées des couches appliquées. Les secrets sont toujours masqués."
            ),
            locale(
                "de-DE",
                summary = "Eine Einstellung über alle Konfigurationsebenen hinweg prüfen.",
                help = "Liefert den gespeicherten globalen und Workspace-Wert, den zusammengeführten effektiven Wert, Quellpfade und Metadaten der angewendeten Ebenen. Geheimnisse werden immer ausgeblendet."
            ),
            locale(
                "es-ES",
                summary = "Inspecciona un ajuste en todas las capas de configuración.",
                help = "Devuelve el valor global guardado, el valor del espacio de trabajo, el valor efectivo combinado, las rutas de los archivos de origen y los metadatos de las capas aplicadas. Los secretos siempre se ocultan."
            ),
            locale(
                "hi-IN",
                summary = "हर कॉन्फ़िगरेशन स्तर में सेटिंग देखें।",
                help = "सहेजा गया वैश्विक मान, कार्यक्षेत्र मान, मिला हुआ प्रभावी मान, स्रोत फ़ाइल पथ और लागू स्तरों का मेटाडेटा लौटाता है। गोपनीय मान हमेशा छिपाए जाते हैं।"
            ),
            locale(
                "ar-SA",
                summary = "افحص إعدادًا عبر جميع طبقات التكوين.",
                help = "يعرض القيمة العامة المحفوظة وقيمة مساحة العمل والقيمة الفعلية المدمجة ومسارات ملفات المصدر وبيانات الطبقات المطبقة. تُحجب القيم السرية دائمًا."
            ),
            locale(
                "pt-BR",
                summary = "Inspecione uma configuração em todas as camadas.",
                help = "Retorna o valor global salvo, o valor do espaço de trabalho, o valor efetivo combinado, os caminhos dos arquivos de origem e os metadados das camadas aplicadas. Valores secretos são sempre ocultados."
            )
        ),
        tags(
            ToolTag::Query,
            ToolTag::Discovery,
            ToolTag::Filesystem,
            settings_tag(),
            settings_read_tag()
        )
    )]
    async fn inspect(&self, input: SettingsInspectToolInput) -> SdkResult<ToolInvokeOutput> {
        let meta = self.config_meta().await?;
        let global = inspect_file_value(&meta, SettingsLayer::Global, input.path.clone())?;
        let workspace = inspect_file_value(&meta, SettingsLayer::Workspace, input.path.clone())?;
        let effective_path =
            resolve_effective_settings_path(Some(SettingsScope::Config), input.path.as_deref())?;
        let effective = self
            .effective_config_value(effective_path.as_deref())
            .await?;
        let response = SettingsInspectResponse {
            path: input.path,
            global,
            workspace,
            effective,
            applied_layers: meta.applied_layers,
        };
        output(
            "Settings inspection",
            "Inspected global, workspace, and effective settings values.",
            &response,
        )
    }

    #[tool(
        summary = "Set one settings value.",
        help = "Writes the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing; dry runs request read permission for both config files instead of write permission.",
        translations(
            locale(
                "zh-CN",
                summary = "设置一个配置值。",
                help = "写入 `layer` 指定的全局或工作区配置，并校验合并后的分层配置。使用 `dry_run=true` 可只预览不写入；此时会请求读取两个配置文件的权限，而不是写权限。"
            ),
            locale(
                "zh-TW",
                summary = "設定一個設定值。",
                help = "寫入 `layer` 指定的全域或工作區設定，並驗證合併後的分層設定。使用 `dry_run=true` 可預覽而不寫入；此時會要求讀取兩個設定檔的權限，而非寫入權限。"
            ),
            locale(
                "ja-JP",
                summary = "設定値を 1 つ書き込みます。",
                help = "`layer` で選んだグローバルまたはワークスペース設定を書き込み、統合後の構成を検証します。`dry_run=true` なら書き込まずに確認でき、その場合は両方の設定ファイルへの読み取り権限を求めます。"
            ),
            locale(
                "ko-KR",
                summary = "설정 값 하나를 지정합니다.",
                help = "`layer`로 선택한 전역 또는 작업 공간 설정을 쓰고 병합된 구성을 검증합니다. `dry_run=true`로 쓰지 않고 미리 볼 수 있으며, 이때는 쓰기 대신 두 설정 파일에 대한 읽기 권한을 요청합니다."
            ),
            locale(
                "fr-FR",
                summary = "Définir une valeur de configuration.",
                help = "Écrit dans la configuration globale ou d’espace de travail choisie par `layer`, puis valide l’ensemble fusionné. `dry_run=true` permet de prévisualiser sans écrire ; il demande alors l’accès en lecture aux deux fichiers plutôt qu’un accès en écriture."
            ),
            locale(
                "de-DE",
                summary = "Einen Einstellungswert setzen.",
                help = "Schreibt in die mit `layer` gewählte globale oder Workspace-Konfiguration und validiert die zusammengeführte Konfiguration. Mit `dry_run=true` wird nur eine Vorschau erstellt; dafür werden Leserechte auf beide Dateien statt Schreibrechte angefordert."
            ),
            locale(
                "es-ES",
                summary = "Establece un valor de configuración.",
                help = "Escribe en la configuración global o del espacio de trabajo indicada por `layer` y valida la configuración combinada. Con `dry_run=true` puedes previsualizar sin escribir; se solicita permiso de lectura para ambos archivos en lugar de permiso de escritura."
            ),
            locale(
                "hi-IN",
                summary = "एक सेटिंग मान लिखें।",
                help = "`layer` से चुने गए वैश्विक या कार्यक्षेत्र कॉन्फ़िगरेशन में लिखकर संयुक्त परतों की जाँच करता है। `dry_run=true` बिना लिखे पूर्वावलोकन करता है; तब लिखने की जगह दोनों कॉन्फ़िग फ़ाइलों की पढ़ने की अनुमति माँगी जाती है।"
            ),
            locale(
                "ar-SA",
                summary = "عيّن قيمة إعداد واحدة.",
                help = "اكتب في التكوين العام أو الخاص بمساحة العمل الذي يحدده `layer` ثم تحقّق من التكوين المدمج. استخدم `dry_run=true` للمعاينة دون كتابة؛ عندها يُطلب إذن القراءة للملفين بدلًا من إذن الكتابة."
            ),
            locale(
                "pt-BR",
                summary = "Defina um valor de configuração.",
                help = "Grava na configuração global ou do espaço de trabalho escolhida por `layer` e valida a configuração combinada. Use `dry_run=true` para visualizar sem gravar; nesse caso, é solicitada permissão de leitura para os dois arquivos em vez de permissão de escrita."
            )
        ),
        tags(
            ToolTag::Mutate,
            ToolTag::Filesystem,
            settings_tag(),
            settings_write_tag()
        )
    )]
    async fn set(&self, input: SettingsSetToolInput) -> SdkResult<ToolInvokeOutput> {
        let layer = self.edit_layer(input.layer)?;
        let meta = self.config_meta().await?;
        let mut options = self.edit_options(input.dry_run, input.validate, input.reload)?;
        options.expected_revision = input.expected_revision.clone();
        let reload = options.reload;
        let response = set_layered_file_setting(
            meta.global_path.clone(),
            meta.workspace_path.clone(),
            layer,
            ConfigSettingsSetInput {
                path: input.path,
                value: input.value,
                options,
            },
        )
        .map_err(map_err)?;
        self.edit_output(
            "Settings updated",
            "Updated settings value.",
            response,
            reload,
            layer,
        )
        .await
    }

    #[tool(
        summary = "Delete one settings value.",
        help = "Deletes from the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing.",
        translations(
            locale(
                "zh-CN",
                summary = "删除一个设置值。",
                help = "从 `layer` 指定的全局或工作区配置中删除，并校验合并后的分层配置。使用 `dry_run=true` 可只预览不写入。"
            ),
            locale(
                "zh-TW",
                summary = "刪除一個設定值。",
                help = "從 `layer` 指定的全域或工作區設定中刪除，並驗證合併後的分層設定。使用 `dry_run=true` 可預覽而不寫入。"
            ),
            locale(
                "ja-JP",
                summary = "設定値を 1 つ削除します。",
                help = "`layer` で選んだグローバルまたはワークスペース設定から削除し、統合後の構成を検証します。`dry_run=true` なら書き込みなしで確認できます。"
            ),
            locale(
                "ko-KR",
                summary = "설정 값 하나를 삭제합니다.",
                help = "`layer`로 선택한 전역 또는 작업 공간 설정에서 삭제하고 병합된 구성을 검증합니다. `dry_run=true`로 쓰지 않고 미리 볼 수 있습니다."
            ),
            locale(
                "fr-FR",
                summary = "Supprimer une valeur de configuration.",
                help = "Supprime la valeur de la configuration globale ou d’espace de travail choisie par `layer`, puis valide l’ensemble fusionné. Utilisez `dry_run=true` pour prévisualiser sans écrire."
            ),
            locale(
                "de-DE",
                summary = "Einen Einstellungswert löschen.",
                help = "Löscht den Wert aus der mit `layer` gewählten globalen oder Workspace-Konfiguration und validiert die zusammengeführte Konfiguration. Mit `dry_run=true` können Sie die Änderung ohne Schreiben prüfen."
            ),
            locale(
                "es-ES",
                summary = "Elimina un valor de configuración.",
                help = "Elimina el valor de la configuración global o del espacio de trabajo indicada por `layer` y valida la configuración combinada. Usa `dry_run=true` para previsualizar sin escribir."
            ),
            locale(
                "hi-IN",
                summary = "एक सेटिंग मान हटाएँ।",
                help = "`layer` से चुने गए वैश्विक या कार्यक्षेत्र कॉन्फ़िगरेशन से हटाकर संयुक्त कॉन्फ़िगरेशन की जाँच करता है। `dry_run=true` बिना लिखे पूर्वावलोकन देता है।"
            ),
            locale(
                "ar-SA",
                summary = "احذف قيمة إعداد واحدة.",
                help = "احذف من التكوين العام أو تكوين مساحة العمل الذي يحدده `layer` ثم تحقّق من التكوين المدمج. استخدم `dry_run=true` للمعاينة دون كتابة."
            ),
            locale(
                "pt-BR",
                summary = "Exclua um valor de configuração.",
                help = "Exclui da configuração global ou do espaço de trabalho escolhida por `layer` e valida a configuração combinada. Use `dry_run=true` para visualizar sem gravar."
            )
        ),
        tags(
            ToolTag::Mutate,
            ToolTag::Filesystem,
            settings_tag(),
            settings_write_tag()
        )
    )]
    async fn delete(&self, input: SettingsDeleteToolInput) -> SdkResult<ToolInvokeOutput> {
        let layer = self.edit_layer(input.layer)?;
        let meta = self.config_meta().await?;
        let mut options = self.edit_options(input.dry_run, input.validate, input.reload)?;
        options.expected_revision = input.expected_revision.clone();
        let reload = options.reload;
        let response = delete_layered_file_setting(
            meta.global_path.clone(),
            meta.workspace_path.clone(),
            layer,
            ConfigSettingsDeleteInput {
                path: input.path,
                options,
            },
        )
        .map_err(map_err)?;
        self.edit_output(
            "Settings deleted",
            "Deleted settings value.",
            response,
            reload,
            layer,
        )
        .await
    }

    #[tool(
        summary = "Patch settings in agena.json.",
        help = "Deep-merges a JSON object into the global or workspace config selected by `layer`, then validates the combined layered configuration; null object entries delete keys. Use `dry_run=true` to preview without writing.",
        translations(
            locale(
                "zh-CN",
                summary = "以补丁方式更新 agena.json 设置。",
                help = "将 JSON 对象深度合并到 `layer` 指定的全局或工作区配置，再校验合并后的分层配置；对象项为 null 时会删除对应键。使用 `dry_run=true` 可只预览不写入。"
            ),
            locale(
                "zh-TW",
                summary = "以補丁方式更新 agena.json 設定。",
                help = "將 JSON 物件深度合併至 `layer` 指定的全域或工作區設定，再驗證合併後的分層設定；物件項目為 null 時會刪除對應索引鍵。使用 `dry_run=true` 可預覽而不寫入。"
            ),
            locale(
                "ja-JP",
                summary = "agena.json の設定にパッチを適用します。",
                help = "JSON オブジェクトを `layer` で選んだグローバルまたはワークスペース設定に深くマージし、統合後の構成を検証します。オブジェクト内の null はキーを削除します。`dry_run=true` なら書き込みなしで確認できます。"
            ),
            locale(
                "ko-KR",
                summary = "agena.json 설정에 패치를 적용합니다.",
                help = "JSON 객체를 `layer`로 선택한 전역 또는 작업 공간 설정에 깊게 병합한 뒤 통합 구성을 검증합니다. 객체 항목이 null이면 해당 키를 삭제합니다. `dry_run=true`로 쓰지 않고 미리 볼 수 있습니다."
            ),
            locale(
                "fr-FR",
                summary = "Appliquer un patch aux réglages de agena.json.",
                help = "Fusionne récursivement un objet JSON dans la configuration globale ou d’espace de travail choisie par `layer`, puis valide l’ensemble ; une valeur null dans un objet supprime la clé correspondante. Utilisez `dry_run=true` pour prévisualiser sans écrire."
            ),
            locale(
                "de-DE",
                summary = "Einstellungen in agena.json patchen.",
                help = "Führt ein JSON-Objekt tief in die mit `layer` gewählte globale oder Workspace-Konfiguration ein und validiert danach die Gesamtkonfiguration. Null-Einträge löschen den jeweiligen Schlüssel. Mit `dry_run=true` ist eine Vorschau ohne Schreiben möglich."
            ),
            locale(
                "es-ES",
                summary = "Aplica un parche a los ajustes de agena.json.",
                help = "Combina en profundidad un objeto JSON con la configuración global o del espacio de trabajo indicada por `layer` y valida el resultado conjunto. Las entradas null eliminan la clave correspondiente. Usa `dry_run=true` para previsualizar sin escribir."
            ),
            locale(
                "hi-IN",
                summary = "agena.json सेटिंग पर पैच लगाएँ।",
                help = "JSON ऑब्जेक्ट को `layer` से चुने गए वैश्विक या कार्यक्षेत्र कॉन्फ़िगरेशन में गहराई से मिलाकर संयुक्त कॉन्फ़िगरेशन जाँचता है। ऑब्जेक्ट में null संबंधित कुंजी हटाता है। `dry_run=true` बिना लिखे पूर्वावलोकन देता है।"
            ),
            locale(
                "ar-SA",
                summary = "طبّق تحديثًا على إعدادات agena.json.",
                help = "ادمج كائن JSON بعمق في التكوين العام أو تكوين مساحة العمل الذي يحدده `layer` ثم تحقّق من التكوين المدمج. تحذف قيمة null داخل الكائن المفتاح المقابل. استخدم `dry_run=true` للمعاينة دون كتابة."
            ),
            locale(
                "pt-BR",
                summary = "Aplique um patch às configurações de agena.json.",
                help = "Mescla profundamente um objeto JSON à configuração global ou do espaço de trabalho escolhida por `layer` e valida o conjunto. Entradas null em objetos excluem as chaves correspondentes. Use `dry_run=true` para visualizar sem gravar."
            )
        ),
        tags(
            ToolTag::Mutate,
            ToolTag::Filesystem,
            settings_tag(),
            settings_write_tag()
        )
    )]
    async fn patch(&self, input: SettingsPatchToolInput) -> SdkResult<ToolInvokeOutput> {
        let layer = self.edit_layer(input.layer)?;
        let meta = self.config_meta().await?;
        let mut options = self.edit_options(input.dry_run, input.validate, input.reload)?;
        options.expected_revision = input.expected_revision.clone();
        let reload = options.reload;
        let response = patch_layered_file_settings(
            meta.global_path.clone(),
            meta.workspace_path.clone(),
            layer,
            ConfigSettingsPatchInput {
                target: ConfigSettingsPathInput { path: input.path },
                changes: input.changes,
                options,
            },
        )
        .map_err(map_err)?;
        self.edit_output(
            "Settings patched",
            "Patched settings.",
            response,
            reload,
            layer,
        )
        .await
    }

    #[tool(
        summary = "Validate layered agena.json settings.",
        translations(
            locale("zh-CN", summary = "校验分层的 agena.json 设置。"),
            locale("zh-TW", summary = "驗證分層的 agena.json 設定。"),
            locale("ja-JP", summary = "階層化された agena.json 設定を検証します。"),
            locale("ko-KR", summary = "계층형 agena.json 설정을 검증합니다."),
            locale("fr-FR", summary = "Valider les réglages agena.json par couche."),
            locale("de-DE", summary = "Geschichtete agena.json-Einstellungen validieren."),
            locale("es-ES", summary = "Valida los ajustes por capas de agena.json."),
            locale("hi-IN", summary = "स्तरित agena.json सेटिंग की जाँच करें।"),
            locale("ar-SA", summary = "تحقّق من إعدادات agena.json متعددة الطبقات."),
            locale("pt-BR", summary = "Valide as configurações em camadas do agena.json.")
        ),
        tags(
            ToolTag::Query,
            ToolTag::Filesystem,
            settings_tag(),
            settings_read_tag()
        )
    )]
    async fn validate(&self, input: SettingsValidateToolInput) -> SdkResult<ToolInvokeOutput> {
        let layer = self.read_layer(input.layer)?;
        let meta = self.config_meta().await?;
        let response: ConfigSettingsValidateResponse = validate_layered_file_settings(
            meta.global_path.clone(),
            meta.workspace_path.clone(),
            layer,
        )
        .map_err(map_err)?;
        output_with_layer(
            "Settings valid",
            "Settings file is valid.",
            &response,
            Some(layer),
        )
    }

    async fn edit_output<T>(
        &self,
        title: &str,
        text: &str,
        response: T,
        reload: bool,
        layer: SettingsLayer,
    ) -> SdkResult<ToolInvokeOutput>
    where
        T: Serialize,
    {
        let mut payload =
            serde_json::to_value(&response).map_err(|err| PluginError::internal_error(&err))?;
        let needs_reload = payload
            .get("reload_required")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
            && reload;
        let dry_run = payload
            .get("dry_run")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        payload["committed"] = JsonValue::Bool(!dry_run);
        let mut text = if needs_reload {
            match self.host()?.request_config_reload().await {
                Ok(request) => {
                    insert_reload_request(&mut payload, request);
                    payload["activation_state"] = JsonValue::String("queued".into());
                    format!(
                        "{text} Configuration saved; runtime reload queued, not yet confirmed active."
                    )
                }
                Err(error) => {
                    payload["activation_state"] = JsonValue::String("request_failed".into());
                    payload["activation_warning"] =
                        JsonValue::String(error.failure.user.fallback.clone());
                    format!(
                        "{text} Configuration was saved, but reload request failed: {}. Do not repeat the file edit; retry activation separately.",
                        error.failure.user.fallback
                    )
                }
            }
        } else {
            payload["activation_state"] = JsonValue::String(
                if dry_run {
                    "not_committed"
                } else {
                    "not_requested"
                }
                .into(),
            );
            text.to_owned()
        };
        if let Some(revision) = payload.get("after_revision").and_then(JsonValue::as_str) {
            text.push_str(&format!("\nResult document revision: {revision}"));
        }
        insert_settings_layer(&mut payload, layer);
        redact_settings_payload(&mut payload);
        Ok(ToolInvokeOutput::from_parts(
            title,
            text.clone(),
            text,
            Some(payload),
            std::collections::BTreeMap::from([("agena.effect".to_string(), "config".to_string())]),
            Vec::new(),
        ))
    }
}

fn settings_tag() -> ToolTag {
    ToolTag::custom("settings").expect("settings tag is valid")
}

fn settings_read_tag() -> ToolTag {
    ToolTag::custom("settings_read").expect("settings_read tag is valid")
}

fn settings_write_tag() -> ToolTag {
    ToolTag::custom("settings_write").expect("settings_write tag is valid")
}

fn required_meta_path(meta: &JsonValue, field: &str) -> SdkResult<PathBuf> {
    meta.get(field)
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| PluginError::internal(format!("host config meta is missing {field}")))
}

fn inspect_file_value(
    meta: &SettingsConfigMeta,
    layer: SettingsLayer,
    path: Option<String>,
) -> SdkResult<SettingsInspectFileValue> {
    let (config_path, config_found) = meta.file(layer);
    let response = read_file_setting(
        config_path.clone(),
        ConfigSettingsGetInput {
            target: ConfigSettingsPathInput { path: path.clone() },
            source: ConfigSettingsSource::File,
        },
    )
    .map_err(map_err)?;
    let defined = match path.as_deref() {
        Some(_) => !response.value.is_null(),
        None => config_found,
    };
    Ok(SettingsInspectFileValue {
        layer,
        config_path: config_path.clone(),
        config_found,
        path,
        defined,
        value: response.value,
    })
}

fn effective_host_path(path: Option<&str>) -> Option<String> {
    match path.map(str::trim).filter(|path| !path.is_empty()) {
        None => Some("config".to_string()),
        Some(path) if path == "config" || path == "meta" => Some(path.to_string()),
        Some(path) if path.starts_with("config.") || path.starts_with("meta.") => {
            Some(path.to_string())
        }
        Some(path) => Some(format!("config.{path}")),
    }
}

fn resolve_effective_settings_path(
    scope: Option<SettingsScope>,
    path: Option<&str>,
) -> SdkResult<Option<String>> {
    match scope {
        None => Ok(effective_host_path(path)),
        Some(scope) => {
            let trimmed = path.map(str::trim).filter(|value| !value.is_empty());
            if let Some(path) = trimmed
                && (path == "config"
                    || path == "meta"
                    || path.starts_with("config.")
                    || path.starts_with("meta."))
            {
                return Err(PluginError::invalid_params(
                    "explicit settings scope expects a relative path without `config.` or `meta.` prefix",
                ));
            }
            let root = match scope {
                SettingsScope::Config => "config",
                SettingsScope::Meta => "meta",
            };
            Ok(match trimmed {
                None => Some(root.to_string()),
                Some(path) => Some(format!("{root}.{path}")),
            })
        }
    }
}

fn insert_reload_request(payload: &mut JsonValue, request: HostConfigReloadRequestResponse) {
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "reload_task".to_string(),
            serde_json::json!({
                "task_id": request.task_id,
                "started": request.started,
            }),
        );
    }
}

fn insert_settings_layer(payload: &mut JsonValue, layer: SettingsLayer) {
    if let Some(object) = payload.as_object_mut()
        && let Ok(layer) = serde_json::to_value(layer)
    {
        object.insert("layer".to_string(), layer);
    }
}

fn output_with_layer<T>(
    title: &str,
    text: impl Into<String>,
    payload: &T,
    layer: Option<SettingsLayer>,
) -> SdkResult<ToolInvokeOutput>
where
    T: Serialize,
{
    let mut text = text.into();
    let mut payload =
        serde_json::to_value(payload).map_err(|err| PluginError::internal_error(&err))?;
    if let Some(layer) = layer {
        insert_settings_layer(&mut payload, layer);
    }
    if let Some(revision) = payload.get("revision").and_then(JsonValue::as_str) {
        text.push_str(&format!("\nPersisted document revision: {revision}"));
    }
    redact_settings_payload(&mut payload);
    Ok(ToolInvokeOutput::from_parts(
        title,
        text.clone(),
        text,
        Some(payload),
        std::collections::BTreeMap::from([("agena.effect".to_string(), "settings".to_string())]),
        Vec::new(),
    ))
}

fn output<T>(title: &str, text: impl Into<String>, payload: &T) -> SdkResult<ToolInvokeOutput>
where
    T: Serialize,
{
    output_with_layer(title, text, payload, None)
}

fn redact_settings_payload(value: &mut JsonValue) {
    redact_settings_value(value, None, false);
}

fn redact_settings_value(value: &mut JsonValue, key_hint: Option<&str>, secret_context: bool) {
    let secret_context = secret_context
        || is_inline_secret_source(value)
        || (key_hint.is_some_and(is_sensitive_container_key)
            && !is_environment_secret_source(value));
    match value {
        JsonValue::Object(object) => {
            let path_hint = object
                .get("path")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned);
            for (key, child) in object.iter_mut() {
                if path_hint.as_deref().is_some_and(|_| {
                    matches!(key.as_str(), "value" | "previous" | "current" | "effective")
                }) {
                    redact_settings_value_for_path(
                        child,
                        path_hint.as_deref().expect("checked above"),
                    );
                    continue;
                }
                let child_context = secret_context
                    || is_inline_secret_source(child)
                    || (is_sensitive_container_key(key) && !is_environment_secret_source(child));
                redact_settings_value(child, Some(key.as_str()), child_context);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                redact_settings_value(item, key_hint, secret_context);
            }
        }
        JsonValue::Null => {}
        _ if is_sensitive_scalar_key(key_hint, secret_context) => {
            *value = JsonValue::String("<redacted>".to_string());
        }
        _ => {}
    }
}

fn redact_settings_value_for_path(value: &mut JsonValue, path: &str) {
    let Ok(segments) = crate::config::parse_settings_path(path) else {
        redact_settings_value(value, None, false);
        return;
    };
    let key_hint = segments.last().map(String::as_str);
    let parent_segments = &segments[..segments.len().saturating_sub(1)];
    let secret_context = parent_segments
        .iter()
        .any(|segment| is_sensitive_container_key(segment))
        || (key_hint.is_some_and(|key| normalize_sensitive_key(key) == "value")
            && parent_segments
                .iter()
                .any(|segment| normalize_sensitive_key(segment) == "source")
            && parent_segments.iter().any(|segment| {
                matches!(
                    normalize_sensitive_key(segment).as_str(),
                    "auth" | "access" | "api_key"
                )
            }));
    redact_settings_value(value, key_hint, secret_context);
}

fn is_sensitive_scalar_key(key: Option<&str>, secret_context: bool) -> bool {
    let Some(key) = key else {
        return false;
    };
    let normalized = normalize_sensitive_key(key);
    let compact = normalized.replace('_', "");
    is_sensitive_container_key(&normalized)
        || compact == "accesskeyid"
        || normalized == "authorization"
        || normalized == "proxy_authorization"
        || normalized == "token"
        || normalized.ends_with("_token")
        || normalized == "secret"
        || normalized.ends_with("_secret")
        || normalized.starts_with("secret_")
        || normalized == "password"
        || normalized.ends_with("_password")
        || normalized.contains("cookie")
        || normalized.contains("signature")
        || compact.contains("privatekey")
        || (secret_context
            && matches!(
                normalized.as_str(),
                "value" | "key" | "access" | "refresh" | "data"
            ))
}

fn is_sensitive_container_key(key: &str) -> bool {
    let normalized = normalize_sensitive_key(key);
    let compact = normalized.replace('_', "");
    matches!(compact.as_str(), "apikey" | "xapikey")
        || normalized == "credential"
        || normalized.ends_with("_credential")
}

fn normalize_sensitive_key(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace(['-', '.'], "_")
}

fn is_environment_secret_source(value: &JsonValue) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(JsonValue::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("env"))
}

fn is_inline_secret_source(value: &JsonValue) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(JsonValue::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("inline"))
        && value.get("value").is_some()
}

fn map_err(error: ConfigError) -> PluginError {
    match error {
        ConfigError::Validation(detail) => PluginError::invalid_params_with_public_detail(
            format!("config validation failed: {detail}"),
            format!("Invalid settings: {detail}"),
        ),
        ConfigError::ParseFile { path, source } => PluginError::invalid_params_with_public_detail(
            format!("failed to parse config file {}: {source}", path.display()),
            format!("Invalid settings JSON: {source}"),
        ),
        other => PluginError::internal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::ToolPermissionConfig;
    use crate::permission::ToolPermissionPolicy;
    use agena_domain::PermissionDecision;
    use agena_domain::PermissionMode;
    use agena_plugin_host::sdk::Plugin;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn settings_parse_errors_expose_schema_detail_without_private_config_path() {
        #[allow(dead_code)]
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Root {
            compaction: Option<bool>,
        }

        let source = serde_json::from_str::<Root>(r#"{"tool_trial":true}"#)
            .expect_err("unknown field must fail");
        let error = map_err(ConfigError::ParseFile {
            path: PathBuf::from("/Users/private/agena/agena.json"),
            source,
        });
        assert!(
            error
                .failure
                .user
                .fallback
                .contains("unknown field `tool_trial`")
        );
        assert!(error.failure.user.fallback.contains("compaction"));
        assert!(!error.failure.user.fallback.contains("/Users"));
        assert!(error.diagnostic_message().contains("/Users/private"));
    }

    #[test]
    fn manifest_exposes_layered_settings_tools_and_minimal_read_capabilities() {
        let manifest = SettingsPlugin::new().manifest();
        let names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "get", "list", "inspect", "set", "delete", "patch", "validate"
            ]
        );

        for name in ["get", "list", "inspect", "validate"] {
            let tool = manifest
                .tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("settings read tool exists");
            assert!(tool.tags.contains(&settings_read_tag()));
            assert!(tool.tags.contains(&ToolTag::Filesystem));
            assert!(!tool.tags.contains(&settings_write_tag()));
        }

        let set = manifest
            .tools
            .iter()
            .find(|tool| tool.name == "set")
            .expect("settings set tool exists");
        assert!(set.tags.contains(&settings_write_tag()));
    }

    #[test]
    fn settings_exact_names_use_the_existing_tool_policy() {
        let config = ToolPermissionConfig {
            default: Some(PermissionMode::Auto),
            names: BTreeMap::from([
                ("agena.settings.inspect".to_string(), PermissionMode::Allow),
                ("agena.settings.patch".to_string(), PermissionMode::Deny),
                ("agena.settings.set".to_string(), PermissionMode::Allow),
            ]),
            ..Default::default()
        };
        let policy = crate::authorization::apply_tool_permission_config(
            &config,
            ToolPermissionPolicy::new(PermissionMode::Auto),
        )
        .expect("valid settings policy");
        let tags: &[agena_plugin_host::sdk::ToolTag] = &[];

        assert!(matches!(
            policy.check_tool("agena.settings.inspect", None, tags),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            policy.check_tool("agena.settings.patch", None, tags),
            PermissionDecision::Deny { .. }
        ));
        assert!(matches!(
            policy.check_tool("agena.settings.set", None, tags),
            PermissionDecision::Allow
        ));
    }

    #[test]
    fn settings_payloads_redact_secrets_without_hiding_env_references() {
        let mut payload = json!({
            "path": "providers.openai.auth.api_key.value",
            "value": "previous-secret",
            "current": "current-secret",
            "config": {
                "tracing": { "filter": "info" },
                "providers": {
                    "openai": {
                        "auth": {
                            "api_key": {"kind": "inline", "value": "inline-secret"},
                            "credential": {
                                "type": "oauth",
                                "access": "access-secret",
                                "refresh": "refresh-secret",
                                "account_id": "acct-1"
                            }
                        }
                    },
                    "anthropic": {
                        "auth": {
                            "api_key": {"kind": "env", "value": "ANTHROPIC_API_KEY"}
                        }
                    },
                    "gitlab": {
                        "auth": {
                            "access": {
                                "kind": "api_key",
                                "source": {"kind": "inline", "value": "gitlab-secret"}
                            }
                        }
                    },
                    "bedrock": {
                        "auth": {
                            "accessKeyId": "AKIAEXAMPLE",
                            "secret_access_key": "bedrock-secret"
                        }
                    }
                }
            }
        });
        redact_settings_payload(&mut payload);

        assert_eq!(payload["value"], "<redacted>");
        assert_eq!(payload["current"], "<redacted>");
        assert_eq!(payload["config"]["tracing"]["filter"], "info");
        assert_eq!(
            payload["config"]["providers"]["openai"]["auth"]["api_key"]["value"],
            "<redacted>"
        );
        assert_eq!(
            payload["config"]["providers"]["openai"]["auth"]["credential"]["access"],
            "<redacted>"
        );
        assert_eq!(
            payload["config"]["providers"]["openai"]["auth"]["credential"]["account_id"],
            "acct-1"
        );
        assert_eq!(
            payload["config"]["providers"]["anthropic"]["auth"]["api_key"]["value"],
            "ANTHROPIC_API_KEY"
        );
        assert_eq!(
            payload["config"]["providers"]["gitlab"]["auth"]["access"]["source"]["value"],
            "<redacted>"
        );
        assert_eq!(
            payload["config"]["providers"]["bedrock"]["auth"]["accessKeyId"],
            "<redacted>"
        );

        let mut listed_secret = json!({
            "path": "providers.gitlab.auth.access.source.value",
            "kind": "string",
            "value": "gitlab-secret"
        });
        redact_settings_payload(&mut listed_secret);
        assert_eq!(listed_secret["value"], "<redacted>");
    }

    #[test]
    fn effective_paths_support_explicit_scopes_and_reject_mixed_forms() {
        assert_eq!(
            resolve_effective_settings_path(Some(SettingsScope::Config), Some("tracing.filter"))
                .expect("relative config path")
                .as_deref(),
            Some("config.tracing.filter")
        );
        assert_eq!(
            resolve_effective_settings_path(Some(SettingsScope::Meta), Some("config_path"))
                .expect("relative meta path")
                .as_deref(),
            Some("meta.config_path")
        );
        assert!(
            resolve_effective_settings_path(Some(SettingsScope::Config), Some("config.runtime"))
                .is_err()
        );
        assert!(
            resolve_effective_settings_path(Some(SettingsScope::Config), Some("meta.config_path"))
                .is_err()
        );
    }
}

#[cfg(test)]
mod commit_tests {
    use super::*;
    #[tokio::test]
    async fn failed_reload_after_save_preserves_commit_fact() {
        let plugin = SettingsPlugin::new();
        *plugin.host.write().unwrap() = Some(Arc::new(agena_plugin_host::sdk::NoopHostClient));
        let output=plugin.edit_output("Settings updated","Saved.",serde_json::json!({"changed":true,"dry_run":false,"reload_required":true,"after_revision":"revision"}),true,SettingsLayer::Workspace).await.unwrap();
        let payload = output.payload.unwrap();
        assert_eq!(payload["committed"], true);
        assert_eq!(payload["activation_state"], "request_failed");
        assert!(output.output_text.contains("Do not repeat"));
        assert!(
            output
                .output_text
                .contains("Result document revision: revision")
        );
    }
}

#[cfg(test)]
mod revision_text_tests {
    use super::*;
    #[test]
    fn read_revision_is_available_to_text_only_clients() {
        let output = output_with_layer(
            "Settings read",
            "Value",
            &serde_json::json!({"revision":"read-version"}),
            Some(SettingsLayer::Workspace),
        )
        .unwrap();
        assert!(
            output
                .output_text
                .contains("Persisted document revision: read-version")
        );
    }
}
