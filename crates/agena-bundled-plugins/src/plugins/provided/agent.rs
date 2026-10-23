//! Thin tool adapter. Agent identity, delivery, supervision and recovery belong to the runtime.
use agena_plugin_host::sdk::host_api::{
    AgentMessageRequest, AgentSpawnRequest, AgentTargetRequest, AgentWaitRequest, HostClient,
};
use agena_plugin_host::sdk::{InitContext, InitOutcome, PluginError, Result, ToolInvokeOutput};
use std::sync::{Arc, OnceLock};

pub(crate) const AGENT_PLUGIN_ID: &str = "agena.agent";
pub(crate) struct AgentPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena", name = "agent", version = env!("CARGO_PKG_VERSION"),
    summary = "Agent communication and delegation.",
    translations(
            locale("zh-CN", summary = "会话间的 agent 协作工具。"),
            locale("zh-TW", summary = "會話間的 agent 協作工具。"),
            locale("ja-JP", summary = "エージェント間の連携ツール。"),
            locale("ko-KR", summary = "에이전트 간 협업 도구."),
            locale("fr-FR", summary = "Coordination entre agents."),
            locale("de-DE", summary = "Zusammenarbeit zwischen Agenten."),
            locale("es-ES", summary = "Coordinación entre agentes."),
            locale("hi-IN", summary = "एजेंट के बीच समन्वय।"),
            locale("ar-SA", summary = "التنسيق بين الوكلاء."),
            locale("pt-BR", summary = "Coordenação entre agentes.")
        )
)]
impl AgentPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: OnceLock::new(),
        }
    }
    #[hook(init)]
    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("agent plugin already initialized"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }
    #[tool(
        tags(agent),
        summary = "Create an independent child and return its handle immediately. Its final reply is delivered automatically. Parent history is not copied; include scope and constraints in prompt.",
        translations(
            locale(
                "zh-CN",
                summary = "创建独立 agent，立即返回其标识；完成结果自动送达。"
            ),
            locale("zh-TW", summary = "建立獨立 agent，立即回傳識別碼；結果自動送達。"),
            locale(
                "ja-JP",
                summary = "独立したエージェントを起動し、識別子を返します。結果は自動通知されます。"
            ),
            locale(
                "ko-KR",
                summary = "독립 에이전트를 시작하고 식별자를 반환합니다. 결과는 자동 전달됩니다."
            ),
            locale(
                "fr-FR",
                summary = "Démarrer un agent indépendant ; son résultat est transmis automatiquement."
            ),
            locale(
                "de-DE",
                summary = "Einen unabhängigen Agenten starten; Ergebnisse werden automatisch zugestellt."
            ),
            locale(
                "es-ES",
                summary = "Inicia un agente independiente; su resultado se entrega automáticamente."
            ),
            locale("hi-IN", summary = "स्वतंत्र एजेंट शुरू करें; परिणाम अपने आप आता है।"),
            locale("ar-SA", summary = "ابدأ وكيلاً مستقلاً؛ تُرسل نتيجته تلقائياً."),
            locale(
                "pt-BR",
                summary = "Inicie um agente independente; o resultado é entregue automaticamente."
            )
        )
    )]
    async fn spawn(&self, input: &AgentSpawnRequest) -> Result<ToolInvokeOutput> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("agent plugin is not initialized"))?;
        let response = host.spawn_agent(input.clone()).await?;
        let value = serde_json::to_value(&response)?;
        let mut output = ToolInvokeOutput::text(serde_json::to_string(&value)?);
        output.payload = Some(value);
        Ok(output)
    }
    #[tool(
        tags(agent),
        summary = "Queue a message to a visible agent. Does not start idle recipients or pause the sender. Use agent.wait when you need a reply.",
        translations(
            locale("zh-CN", summary = "向可见 agent 发送消息；不启动空闲会话。"),
            locale("zh-TW", summary = "傳送訊息給可見 agent；不啟動閒置會話。"),
            locale("ja-JP", summary = "表示されたエージェントにメッセージを送信します。"),
            locale("ko-KR", summary = "보이는 에이전트에게 메시지를 보냅니다."),
            locale(
                "fr-FR",
                summary = "Envoyer un message à un agent visible sans le démarrer."
            ),
            locale(
                "de-DE",
                summary = "Eine Nachricht an einen sichtbaren Agenten senden, ohne ihn zu starten."
            ),
            locale(
                "es-ES",
                summary = "Envía un mensaje a un agente visible sin iniciarlo."
            ),
            locale("hi-IN", summary = "दिखने वाले एजेंट को संदेश भेजें, बिना उसे शुरू किए।"),
            locale("ar-SA", summary = "أرسل رسالة لوكيل ظاهر دون تشغيله."),
            locale(
                "pt-BR",
                summary = "Envie uma mensagem a um agente visível sem iniciá-lo."
            )
        )
    )]
    async fn message(&self, input: &AgentMessageRequest) -> Result<ToolInvokeOutput> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("agent plugin is not initialized"))?;
        let response = host.message_agent(input.clone()).await?;
        let value = serde_json::to_value(&response)?;
        let mut output = ToolInvokeOutput::text(serde_json::to_string(&value)?);
        output.payload = Some(value);
        Ok(output)
    }
    #[tool(
        tags(agent),
        summary = "Wait on your own mailbox for messages, new user input or timeout. Returns only a wake reason; messages enter context automatically. Do not poll.",
        translations(
            locale(
                "zh-CN",
                summary = "等待本会话收到消息、新用户输入或超时，仅返回唤醒原因。"
            ),
            locale("zh-TW", summary = "等待本會話訊息、新輸入或逾時，只回傳喚醒原因。"),
            locale(
                "ja-JP",
                summary = "メッセージ、新規入力、タイムアウトまで待機します。"
            ),
            locale("ko-KR", summary = "메시지, 새 입력 또는 시간 초과를 기다립니다."),
            locale(
                "fr-FR",
                summary = "Attendre un message, une nouvelle saisie ou le délai."
            ),
            locale(
                "de-DE",
                summary = "Auf eine Nachricht, neue Eingaben oder das Zeitlimit warten."
            ),
            locale("es-ES", summary = "Espera un mensaje, una entrada nueva o el plazo."),
            locale("hi-IN", summary = "संदेश, नया इनपुट या समय सीमा तक प्रतीक्षा करें।"),
            locale("ar-SA", summary = "انتظر رسالة أو إدخالاً جديداً أو انتهاء المهلة."),
            locale("pt-BR", summary = "Aguarde mensagem, nova entrada ou o prazo.")
        )
    )]
    async fn wait(&self, input: &AgentWaitRequest) -> Result<ToolInvokeOutput> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("agent plugin is not initialized"))?;
        let response = host.wait_agent(input.clone()).await?;
        let value = serde_json::to_value(&response)?;
        let mut output = ToolInvokeOutput::text(serde_json::to_string(&value)?);
        output.payload = Some(value);
        Ok(output)
    }
    #[tool(
        tags(agent),
        summary = "Give a direct child more work. Queues guidance while running; starts another execution in the same session when idle.",
        translations(
            locale(
                "zh-CN",
                summary = "向直属子 agent 补充指导；空闲时启动新一轮，运行时排队送达。"
            ),
            locale("zh-TW", summary = "向直屬子 agent 補充指示；閒置時啟動，執行時排隊。"),
            locale(
                "ja-JP",
                summary = "子エージェントに指示を送り、待機中なら再開します。"
            ),
            locale(
                "ko-KR",
                summary = "자식 에이전트에 지시를 보내고 유휴 상태면 재개합니다."
            ),
            locale(
                "fr-FR",
                summary = "Envoyer des instructions à un enfant et le relancer s’il est inactif."
            ),
            locale(
                "de-DE",
                summary = "Einem Kind Anweisungen senden und es bei Bedarf starten."
            ),
            locale(
                "es-ES",
                summary = "Envía instrucciones a un hijo y lo inicia si está inactivo."
            ),
            locale("hi-IN", summary = "अपने चाइल्ड को निर्देश दें; निष्क्रिय हो तो शुरू करें।"),
            locale("ar-SA", summary = "أرسل تعليمات للوكيل الابن وشغّله إن كان خاملاً."),
            locale(
                "pt-BR",
                summary = "Envie instruções a um filho e inicie-o se estiver ocioso."
            )
        )
    )]
    async fn followup(&self, input: &AgentMessageRequest) -> Result<ToolInvokeOutput> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("agent plugin is not initialized"))?;
        let response = host.followup_agent(input.clone()).await?;
        let value = serde_json::to_value(&response)?;
        let mut output = ToolInvokeOutput::text(serde_json::to_string(&value)?);
        output.payload = Some(value);
        Ok(output)
    }
    #[tool(
        tags(agent),
        summary = "Stop a direct child execution. Preserves the agent session for agent.followup.",
        translations(
            locale(
                "zh-CN",
                summary = "停止直属子 agent 的当前执行，保留会话以便后续继续。"
            ),
            locale("zh-TW", summary = "停止直屬子 agent 的執行，保留會話。"),
            locale(
                "ja-JP",
                summary = "子エージェントの実行を停止し、セッションを保持します。"
            ),
            locale("ko-KR", summary = "자식 에이전트의 실행을 멈추고 세션을 보존합니다."),
            locale(
                "fr-FR",
                summary = "Arrêter l’exécution d’un enfant en conservant sa session."
            ),
            locale(
                "de-DE",
                summary = "Die Ausführung eines Kindes stoppen und seine Sitzung behalten."
            ),
            locale(
                "es-ES",
                summary = "Detiene la ejecución de un hijo y conserva su sesión."
            ),
            locale("hi-IN", summary = "चाइल्ड का निष्पादन रोकें और सत्र बचाएँ।"),
            locale("ar-SA", summary = "أوقف تنفيذ الوكيل الابن مع الاحتفاظ بجلسته."),
            locale("pt-BR", summary = "Pare a execução de um filho preservando a sessão.")
        )
    )]
    async fn cancel(&self, input: &AgentTargetRequest) -> Result<ToolInvokeOutput> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("agent plugin is not initialized"))?;
        let response = host.cancel_agent(input.clone()).await?;
        let value = serde_json::to_value(&response)?;
        let mut output = ToolInvokeOutput::text(serde_json::to_string(&value)?);
        output.payload = Some(value);
        Ok(output)
    }
}
