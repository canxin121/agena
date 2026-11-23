//! Source-resolved companion policy; persistence, identities and timing belong to Runtime.
use agena_plugin_host::sdk::host_api::{CompanionRequest, HostClient};
use agena_plugin_host::sdk::{
    ChatSystemTransformInput, ChatSystemTransformPatch, InitContext, InitOutcome, PluginError,
    Result, ToolInvokeOutput,
};
use std::sync::{Arc, OnceLock};

pub(crate) struct CompanionPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
}
#[agena_plugin_host::sdk::agena_plugin(namespace="agena",name="companion",version=env!("CARGO_PKG_VERSION"),
    display_name = "Companion",
    icon = crate::plugin_metadata::icon(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/plugin-icons/companion.svg"))),
    authors = ["canxin121"],
    help = "Manage memory and reminder policy and schedule reminders requested by the user.",
    tags(agena_plugin_host::sdk::ToolTag::Interactive, agena_plugin_host::sdk::ToolTag::Custom("assistant".to_owned())),
    summary = "Memory settings and reminders.",
    translations(
        locale("zh-CN", summary = "记忆设置和提醒。", help = "管理记忆和提醒设置，安排用户明确要求的提醒。"),
        locale("zh-TW", summary = "記憶設定和提醒。"),
        locale("ja-JP", summary = "メモリ設定とリマインダー。"),
        locale("ko-KR", summary = "메모리 설정과 알림."),
        locale("fr-FR", summary = "Paramètres de mémoire et rappels."),
        locale("de-DE", summary = "Speichereinstellungen und Erinnerungen."),
        locale("es-ES", summary = "Configuración de memoria y recordatorios."),
        locale("pt-BR", summary = "Configurações de memória e lembretes."),
        locale("hi-IN", summary = "मेमोरी सेटिंग और अनुस्मारक।"),
        locale("ar-SA", summary = "إعدادات الذاكرة والتذكيرات.")
    )
)]
impl CompanionPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: OnceLock::new(),
        }
    }
    #[hook(
        init,
        summary = "Connect reminder services",
        help = "Connects to the host for memory policy and reminder scheduling.",
        translations(
            locale(
                "zh-CN",
                summary = "连接提醒服务",
                help = "连接宿主以管理记忆设置和安排提醒。"
            ),
            locale(
                "zh-TW",
                summary = "連接提醒服務",
                help = "連接宿主以管理記憶設定及安排提醒。"
            ),
            locale(
                "ja-JP",
                summary = "リマインダーサービスに接続",
                help = "ホストに接続してメモリ設定とリマインダーを管理します。"
            ),
            locale(
                "ko-KR",
                summary = "알림 서비스 연결",
                help = "호스트에 연결하여 메모리 설정과 알림 일정을 관리합니다."
            ),
            locale(
                "fr-FR",
                summary = "Connecter le service de rappels",
                help = "Se connecte à l’hôte pour gérer la mémoire et programmer les rappels."
            ),
            locale(
                "de-DE",
                summary = "Erinnerungsdienst verbinden",
                help = "Verbindet den Host für Speichereinstellungen und geplante Erinnerungen."
            ),
            locale(
                "es-ES",
                summary = "Conectar el servicio de recordatorios",
                help = "Se conecta al host para gestionar la memoria y programar recordatorios."
            ),
            locale(
                "pt-BR",
                summary = "Conectar o serviço de lembretes",
                help = "Conecta-se ao host para gerenciar a memória e agendar lembretes."
            ),
            locale(
                "hi-IN",
                summary = "अनुस्मारक सेवा से जुड़ें",
                help = "मेमोरी सेटिंग संभालने और अनुस्मारक तय करने के लिए होस्ट से जुड़ता है।"
            ),
            locale(
                "ar-SA",
                summary = "ربط خدمة التذكيرات",
                help = "يتصل بالمضيف لإدارة إعدادات الذاكرة وجدولة التذكيرات."
            )
        )
    )]
    async fn init(&self, _context: InitContext, host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("assistant already initialized"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }
    #[hook(
        chat.system,
        summary = "Attach reminder guidance",
        help = "Explains how to schedule and deliver reminders requested by the user.",
        translations(
            locale("zh-CN", summary = "加入提醒指引", help = "说明如何安排和发送用户要求的提醒。"),
            locale("zh-TW", summary = "加入提醒指引", help = "說明如何安排及傳送使用者要求的提醒。"),
            locale("ja-JP", summary = "リマインダーの案内を追加", help = "ユーザーが依頼したリマインダーの設定と配信方法を説明します。"),
            locale("ko-KR", summary = "알림 안내 추가", help = "사용자가 요청한 알림의 예약 및 전달 방법을 설명합니다."),
            locale("fr-FR", summary = "Ajouter les consignes de rappel", help = "Explique comment programmer et envoyer les rappels demandés."),
            locale("de-DE", summary = "Anleitung für Erinnerungen ergänzen", help = "Erklärt, wie angeforderte Erinnerungen geplant und zugestellt werden."),
            locale("es-ES", summary = "Añadir instrucciones de recordatorios", help = "Explica cómo programar y enviar los recordatorios solicitados."),
            locale("pt-BR", summary = "Adicionar instruções de lembretes", help = "Explica como agendar e enviar os lembretes solicitados."),
            locale("hi-IN", summary = "अनुस्मारक के निर्देश जोड़ें", help = "उपयोगकर्ता के अनुरोध पर अनुस्मारक तय करने और भेजने का तरीका बताता है।"),
            locale("ar-SA", summary = "إضافة إرشادات التذكيرات", help = "يوضح كيفية جدولة التذكيرات التي طلبها المستخدم وإرسالها.")
        )
    )]
    async fn system(
        &self,
        _input: ChatSystemTransformInput,
    ) -> Result<Option<ChatSystemTransformPatch>> {
        let Some(host) = self.host.get() else {
            return Ok(None);
        };
        let Ok(context) = host.companion_context().await else {
            return Ok(None);
        };
        if context.get("enabled").and_then(serde_json::Value::as_bool) != Some(true) {
            return Ok(None);
        }
        Ok(Some(ChatSystemTransformPatch {
            append: Some(include_str!("../../../assets/prompts/companion.md").to_owned()),
            ..Default::default()
        }))
    }
    #[tool(
        tags(mutate, agent),
        summary = "Manage memory and reminder policy and finite reminders. Use profile to read the current policy, revision and time. In project conversations, configure updates memory and reminder policy. Remind needs an explicit user request and a due time.",
        translations(
            locale("zh-CN", summary = "管理工作区的记忆设置和主动提醒。"),
            locale("zh-TW", summary = "管理工作區的記憶設定和主動提醒。"),
            locale("ja-JP", summary = "アシスタント設定と通知を管理します。"),
            locale("ko-KR", summary = "도우미 설정과 알림을 관리합니다."),
            locale("fr-FR", summary = "Gérer les préférences et les rappels."),
            locale("de-DE", summary = "Einstellungen und Hinweise verwalten."),
            locale("es-ES", summary = "Gestionar preferencias y recordatorios."),
            locale("pt-BR", summary = "Gerenciar preferências e lembretes."),
            locale("hi-IN", summary = "सहायक की प्राथमिकताएँ और अनुस्मारक प्रबंधित करें।"),
            locale("ar-SA", summary = "إدارة تفضيلات المساعد والتذكيرات.")
        )
    )]
    async fn manage(&self, input: &CompanionRequest) -> Result<ToolInvokeOutput> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("assistant service unavailable"))?;
        let value = host.companion_operation(input.clone()).await?;
        let mut output = ToolInvokeOutput::text(serde_json::to_string(&value)?);
        output.payload = Some(value);
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_system_prompt_does_not_repeat_the_tool_input_schema() {
        let prompt = include_str!("../../../assets/prompts/companion.md");
        for schema_marker in ["$defs", "additionalProperties", "x-agena-order"] {
            assert!(!prompt.contains(schema_marker));
        }
    }
}
