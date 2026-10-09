mod rewrite;
use rewrite::{CodeRewriteInput, invoke_rewrite};

use std::path::Path;

use agena_macros::ToolInput;
use agena_tool::code_search::{
    CodeLanguage, CodeSearchError, StructuralSearchRequest, SyntaxTreeRequest,
    format_search_output, search_ast, syntax_tree,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};

pub(crate) const CODE_PLUGIN_ID: &str = "agena.code";

pub(crate) struct CodePlugin;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(exactly_one_of("pattern", "rule"), non_empty_if_present("pattern"))]
struct CodeSearchAstInput {
    #[arg(trim, non_empty)]
    path: String,
    /// Simple ast-grep pattern; provide exactly one of pattern or rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 16384)]
    pattern: Option<String>,
    /// Structured ast-grep rule object (kind, pattern, all/any/not, inside/has, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rule: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<CodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(minimum = 1, maximum = 100)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct CodeSyntaxTreeInput {
    #[arg(trim, non_empty)]
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<CodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(minimum = 1, maximum = 6)]
    max_depth: Option<u8>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "code",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Structured code search and syntax inspection tools.",
    translations(
        locale("zh-CN", summary = "结构化代码搜索和语法检查工具。"),
        locale("zh-TW", summary = "結構化程式碼搜尋與語法檢視工具。"),
        locale("ja-JP", summary = "構造を指定したコード検索と構文の確認に使うツールです。"),
        locale("ko-KR", summary = "구조 기반 코드 검색과 구문 검사 도구입니다."),
        locale("fr-FR", summary = "Outils de recherche structurelle dans le code et d’inspection syntaxique."),
        locale("de-DE", summary = "Werkzeuge für strukturierte Codesuche und Syntaxprüfung."),
        locale("es-ES", summary = "Herramientas de búsqueda estructural e inspección sintáctica del código."),
        locale("hi-IN", summary = "कोड में संरचनात्मक खोज और सिंटैक्स जाँच के टूल।"),
        locale("ar-SA", summary = "أدوات للبحث البنيوي في الشيفرة وفحص صياغتها."),
        locale("pt-BR", summary = "Ferramentas de busca estrutural e inspeção de sintaxe no código.")
    ),
)]
impl CodePlugin {
    #[tool(
        tags(query, filesystem, discovery, read_only),
        summary = "Search code structurally with ast-grep.",
        help = "Supported languages: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml. Use patterns like `if $COND { $BODY }`, `def $NAME($ARGS): $$$`, or `function $NAME($ARGS) { $$$ }`. When `language` is omitted for a file path, Agena infers it from the extension. Directory searches require `language` explicitly. Provide exactly one of pattern or a structured ast-grep rule object; relational/composite rules are supported. Rule bounds: 16 KiB, 16 levels, 512 values. Search returns at most 100 matches, explicitly marks incomplete scans, and flags shortened text previews. Use rewrite_ast to preview a single-file structural edit.",
        translations(
            locale(
                "zh-CN",
                summary = "使用 ast-grep 按代码结构搜索。",
                help = "支持 bash、c、cpp、csharp、css、dart、elixir、go、haskell、hcl、html、java、javascript、json、lua、markdown、nix、php、python、ruby、rust、solidity、swift、tsx、typescript、yaml。可用 `if $COND { $BODY }`、`def $NAME($ARGS): $$$` 或 `function $NAME($ARGS) { $$$ }` 等模式。按文件路径搜索时，省略 `language` 会根据扩展名推断；搜索目录必须显式指定。`pattern` 和结构化 ast-grep 规则对象必须且只能提供一个。规则上限为 16 KiB、16 层、512 个值。最多返回 100 个匹配项，并标明扫描不完整或文本预览被截断的情况。要预览单文件结构化修改，请用 rewrite_ast。"
            ),
            locale(
                "zh-TW",
                summary = "使用 ast-grep 依程式碼結構搜尋。",
                help = "支援 bash、c、cpp、csharp、css、dart、elixir、go、haskell、hcl、html、java、javascript、json、lua、markdown、nix、php、python、ruby、rust、solidity、swift、tsx、typescript、yaml。可使用 `if $COND { $BODY }`、`def $NAME($ARGS): $$$` 或 `function $NAME($ARGS) { $$$ }` 等模式。依檔案路徑搜尋時，省略 `language` 會依副檔名推斷；搜尋目錄則必須明確指定。`pattern` 與結構化 ast-grep 規則物件必須且只能提供一種。規則上限為 16 KiB、16 層、512 個值。最多回傳 100 個符合項目，並標明掃描不完整或文字預覽遭截斷的情況。要預覽單檔結構化修改，請使用 rewrite_ast。"
            ),
            locale(
                "ja-JP",
                summary = "ast-grep でコードの構造を検索します。",
                help = "対応言語は bash、c、cpp、csharp、css、dart、elixir、go、haskell、hcl、html、java、javascript、json、lua、markdown、nix、php、python、ruby、rust、solidity、swift、tsx、typescript、yaml です。`if $COND { $BODY }`、`def $NAME($ARGS): $$$`、`function $NAME($ARGS) { $$$ }` のようなパターンを使えます。ファイル指定では `language` を省略すると拡張子から推定します。ディレクトリ検索では明示してください。`pattern` と構造化 ast-grep ルールのどちらか一方だけを指定します。ルール上限は 16 KiB、16 階層、512 個の値です。検索結果は最大 100 件で、未完了の走査や短縮されたプレビューも明示します。単一ファイルの構造編集を確認するには rewrite_ast を使ってください。"
            ),
            locale(
                "ko-KR",
                summary = "ast-grep으로 코드 구조를 검색합니다.",
                help = "지원 언어는 bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml입니다. `if $COND { $BODY }`, `def $NAME($ARGS): $$$`, `function $NAME($ARGS) { $$$ }` 같은 패턴을 사용할 수 있습니다. 파일 경로 검색에서 `language`를 생략하면 확장자로 추론합니다. 디렉터리 검색에는 언어를 명시해야 합니다. `pattern`과 구조화된 ast-grep 규칙 중 하나만 제공하세요. 규칙 한도는 16 KiB, 16단계, 값 512개입니다. 결과는 최대 100개이며, 스캔 미완료나 축약된 미리보기는 표시됩니다. 단일 파일의 구조 편집 미리보기에는 rewrite_ast를 사용하세요."
            ),
            locale(
                "fr-FR",
                summary = "Rechercher du code par structure avec ast-grep.",
                help = "Langages pris en charge : bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript et yaml. Utilisez par exemple `if $COND { $BODY }`, `def $NAME($ARGS): $$$` ou `function $NAME($ARGS) { $$$ }`. Pour un fichier, `language` peut être déduit de l’extension ; il doit être indiqué pour une recherche dans un dossier. Fournissez exactement un `pattern` ou une règle ast-grep structurée. Limites : 16 Kio, 16 niveaux et 512 valeurs. Au plus 100 résultats sont renvoyés et les recherches incomplètes ou aperçus raccourcis sont signalés. Utilisez rewrite_ast pour prévisualiser une modification structurelle dans un seul fichier."
            ),
            locale(
                "de-DE",
                summary = "Code mit ast-grep anhand seiner Struktur durchsuchen.",
                help = "Unterstützt werden bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript und yaml. Beispiele: `if $COND { $BODY }`, `def $NAME($ARGS): $$$` oder `function $NAME($ARGS) { $$$ }`. Bei einer Datei wird `language` standardmäßig aus der Endung abgeleitet; bei Verzeichnissen muss es angegeben werden. Geben Sie genau eines an: `pattern` oder eine strukturierte ast-grep-Regel. Grenzen: 16 KiB, 16 Ebenen, 512 Werte. Es werden höchstens 100 Treffer geliefert; unvollständige Scans und gekürzte Vorschauen sind gekennzeichnet. Mit rewrite_ast lässt sich eine strukturelle Änderung an einer einzelnen Datei vorab anzeigen."
            ),
            locale(
                "es-ES",
                summary = "Busca código por su estructura con ast-grep.",
                help = "Lenguajes compatibles: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript y yaml. Puedes usar patrones como `if $COND { $BODY }`, `def $NAME($ARGS): $$$` o `function $NAME($ARGS) { $$$ }`. En una ruta de archivo, si omites `language`, Agena lo deduce de la extensión; en una búsqueda de directorio debes indicarlo. Proporciona exactamente uno: `pattern` o una regla ast-grep estructurada. Límites de regla: 16 KiB, 16 niveles y 512 valores. Devuelve como máximo 100 coincidencias e indica si el análisis quedó incompleto o si se acortó una vista previa. Usa rewrite_ast para previsualizar un cambio estructural en un único archivo."
            ),
            locale(
                "hi-IN",
                summary = "ast-grep से कोड की संरचना के आधार पर खोजें।",
                help = "समर्थित भाषाएँ: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml। `if $COND { $BODY }`, `def $NAME($ARGS): $$$` या `function $NAME($ARGS) { $$$ }` जैसे पैटर्न उपयोग करें। फ़ाइल पथ के लिए `language` छोड़ने पर Agena एक्सटेंशन से भाषा पहचानता है; डायरेक्टरी खोज में इसे स्पष्ट दें। `pattern` या संरचित ast-grep नियम—इनमें से ठीक एक दें। सीमा: 16 KiB, 16 स्तर और 512 मान। अधिकतम 100 मिलान लौटते हैं; अधूरी स्कैनिंग और छोटे किए गए पूर्वावलोकन बताए जाते हैं। एक फ़ाइल के संरचनात्मक बदलाव का पूर्वावलोकन करने के लिए rewrite_ast उपयोग करें।"
            ),
            locale(
                "ar-SA",
                summary = "ابحث في بنية الشيفرة باستخدام ast-grep.",
                help = "اللغات المدعومة: bash وc وcpp وcsharp وcss وdart وelixir وgo وhaskell وhcl وhtml وjava وjavascript وjson وlua وmarkdown وnix وphp وpython وruby وrust وsolidity وswift وtsx وtypescript وyaml. استخدم أنماطًا مثل `if $COND { $BODY }` أو `def $NAME($ARGS): $$$` أو `function $NAME($ARGS) { $$$ }`. عند تحديد ملف، تُستنتج اللغة من الامتداد إن حُذف `language`؛ أما البحث في مجلد فيتطلب تحديدها. قدّم `pattern` أو قاعدة ast-grep منظمة واحدة فقط. الحدود: 16 KiB و16 مستوى و512 قيمة. تُعاد 100 نتيجة كحد أقصى مع توضيح البحث غير المكتمل أو المعاينة المختصرة. استخدم rewrite_ast لمعاينة تعديل بنيوي في ملف واحد."
            ),
            locale(
                "pt-BR",
                summary = "Pesquise código pela estrutura usando ast-grep.",
                help = "Linguagens compatíveis: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript e yaml. Use padrões como `if $COND { $BODY }`, `def $NAME($ARGS): $$$` ou `function $NAME($ARGS) { $$$ }`. Ao indicar um arquivo, Agena deduz a linguagem pela extensão se `language` for omitido; buscas em diretórios exigem esse campo. Informe exatamente um `pattern` ou uma regra ast-grep estruturada. Limites: 16 KiB, 16 níveis e 512 valores. São retornadas até 100 correspondências, com indicação de varreduras incompletas e prévias encurtadas. Use rewrite_ast para visualizar uma alteração estrutural em um único arquivo."
            )
        )
    )]
    async fn dispatch_search_ast(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_owned();
        run_code_blocking(move || Self::invoke_search_ast(&workspace_root, input)).await
    }

    #[tool(
        tags(query, filesystem, discovery, read_only),
        summary = "Inspect a parsed syntax tree.",
        help = "Use `syntax_tree` to inspect named syntax nodes for a supported file. When `language` is omitted, Agena infers it from the file extension. The preview has at most 512 nodes, 50 children per node and max_depth 1–6 (default 2); children_truncated and truncated report omitted descendants. Source files are limited to 8 MiB.",
        translations(
            locale(
                "zh-CN",
                summary = "查看文件解析后的语法树。",
                help = "使用 `syntax_tree` 查看支持的文件中的命名语法节点。省略 `language` 时，Agena 会从扩展名推断。预览最多 512 个节点，每个节点最多 50 个子节点；`max_depth` 为 1–6，默认 2。`children_truncated` 和 `truncated` 会标明省略的后代节点。源文件最大 8 MiB。"
            ),
            locale(
                "zh-TW",
                summary = "檢視檔案剖析後的語法樹。",
                help = "使用 `syntax_tree` 查看支援檔案中的具名語法節點。省略 `language` 時，Agena 會依副檔名推斷。預覽最多 512 個節點，每個節點最多 50 個子節點；`max_depth` 為 1–6，預設 2。`children_truncated` 與 `truncated` 會標示省略的後代節點。來源檔案上限為 8 MiB。"
            ),
            locale(
                "ja-JP",
                summary = "解析済みの構文木を確認します。",
                help = "`syntax_tree` で対応ファイルの名前付き構文ノードを確認します。`language` を省略すると拡張子から推定します。プレビューは最大 512 ノード、各ノード 50 子、`max_depth` は 1～6（既定値 2）です。省略された子孫は `children_truncated` と `truncated` で示されます。ソースファイルは最大 8 MiB です。"
            ),
            locale(
                "ko-KR",
                summary = "파싱된 구문 트리를 살펴봅니다.",
                help = "`syntax_tree`로 지원되는 파일의 이름 있는 구문 노드를 확인하세요. `language`를 생략하면 확장자로 추론합니다. 미리보기는 노드 최대 512개, 노드당 자식 50개이며 `max_depth`는 1~6(기본 2)입니다. 생략된 하위 노드는 `children_truncated`와 `truncated`에 표시됩니다. 소스 파일은 최대 8 MiB입니다."
            ),
            locale(
                "fr-FR",
                summary = "Examiner un arbre syntaxique analysé.",
                help = "Utilisez `syntax_tree` pour inspecter les nœuds nommés d’un fichier pris en charge. Si `language` est omis, Agena le déduit de l’extension. L’aperçu est limité à 512 nœuds, 50 enfants par nœud et une profondeur de 1 à 6 (2 par défaut). `children_truncated` et `truncated` signalent les descendants omis. Les fichiers source sont limités à 8 Mio."
            ),
            locale(
                "de-DE",
                summary = "Einen geparsten Syntaxbaum untersuchen.",
                help = "Mit `syntax_tree` können Sie benannte Syntaxknoten einer unterstützten Datei ansehen. Fehlt `language`, wird es aus der Dateiendung abgeleitet. Die Vorschau umfasst höchstens 512 Knoten und 50 Kinder je Knoten; `max_depth` liegt zwischen 1 und 6 (Standard: 2). `children_truncated` und `truncated` melden ausgelassene Nachfahren. Quelldateien dürfen höchstens 8 MiB groß sein."
            ),
            locale(
                "es-ES",
                summary = "Inspecciona un árbol sintáctico analizado.",
                help = "Usa `syntax_tree` para consultar los nodos con nombre de un archivo compatible. Si omites `language`, Agena lo deduce de la extensión. La vista previa admite hasta 512 nodos y 50 hijos por nodo; `max_depth` va de 1 a 6 (por defecto, 2). `children_truncated` y `truncated` indican los descendientes omitidos. Los archivos fuente tienen un límite de 8 MiB."
            ),
            locale(
                "hi-IN",
                summary = "पार्स किए गए सिंटैक्स ट्री को देखें।",
                help = "समर्थित फ़ाइल के नामित सिंटैक्स नोड देखने के लिए `syntax_tree` उपयोग करें। `language` न देने पर Agena एक्सटेंशन से भाषा पहचानता है। पूर्वावलोकन में अधिकतम 512 नोड और प्रति नोड 50 बच्चे होते हैं; `max_depth` 1–6 (डिफ़ॉल्ट 2) है। छोड़े गए वंशज `children_truncated` और `truncated` में दिखते हैं। स्रोत फ़ाइल अधिकतम 8 MiB हो सकती है।"
            ),
            locale(
                "ar-SA",
                summary = "استعرض شجرة بناء الجملة بعد تحليلها.",
                help = "استخدم `syntax_tree` لفحص عقد بناء الجملة المسماة في ملف مدعوم. إذا حُذف `language` فسيُستنتج من امتداد الملف. تعرض المعاينة 512 عقدة كحد أقصى و50 ابنًا لكل عقدة، ويكون `max_depth` من 1 إلى 6 (الافتراضي 2). تشير `children_truncated` و`truncated` إلى الأحفاد المحذوفين. الحد الأقصى لحجم الملف المصدر 8 MiB."
            ),
            locale(
                "pt-BR",
                summary = "Inspecione uma árvore sintática analisada.",
                help = "Use `syntax_tree` para examinar nós nomeados de um arquivo compatível. Se `language` for omitido, Agena deduz a linguagem pela extensão. A prévia tem até 512 nós e 50 filhos por nó; `max_depth` varia de 1 a 6 (padrão 2). `children_truncated` e `truncated` indicam descendentes omitidos. Arquivos de origem podem ter até 8 MiB."
            )
        )
    )]
    async fn dispatch_syntax_tree(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeSyntaxTreeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_owned();
        run_code_blocking(move || Self::invoke_syntax_tree(&workspace_root, input)).await
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Preview or apply a revision-checked ast-grep rewrite in one file.",
        help = "Defaults to apply=false: returns a bounded unified diff, replacement count and before_sha256 without writing. Repeat with apply=true and expected_sha256 from the reviewed preview to publish. Provide exactly one pattern or structured rule and a replacement template (metavariables supported; empty deletes). Requires valid UTF-8 source, at most 8 MiB/file and 100 non-overlapping matches; rejects parse errors, unknown replacement variables, stale revisions and partial plans. Same file locks and publication checks as fs.replace; no directory-wide rewrite.",
        translations(
            locale(
                "zh-CN",
                summary = "在单个文件中预览或应用经过版本校验的 ast-grep 改写。",
                help = "默认 `apply=false`：仅返回有大小限制的 unified diff、替换数量和 `before_sha256`，不会写入文件。审阅预览后，再以其中的 `expected_sha256` 和 `apply=true` 提交。必须且只能提供一个模式或结构化规则，以及替换模板（支持元变量；空模板表示删除）。源文件须为有效 UTF-8，单个文件不超过 8 MiB，最多 100 个互不重叠的匹配；解析错误、未知替换变量、版本过期和部分计划都会被拒绝。文件锁和提交校验与 fs.replace 一致；不支持整目录改写。"
            ),
            locale(
                "zh-TW",
                summary = "在單一檔案中預覽或套用經版本檢查的 ast-grep 改寫。",
                help = "預設 `apply=false`：只回傳有大小限制的 unified diff、替換數量與 `before_sha256`，不會寫入檔案。檢視預覽後，再以其中的 `expected_sha256` 搭配 `apply=true` 提交。必須且只能提供一個模式或結構化規則，以及替換範本（支援中介變數；空範本表示刪除）。來源必須是有效 UTF-8，單檔不超過 8 MiB，最多 100 個不重疊的符合項目；解析錯誤、未知替換變數、版本過期與部分計畫都會遭拒。檔案鎖與提交檢查和 fs.replace 相同；不支援整個目錄改寫。"
            ),
            locale(
                "ja-JP",
                summary = "単一ファイルの ast-grep 書き換えを確認または適用します。",
                help = "既定の `apply=false` では、サイズ制限付き unified diff、置換数、`before_sha256` を返すだけで書き込みません。プレビューを確認した後、その `expected_sha256` と `apply=true` を指定して適用します。パターンまたは構造化ルールをちょうど 1 つと、置換テンプレートを指定してください（メタ変数に対応し、空なら削除）。入力は有効な UTF-8、1 ファイル 8 MiB 以下、重複しない一致は最大 100 件です。解析エラー、未知の置換変数、古いリビジョン、部分適用は拒否されます。ファイルロックと公開前検査は fs.replace と同じです。ディレクトリ全体の書き換えはできません。"
            ),
            locale(
                "ko-KR",
                summary = "단일 파일의 ast-grep 변경을 미리 보거나 적용합니다.",
                help = "기본값 `apply=false`에서는 크기가 제한된 unified diff, 교체 수, `before_sha256`만 반환하고 파일을 쓰지 않습니다. 미리보기를 검토한 뒤 해당 `expected_sha256`와 `apply=true`로 적용하세요. 패턴 또는 구조화 규칙은 정확히 하나만 주고 교체 템플릿도 제공하세요(메타변수 지원, 빈 값은 삭제). 유효한 UTF-8 원본이어야 하며 파일당 8 MiB, 겹치지 않는 일치 100개까지입니다. 파싱 오류, 알 수 없는 교체 변수, 오래된 리비전, 부분 계획은 거부됩니다. 파일 잠금과 게시 검사는 fs.replace와 같습니다. 디렉터리 전체 변경은 지원하지 않습니다."
            ),
            locale(
                "fr-FR",
                summary = "Prévisualiser ou appliquer une réécriture ast-grep vérifiée dans un fichier.",
                help = "Par défaut, `apply=false` renvoie un diff unifié limité, le nombre de remplacements et `before_sha256`, sans écrire. Après vérification, appliquez avec `apply=true` et le `expected_sha256` du diff. Fournissez exactement un motif ou une règle structurée, ainsi qu’un modèle de remplacement (métavariables admises ; modèle vide = suppression). Le fichier doit être en UTF-8 valide, faire au plus 8 Mio et contenir au maximum 100 correspondances sans chevauchement. Les erreurs d’analyse, variables inconnues, révisions périmées et plans partiels sont refusés. Les verrous et contrôles de publication sont ceux de fs.replace ; aucune réécriture de répertoire entier."
            ),
            locale(
                "de-DE",
                summary = "Eine geprüfte ast-grep-Umschreibung in einer Datei anzeigen oder anwenden.",
                help = "Bei `apply=false` (Standard) werden nur ein begrenzter Unified-Diff, die Ersetzungsanzahl und `before_sha256` geliefert; die Datei bleibt unverändert. Prüfen Sie die Vorschau und wenden Sie sie anschließend mit `apply=true` und dem dortigen `expected_sha256` an. Geben Sie genau ein Muster oder eine strukturierte Regel sowie eine Ersetzungsvorlage an (Metavariablen sind möglich; leer bedeutet löschen). Die Quelle muss gültiges UTF-8 sein, höchstens 8 MiB groß sein und darf maximal 100 nicht überlappende Treffer haben. Parsefehler, unbekannte Variablen, veraltete Revisionen und Teilpläne werden abgelehnt. Dateisperren und Veröffentlichungskontrollen entsprechen fs.replace; Verzeichnisänderungen sind ausgeschlossen."
            ),
            locale(
                "es-ES",
                summary = "Previsualiza o aplica un cambio ast-grep con control de revisión en un archivo.",
                help = "Por defecto, `apply=false` devuelve un diff unificado limitado, el número de sustituciones y `before_sha256`, sin escribir. Revisa la vista previa y aplica después con `apply=true` y el `expected_sha256` recibido. Proporciona exactamente un patrón o una regla estructurada y una plantilla de sustitución (admite metavariables; una plantilla vacía elimina). El origen debe ser UTF-8 válido, no superar 8 MiB por archivo ni 100 coincidencias sin solapamiento. Se rechazan errores de análisis, variables desconocidas, revisiones obsoletas y planes parciales. Usa los mismos bloqueos y comprobaciones de publicación que fs.replace; no modifica directorios enteros."
            ),
            locale(
                "hi-IN",
                summary = "एक फ़ाइल में revision-जाँचा ast-grep बदलाव देखें या लागू करें।",
                help = "डिफ़ॉल्ट `apply=false` में बिना लिखे सीमित unified diff, बदलावों की संख्या और `before_sha256` लौटता है। समीक्षा के बाद वही `expected_sha256` और `apply=true` देकर लागू करें। ठीक एक pattern या संरचित नियम और replacement template दें (metavariable समर्थित; खाली template हटाता है)। स्रोत वैध UTF-8 हो, प्रति फ़ाइल 8 MiB और बिना overlap वाले 100 मिलान तक सीमित हो। parse त्रुटि, अज्ञात replacement variable, पुराना revision और अधूरी योजना अस्वीकार होती है। फ़ाइल लॉक और प्रकाशन-जाँच fs.replace जैसी हैं; पूरे डायरेक्टरी पर बदलाव नहीं होता।"
            ),
            locale(
                "ar-SA",
                summary = "عاين أو طبّق إعادة كتابة ast-grep بعد التحقق من مراجعة ملف واحد.",
                help = "افتراضيًا `apply=false`، تُعاد فروق موحدة محدودة وعدد الاستبدالات و`before_sha256` دون كتابة. بعد مراجعة المعاينة، طبّق باستخدام `apply=true` و`expected_sha256` الظاهر فيها. قدّم نمطًا واحدًا فقط أو قاعدة منظمة واحدة مع قالب استبدال (تُدعم المتغيرات الوصفية، والقالب الفارغ يعني الحذف). يجب أن يكون المصدر UTF-8 صالحًا، وألا يتجاوز الملف 8 MiB أو النتائج غير المتداخلة 100. تُرفض أخطاء التحليل والمتغيرات المجهولة والمراجعات القديمة والخطط الجزئية. تستخدم أقفال الملف وفحوص النشر نفسها في fs.replace؛ ولا تُعاد كتابة مجلد كامل."
            ),
            locale(
                "pt-BR",
                summary = "Visualize ou aplique uma reescrita ast-grep validada em um único arquivo.",
                help = "Por padrão, `apply=false` retorna um diff unificado limitado, a quantidade de substituições e `before_sha256`, sem gravar. Revise a prévia e aplique depois com `apply=true` e o `expected_sha256` recebido. Informe exatamente um padrão ou regra estruturada e um modelo de substituição (aceita metavariáveis; vazio significa excluir). A origem deve ser UTF-8 válido, ter até 8 MiB por arquivo e no máximo 100 correspondências sem sobreposição. Erros de análise, variáveis desconhecidas, revisões desatualizadas e planos parciais são rejeitados. Os bloqueios e as verificações de publicação são iguais aos de fs.replace; não há reescrita de diretório inteiro."
            )
        )
    )]
    async fn dispatch_rewrite_ast(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeRewriteInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace = context.workspace_root.to_owned();
        run_code_blocking(move || invoke_rewrite(Path::new(&workspace), input)).await
    }

    fn invoke_search_ast(
        workspace_root: &str,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let title = format!("Search AST · {}", input.path);
        let result = search_ast(
            Path::new(workspace_root),
            StructuralSearchRequest {
                path: input.path.into(),
                pattern: input.pattern.unwrap_or_default(),
                rule: input.rule,
                language: input.language,
                limit: input.limit,
            },
        )
        .map_err(code_search_error_to_plugin)?;
        let output = format_search_output(&result);
        let summary = format!(
            "{} matches in {} files",
            result.matches.len(),
            result.scanned_files
        );
        let payload =
            serde_json::to_value(result).map_err(|err| PluginError::internal_error(&err))?;
        Ok(ToolInvokeOutput::from_parts(
            title,
            summary,
            output,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    fn invoke_syntax_tree(
        workspace_root: &str,
        input: CodeSyntaxTreeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let title = format!("Syntax tree · {}", input.path);
        let result = syntax_tree(
            Path::new(workspace_root),
            SyntaxTreeRequest {
                path: input.path.into(),
                language: input.language,
                max_depth: input.max_depth,
            },
        )
        .map_err(code_search_error_to_plugin)?;
        let summary = format!(
            "{} · root {}{}",
            result.language,
            result.root_kind,
            if result.has_error {
                " · parse errors"
            } else {
                ""
            }
        );
        let payload =
            serde_json::to_value(result).map_err(|err| PluginError::internal_error(&err))?;
        let output = serde_json::to_string_pretty(&payload)
            .map_err(|err| PluginError::internal_error(&err))?;
        Ok(ToolInvokeOutput::from_parts(
            title,
            summary,
            output,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }
}

pub(crate) fn new_plugin() -> CodePlugin {
    CodePlugin
}

async fn run_code_blocking<T, F>(work: F) -> SdkResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> SdkResult<T> + Send + 'static,
{
    let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
        .acquire()
        .await
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "acquire a code plugin worker",
                &error,
            ))
        })?;
    tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        work()
    })
    .await
    .map_err(|error| {
        PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
            "code plugin worker failed",
            &error,
        ))
    })?
}

fn code_search_error_to_plugin(error: CodeSearchError) -> PluginError {
    match error {
        CodeSearchError::InvalidParameters(message) => PluginError::invalid_params(message),
        error => PluginError::internal_error(&error),
    }
}
