//! UI-facing translations for the cloud provider tools.
//!
//! The three provider plugins expose the same small set of concepts with
//! provider-specific schemas. Keeping their translated copy here avoids
//! drifting descriptions while leaving the macro-generated English tool
//! contracts unchanged for model requests.

use agena_plugin_host::sdk::{PluginManifest, ToolDocsTranslation};

const LOCALES: [&str; 10] = [
    "zh-CN", "zh-TW", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "hi-IN", "ar-SA", "pt-BR",
];

pub(crate) fn localize_provider_manifest(manifest: &mut PluginManifest) {
    for tool in &mut manifest.tools {
        let Some(key) = tool_key(manifest.name.as_str(), tool.name.as_str()) else {
            continue;
        };
        for locale in LOCALES {
            let Some(summary) = summary(key, locale) else {
                continue;
            };
            tool.docs.translations.insert(
                locale.to_owned(),
                ToolDocsTranslation {
                    summary: Some(summary.to_owned()),
                    help: Some(help(key, locale).to_owned()),
                    ..Default::default()
                },
            );
        }
    }
}

fn tool_key(plugin: &str, tool: &str) -> Option<&'static str> {
    match (plugin, tool) {
        (_, "cloud_image_understanding") => Some("image_understanding"),
        (_, "cloud_document_understanding") => Some("document_understanding"),
        (_, "cloud_file_upload") => Some("file_upload"),
        (_, "cloud_file_status") => Some("file_status"),
        (_, "cloud_file_delete") => Some("file_delete"),
        ("claude", "cloud_code_execution") | ("gemini", "cloud_code_execution") => {
            Some("code_execution")
        }
        ("chatgpt", "cloud_web_search") | ("claude", "cloud_web_search") => Some("web_search"),
        ("claude", "cloud_web_fetch") => Some("web_fetch"),
        ("claude", "cloud_advisor") => Some("advisor"),
        ("gemini", "cloud_url_context") => Some("url_context"),
        ("gemini", "cloud_google_search") => Some("google_search"),
        ("gemini", "cloud_google_maps") => Some("google_maps"),
        ("chatgpt", "cloud_file_search") | ("gemini", "cloud_file_search") => Some("file_search"),
        ("chatgpt", "cloud_code_interpreter") => Some("code_interpreter"),
        ("chatgpt", "cloud_shell") => Some("shell"),
        (_, "cloud_image_generation") => Some("image_generation"),
        (_, "cloud_image_edit") => Some("image_edit"),
        _ => None,
    }
}

fn summary(key: &str, locale: &str) -> Option<&'static str> {
    Some(match (key, locale) {
        ("image_understanding", "zh-CN") => {
            "将明确指定的图片交给云端模型理解；不会代替本地文件浏览。"
        }
        ("image_understanding", "zh-TW") => {
            "將明確指定的圖片交由雲端模型理解；不會取代本機檔案瀏覽。"
        }
        ("image_understanding", "ja-JP") => {
            "指定した画像をクラウドモデルで解析します。ローカルファイルの閲覧には使いません。"
        }
        ("image_understanding", "ko-KR") => {
            "지정한 이미지를 클라우드 모델로 분석합니다. 로컬 파일을 둘러보는 도구는 아닙니다."
        }
        ("image_understanding", "fr-FR") => {
            "Analyser les images fournies explicitement dans le cloud, sans parcourir les fichiers locaux."
        }
        ("image_understanding", "de-DE") => {
            "Explizit angegebene Bilder in der Cloud auswerten; keine lokalen Dateien durchsuchen."
        }
        ("image_understanding", "es-ES") => {
            "Analiza en la nube las imágenes indicadas; no sirve para explorar archivos locales."
        }
        ("image_understanding", "hi-IN") => {
            "स्पष्ट रूप से दी गई छवियों का क्लाउड मॉडल से विश्लेषण करें; यह स्थानीय फ़ाइलें ब्राउज़ नहीं करता।"
        }
        ("image_understanding", "ar-SA") => {
            "حلّل الصور المحددة صراحةً عبر النموذج السحابي؛ لا تتصفح الملفات المحلية."
        }
        ("image_understanding", "pt-BR") => {
            "Analise na nuvem as imagens informadas; isso não navega pelos arquivos locais."
        }

        ("document_understanding", "zh-CN") => {
            "将明确指定的 PDF 或文本交给云端模型理解；不会代替本地文件浏览。"
        }
        ("document_understanding", "zh-TW") => {
            "將明確指定的 PDF 或文字交由雲端模型理解；不會取代本機檔案瀏覽。"
        }
        ("document_understanding", "ja-JP") => {
            "指定した PDF やテキストをクラウドモデルで解析します。ローカルファイルの閲覧には使いません。"
        }
        ("document_understanding", "ko-KR") => {
            "지정한 PDF나 텍스트 문서를 클라우드 모델로 분석합니다. 로컬 파일을 둘러보는 도구는 아닙니다."
        }
        ("document_understanding", "fr-FR") => {
            "Analyser les PDF ou textes fournis explicitement dans le cloud, sans parcourir les fichiers locaux."
        }
        ("document_understanding", "de-DE") => {
            "Explizit angegebene PDFs oder Texte in der Cloud auswerten; keine lokalen Dateien durchsuchen."
        }
        ("document_understanding", "es-ES") => {
            "Analiza en la nube los PDF o textos indicados; no sirve para explorar archivos locales."
        }
        ("document_understanding", "hi-IN") => {
            "दिए गए PDF या टेक्स्ट दस्तावेज़ों का क्लाउड मॉडल से विश्लेषण करें; यह स्थानीय फ़ाइलें ब्राउज़ नहीं करता।"
        }
        ("document_understanding", "ar-SA") => {
            "حلّل ملفات PDF أو النصوص المحددة عبر النموذج السحابي؛ لا تتصفح الملفات المحلية."
        }
        ("document_understanding", "pt-BR") => {
            "Analise na nuvem os PDFs ou textos informados; isso não navega pelos arquivos locais."
        }

        ("file_upload", "zh-CN") => {
            "将一个获准的本地文件上传到云端，并返回归属当前会话的文件句柄。"
        }
        ("file_upload", "zh-TW") => {
            "將一個獲准的本機檔案上傳至雲端，並回傳屬於目前工作階段的檔案代號。"
        }
        ("file_upload", "ja-JP") => {
            "許可されたローカルファイルを 1 つクラウドへアップロードし、このセッション専用のハンドルを返します。"
        }
        ("file_upload", "ko-KR") => {
            "허용된 로컬 파일 하나를 클라우드에 업로드하고 현재 세션 전용 핸들을 반환합니다."
        }
        ("file_upload", "fr-FR") => {
            "Téléverser un fichier local autorisé et renvoyer une référence propre à la session."
        }
        ("file_upload", "de-DE") => {
            "Eine freigegebene lokale Datei hochladen und eine sitzungsgebundene Referenz zurückgeben."
        }
        ("file_upload", "es-ES") => {
            "Sube un archivo local autorizado y devuelve una referencia vinculada a la sesión."
        }
        ("file_upload", "hi-IN") => {
            "एक अनुमत स्थानीय फ़ाइल क्लाउड पर अपलोड करें और इसी सत्र का हैंडल लौटाएँ।"
        }
        ("file_upload", "ar-SA") => "ارفع ملفًا محليًا مسموحًا وأعد معرّفًا مرتبطًا بهذه الجلسة.",
        ("file_upload", "pt-BR") => {
            "Envie um arquivo local autorizado e retorne uma referência vinculada à sessão."
        }

        ("file_status", "zh-CN") => "查询归属当前连接的云端文件状态；这不是本地路径查询。",
        ("file_status", "zh-TW") => "查詢屬於目前連線的雲端檔案狀態；這不是本機路徑查詢。",
        ("file_status", "ja-JP") => {
            "この接続で管理するクラウドファイルの状態を確認します。ローカルパスの確認ではありません。"
        }
        ("file_status", "ko-KR") => {
            "현재 연결이 소유한 클라우드 파일의 상태를 확인합니다. 로컬 경로 조회가 아닙니다."
        }
        ("file_status", "fr-FR") => {
            "Consulter l’état d’un fichier cloud géré par cette connexion, pas un chemin local."
        }
        ("file_status", "de-DE") => {
            "Den Status einer zugehörigen Cloud-Datei prüfen, keinen lokalen Pfad."
        }
        ("file_status", "es-ES") => {
            "Consulta el estado de un archivo cloud asociado; no es una ruta local."
        }
        ("file_status", "hi-IN") => "इस कनेक्शन से जुड़े क्लाउड फ़ाइल की स्थिति देखें; यह स्थानीय पथ नहीं है।",
        ("file_status", "ar-SA") => {
            "تحقق من حالة ملف سحابي تابع لهذا الاتصال؛ فهذا ليس مسارًا محليًا."
        }
        ("file_status", "pt-BR") => {
            "Consulte o estado de um arquivo de nuvem associado; não é um caminho local."
        }

        ("file_delete", "zh-CN") => "请求删除归属当前会话的云端文件，并保留本地原件。",
        ("file_delete", "zh-TW") => "要求刪除屬於目前工作階段的雲端檔案，並保留本機原始檔。",
        ("file_delete", "ja-JP") => {
            "このセッションが管理するクラウドファイルの削除を依頼し、ローカルの原本は残します。"
        }
        ("file_delete", "ko-KR") => {
            "현재 세션 소유의 클라우드 파일 삭제를 요청하고 로컬 원본은 보존합니다."
        }
        ("file_delete", "fr-FR") => {
            "Demander la suppression d’un fichier cloud associé à la session et conserver l’original local."
        }
        ("file_delete", "de-DE") => {
            "Die Löschung einer sitzungsgebundenen Cloud-Datei anfordern; das lokale Original bleibt erhalten."
        }
        ("file_delete", "es-ES") => {
            "Solicita eliminar un archivo cloud de la sesión y conserva el original local."
        }
        ("file_delete", "hi-IN") => {
            "सत्र के क्लाउड फ़ाइल को हटाने का अनुरोध करें; स्थानीय मूल फ़ाइल बनी रहती है।"
        }
        ("file_delete", "ar-SA") => {
            "اطلب حذف الملف السحابي المرتبط بالجلسة مع الإبقاء على النسخة المحلية الأصلية."
        }
        ("file_delete", "pt-BR") => {
            "Solicite a exclusão de um arquivo de nuvem da sessão e mantenha o original local."
        }

        ("code_execution", "zh-CN") => "在云端基础设施中运行代码，不会在这台电脑上执行。",
        ("code_execution", "zh-TW") => "在雲端基礎架構執行程式碼，不會在這台電腦上執行。",
        ("code_execution", "ja-JP") => "この端末ではなく、クラウド環境でコードを実行します。",
        ("code_execution", "ko-KR") => "이 컴퓨터가 아니라 클라우드 환경에서 코드를 실행합니다.",
        ("code_execution", "fr-FR") => {
            "Exécuter du code dans l’infrastructure cloud, pas sur cet ordinateur."
        }
        ("code_execution", "de-DE") => {
            "Code in der Cloud-Umgebung statt auf diesem Rechner ausführen."
        }
        ("code_execution", "es-ES") => {
            "Ejecuta código en la infraestructura cloud, no en este equipo."
        }
        ("code_execution", "hi-IN") => "इस कंप्यूटर पर नहीं, क्लाउड इन्फ़्रास्ट्रक्चर में कोड चलाएँ।",
        ("code_execution", "ar-SA") => {
            "نفّذ التعليمات البرمجية في البنية السحابية، لا على هذا الجهاز."
        }
        ("code_execution", "pt-BR") => {
            "Execute código na infraestrutura de nuvem, não neste computador."
        }

        ("web_search", "zh-CN") => "在云端搜索网页并返回来源；不会操作本地浏览器。",
        ("web_search", "zh-TW") => "在雲端搜尋網頁並回傳來源；不會操作本機瀏覽器。",
        ("web_search", "ja-JP") => {
            "クラウドでウェブを検索し、出典を返します。ローカルブラウザーは操作しません。"
        }
        ("web_search", "ko-KR") => {
            "클라우드에서 웹을 검색하고 출처를 반환합니다. 로컬 브라우저를 조작하지 않습니다."
        }
        ("web_search", "fr-FR") => {
            "Rechercher sur le Web dans le cloud et renvoyer les sources, sans utiliser le navigateur local."
        }
        ("web_search", "de-DE") => {
            "Im Cloud-Dienst im Web suchen und Quellen zurückgeben, ohne den lokalen Browser zu verwenden."
        }
        ("web_search", "es-ES") => {
            "Busca en la web desde la nube y devuelve fuentes; no usa el navegador local."
        }
        ("web_search", "hi-IN") => "क्लाउड में वेब खोजें और स्रोत लौटाएँ; स्थानीय ब्राउज़र का उपयोग नहीं होता।",
        ("web_search", "ar-SA") => {
            "ابحث في الويب عبر السحابة وأعد المصادر؛ لا تستخدم المتصفح المحلي."
        }
        ("web_search", "pt-BR") => {
            "Pesquise na web pela nuvem e retorne as fontes; não usa o navegador local."
        }

        ("web_fetch", "zh-CN") => "在云端抓取并处理网页内容；不会使用本地浏览器。",
        ("web_fetch", "zh-TW") => "在雲端擷取並處理網頁內容；不會使用本機瀏覽器。",
        ("web_fetch", "ja-JP") => {
            "クラウドでウェブコンテンツを取得・処理します。ローカルブラウザーは使いません。"
        }
        ("web_fetch", "ko-KR") => {
            "클라우드에서 웹 콘텐츠를 가져와 처리합니다. 로컬 브라우저는 사용하지 않습니다."
        }
        ("web_fetch", "fr-FR") => {
            "Récupérer et traiter du contenu Web dans le cloud, sans passer par le navigateur local."
        }
        ("web_fetch", "de-DE") => {
            "Webinhalte in der Cloud abrufen und verarbeiten, ohne den lokalen Browser zu verwenden."
        }
        ("web_fetch", "es-ES") => {
            "Obtiene y procesa contenido web en la nube, no mediante el navegador local."
        }
        ("web_fetch", "hi-IN") => {
            "क्लाउड में वेब सामग्री प्राप्त करके संसाधित करें; स्थानीय ब्राउज़र का उपयोग नहीं होता।"
        }
        ("web_fetch", "ar-SA") => "اجلب محتوى الويب وعالجه عبر السحابة، لا عبر المتصفح المحلي.",
        ("web_fetch", "pt-BR") => {
            "Busque e processe conteúdo da web na nuvem, sem usar o navegador local."
        }

        ("advisor", "zh-CN") => "将提供的上下文交给云端顾问模型分析。",
        ("advisor", "zh-TW") => "將提供的內容交由雲端顧問模型分析。",
        ("advisor", "ja-JP") => "指定したコンテキストをクラウド上のアドバイザーモデルに渡します。",
        ("advisor", "ko-KR") => "제공한 맥락을 클라우드 어드바이저 모델에 전달합니다.",
        ("advisor", "fr-FR") => {
            "Soumettre le contexte fourni à un modèle conseiller dans le cloud."
        }
        ("advisor", "de-DE") => {
            "Den angegebenen Kontext von einem Cloud-Beratermodell auswerten lassen."
        }
        ("advisor", "es-ES") => {
            "Consulta un modelo asesor en la nube con el contexto proporcionado."
        }
        ("advisor", "hi-IN") => "दिए गए संदर्भ को क्लाउड सलाहकार मॉडल से विश्लेषित कराएँ।",
        ("advisor", "ar-SA") => "اطلب من نموذج استشاري سحابي تحليل السياق المقدم.",
        ("advisor", "pt-BR") => "Consulte um modelo consultor na nuvem com o contexto fornecido.",

        ("url_context", "zh-CN") => "在云端读取并引用提示中提供的网址内容，不访问本地文件。",
        ("url_context", "zh-TW") => "在雲端讀取並引用提示中提供的網址內容，不會存取本機檔案。",
        ("url_context", "ja-JP") => {
            "プロンプトで指定した URL の内容をクラウドで取得し、回答の根拠にします。ローカルファイルにはアクセスしません。"
        }
        ("url_context", "ko-KR") => {
            "프롬프트에 지정한 URL 내용을 클라우드에서 가져와 답변 근거로 사용합니다. 로컬 파일에는 접근하지 않습니다."
        }
        ("url_context", "fr-FR") => {
            "Récupérer et citer dans le cloud le contenu des URL fournies, sans accéder aux fichiers locaux."
        }
        ("url_context", "de-DE") => {
            "In der Cloud Inhalte angegebener URLs abrufen und als Belege verwenden; kein Zugriff auf lokale Dateien."
        }
        ("url_context", "es-ES") => {
            "Obtiene contenido de las URL indicadas para fundamentar la respuesta; no accede a archivos locales."
        }
        ("url_context", "hi-IN") => {
            "दिए गए URL की सामग्री क्लाउड में लेकर उत्तर को आधार दें; स्थानीय फ़ाइलें नहीं खुलतीं।"
        }
        ("url_context", "ar-SA") => {
            "استرجع محتوى عناوين URL المحددة واستند إليه عبر السحابة؛ دون الوصول إلى الملفات المحلية."
        }
        ("url_context", "pt-BR") => {
            "Busque conteúdo das URLs informadas para fundamentar a resposta; não acessa arquivos locais."
        }

        ("google_search", "zh-CN") => {
            "在 Google 云端搜索网页并为回答提供依据；不是本地浏览器操作。"
        }
        ("google_search", "zh-TW") => {
            "在 Google 雲端搜尋網頁並為回答提供依據；不是本機瀏覽器操作。"
        }
        ("google_search", "ja-JP") => {
            "Google のクラウド検索でウェブを調べ、回答の根拠を示します。ローカルブラウザーは使いません。"
        }
        ("google_search", "ko-KR") => {
            "Google 클라우드에서 웹을 검색해 답변 근거를 제공합니다. 로컬 브라우저를 조작하지 않습니다."
        }
        ("google_search", "fr-FR") => {
            "Rechercher sur le Web avec Google dans le cloud et étayer la réponse, sans navigateur local."
        }
        ("google_search", "de-DE") => {
            "Mit Google in der Cloud suchen und Antworten belegen, ohne den lokalen Browser zu bedienen."
        }
        ("google_search", "es-ES") => {
            "Busca con Google en la nube y aporta fuentes para la respuesta; no usa el navegador local."
        }
        ("google_search", "hi-IN") => {
            "Google क्लाउड में वेब खोजकर उत्तर के लिए स्रोत दें; स्थानीय ब्राउज़र का उपयोग नहीं होता।"
        }
        ("google_search", "ar-SA") => {
            "ابحث باستخدام Google عبر السحابة واستند إلى المصادر؛ دون استخدام المتصفح المحلي."
        }
        ("google_search", "pt-BR") => {
            "Pesquise com o Google na nuvem e fundamente a resposta; não usa o navegador local."
        }

        ("google_maps", "zh-CN") => "查询 Google Maps 云端数据并返回可引用的地点依据。",
        ("google_maps", "zh-TW") => "查詢 Google Maps 雲端資料並回傳可引用的地點依據。",
        ("google_maps", "ja-JP") => {
            "Google Maps のクラウドデータを検索し、参照できる根拠を返します。"
        }
        ("google_maps", "ko-KR") => {
            "Google Maps 클라우드 데이터를 조회하고 인용할 근거를 반환합니다."
        }
        ("google_maps", "fr-FR") => {
            "Interroger les données Google Maps dans le cloud et renvoyer des sources de localisation."
        }
        ("google_maps", "de-DE") => {
            "Google-Maps-Daten in der Cloud abfragen und belegbare Ortsangaben zurückgeben."
        }
        ("google_maps", "es-ES") => {
            "Consulta los datos de Google Maps en la nube y devuelve fuentes sobre lugares."
        }
        ("google_maps", "hi-IN") => "Google Maps क्लाउड डेटा खोजें और स्थानों के लिए स्रोत लौटाएँ।",
        ("google_maps", "ar-SA") => "استعلم عن بيانات Google Maps السحابية وأعد مصادر للأماكن.",
        ("google_maps", "pt-BR") => {
            "Consulte dados do Google Maps na nuvem e retorne fontes sobre lugares."
        }

        ("file_search", "zh-CN") => "搜索已配置的云端文件库，不会搜索这台电脑上的文件。",
        ("file_search", "zh-TW") => "搜尋已設定的雲端檔案庫，不會搜尋這台電腦上的檔案。",
        ("file_search", "ja-JP") => {
            "設定済みのクラウドファイルストアを検索します。この端末上のファイルは検索しません。"
        }
        ("file_search", "ko-KR") => {
            "구성된 클라우드 파일 저장소를 검색합니다. 이 컴퓨터의 파일은 검색하지 않습니다."
        }
        ("file_search", "fr-FR") => {
            "Rechercher dans les espaces de fichiers cloud configurés, pas sur cet ordinateur."
        }
        ("file_search", "de-DE") => {
            "Konfigurierte Cloud-Dateispeicher durchsuchen, nicht Dateien auf diesem Rechner."
        }
        ("file_search", "es-ES") => {
            "Busca en los almacenes de archivos cloud configurados, no en este equipo."
        }
        ("file_search", "hi-IN") => "कॉन्फ़िगर किए गए क्लाउड फ़ाइल स्टोर खोजें; इस कंप्यूटर की फ़ाइलें नहीं।",
        ("file_search", "ar-SA") => {
            "ابحث في مخازن الملفات السحابية المُهيأة، لا في ملفات هذا الجهاز."
        }
        ("file_search", "pt-BR") => {
            "Pesquise nos repositórios de arquivos na nuvem configurados, não neste computador."
        }

        ("code_interpreter", "zh-CN") => "在云端容器中运行 Python；该环境独立于 Agena 本地工作区。",
        ("code_interpreter", "zh-TW") => "在雲端容器中執行 Python；該環境與 Agena 本機工作區分開。",
        ("code_interpreter", "ja-JP") => {
            "クラウドコンテナで Python を実行します。Agena のローカル作業環境とは別です。"
        }
        ("code_interpreter", "ko-KR") => {
            "클라우드 컨테이너에서 Python을 실행합니다. Agena 로컬 작업 공간과는 별개입니다."
        }
        ("code_interpreter", "fr-FR") => {
            "Exécuter Python dans un conteneur cloud séparé de l’espace de travail local d’Agena."
        }
        ("code_interpreter", "de-DE") => {
            "Python in einem Cloud-Container ausführen, getrennt vom lokalen Agena-Workspace."
        }
        ("code_interpreter", "es-ES") => {
            "Ejecuta Python en un contenedor cloud separado del espacio de trabajo local de Agena."
        }
        ("code_interpreter", "hi-IN") => {
            "क्लाउड कंटेनर में Python चलाएँ; यह Agena के स्थानीय कार्यस्थान से अलग है।"
        }
        ("code_interpreter", "ar-SA") => {
            "نفّذ Python في حاوية سحابية منفصلة عن مساحة عمل Agena المحلية."
        }
        ("code_interpreter", "pt-BR") => {
            "Execute Python em um contêiner na nuvem, separado do espaço de trabalho local do Agena."
        }

        ("shell", "zh-CN") => "在云端容器中运行 shell 命令，绝不会在本地终端执行。",
        ("shell", "zh-TW") => "在雲端容器執行 shell 命令，絕不會在本機終端執行。",
        ("shell", "ja-JP") => {
            "クラウドコンテナで shell コマンドを実行します。ローカル端末では実行しません。"
        }
        ("shell", "ko-KR") => {
            "클라우드 컨테이너에서 셸 명령을 실행합니다. 로컬 터미널에서는 실행하지 않습니다."
        }
        ("shell", "fr-FR") => {
            "Exécuter des commandes shell dans un conteneur cloud, jamais dans le terminal local."
        }
        ("shell", "de-DE") => {
            "Shell-Befehle in einem Cloud-Container ausführen, niemals im lokalen Terminal."
        }
        ("shell", "es-ES") => {
            "Ejecuta comandos de shell en un contenedor cloud, nunca en el terminal local."
        }
        ("shell", "hi-IN") => "क्लाउड कंटेनर में shell कमांड चलाएँ, स्थानीय टर्मिनल में नहीं।",
        ("shell", "ar-SA") => "نفّذ أوامر shell في حاوية سحابية، وليس في الطرفية المحلية مطلقًا.",
        ("shell", "pt-BR") => {
            "Execute comandos shell em um contêiner na nuvem, nunca no terminal local."
        }

        ("image_generation", "zh-CN") => "在云端生成图片，并将返回的图片保存为本地附件。",
        ("image_generation", "zh-TW") => "在雲端產生圖片，並將回傳的圖片儲存為本機附件。",
        ("image_generation", "ja-JP") => {
            "クラウドで画像を生成し、返された画像をローカルの添付ファイルとして保存します。"
        }
        ("image_generation", "ko-KR") => {
            "클라우드에서 이미지를 생성하고 반환된 이미지를 로컬 첨부 파일로 저장합니다."
        }
        ("image_generation", "fr-FR") => {
            "Générer des images dans le cloud et enregistrer les résultats en pièces jointes locales."
        }
        ("image_generation", "de-DE") => {
            "Bilder in der Cloud erzeugen und die Ergebnisse als lokale Anhänge speichern."
        }
        ("image_generation", "es-ES") => {
            "Genera imágenes en la nube y guarda los resultados como adjuntos locales."
        }
        ("image_generation", "hi-IN") => {
            "क्लाउड में छवियाँ बनाएँ और लौटे परिणामों को स्थानीय अटैचमेंट के रूप में सहेजें।"
        }
        ("image_generation", "ar-SA") => "أنشئ صورًا عبر السحابة واحفظ النتائج كمرفقات محلية.",
        ("image_generation", "pt-BR") => {
            "Gere imagens na nuvem e salve os resultados como anexos locais."
        }

        ("image_edit", "zh-CN") => "在云端编辑获准提供的图片，并将结果另存为独立文件。",
        ("image_edit", "zh-TW") => "在雲端編輯獲准提供的圖片，並將結果另存為獨立檔案。",
        ("image_edit", "ja-JP") => {
            "許可された画像をクラウドで編集し、結果を別のファイルとして保存します。"
        }
        ("image_edit", "ko-KR") => {
            "허용된 이미지를 클라우드에서 편집하고 결과를 별도 파일로 저장합니다."
        }
        ("image_edit", "fr-FR") => {
            "Modifier dans le cloud les images autorisées et enregistrer le résultat séparément."
        }
        ("image_edit", "de-DE") => {
            "Freigegebene Bilder in der Cloud bearbeiten und das Ergebnis separat speichern."
        }
        ("image_edit", "es-ES") => {
            "Edita en la nube las imágenes autorizadas y guarda el resultado por separado."
        }
        ("image_edit", "hi-IN") => "अनुमत छवियों को क्लाउड में संपादित करें और परिणाम अलग फ़ाइल में सहेजें।",
        ("image_edit", "ar-SA") => "حرّر الصور المسموح بها عبر السحابة واحفظ النتيجة في ملف مستقل.",
        ("image_edit", "pt-BR") => {
            "Edite na nuvem as imagens autorizadas e salve o resultado separadamente."
        }
        _ => return None,
    })
}

fn localized(key: &str, locale: &str) -> &'static str {
    match (key, locale) {
        ("image_understanding" | "document_understanding", "zh-CN") => {
            "在云端处理明确指定且经过权限检查的输入；本地项目文件不会自动上传。准备过程有大小限制，输入和提示可能产生云端费用。以内联方式发送的内容不等于创建远端文件。结果会包含输入哈希、提供商、模型和用量。"
        }
        ("image_understanding" | "document_understanding", "zh-TW") => {
            "在雲端處理明確指定且通過權限檢查的輸入；本機專案檔案不會自動上傳。準備過程有大小限制，輸入與提示可能產生雲端費用。以內嵌方式傳送的內容不代表建立遠端檔案。結果會包含輸入雜湊、服務提供者、模型與用量。"
        }
        ("image_understanding" | "document_understanding", "ja-JP") => {
            "明示され、権限確認済みの入力だけをクラウドで処理します。ローカルのプロジェクトファイルは自動送信されず、準備処理にはサイズ上限があります。クラウド利用料が発生する場合があります。インライン送信はリモートファイルの作成を意味しません。結果には入力ハッシュ、プロバイダー、モデル、使用量が含まれます。"
        }
        ("image_understanding" | "document_understanding", "ko-KR") => {
            "명시적으로 지정하고 권한을 확인한 입력만 클라우드에서 처리합니다. 로컬 프로젝트 파일은 자동 전송되지 않으며 준비 작업에는 크기 제한이 있습니다. 클라우드 요금이 발생할 수 있습니다. 인라인 입력은 원격 파일을 만드는 것이 아닙니다. 결과에는 입력 해시, 제공자, 모델, 사용량이 포함됩니다."
        }
        ("image_understanding" | "document_understanding", "fr-FR") => {
            "Seules les entrées explicitement fournies et autorisées sont traitées dans le cloud ; les fichiers du projet local ne sont pas téléversés automatiquement. La préparation est limitée et l’inférence peut être facturée. Un envoi intégré à la requête ne crée pas de fichier distant. Le résultat inclut les empreintes des entrées, le fournisseur, le modèle et l’usage."
        }
        ("image_understanding" | "document_understanding", "de-DE") => {
            "Verarbeitet werden nur ausdrücklich angegebene und geprüfte Eingaben; lokale Projektdateien werden nicht automatisch hochgeladen. Die Vorbereitung ist begrenzt, und die Cloud-Nutzung kann Kosten verursachen. Inline-Daten werden dadurch nicht zu einer Remote-Datei. Ergebnisse enthalten Eingabe-Hashes, Anbieter, Modell und Verbrauch."
        }
        ("image_understanding" | "document_understanding", "es-ES") => {
            "En la nube solo se procesan entradas indicadas expresamente y con permiso; los archivos locales del proyecto no se suben automáticamente. La preparación tiene límites y la inferencia puede tener coste. Enviar datos en línea no crea un archivo remoto. El resultado incluye hashes, proveedor, modelo y uso."
        }
        ("image_understanding" | "document_understanding", "hi-IN") => {
            "क्लाउड में केवल स्पष्ट रूप से दी गई और अनुमति-जाँची गई सामग्री भेजी जाती है; स्थानीय प्रोजेक्ट फ़ाइलें अपने-आप अपलोड नहीं होतीं। तैयारी की सीमा है और क्लाउड शुल्क लग सकता है। इनलाइन इनपुट अलग रिमोट फ़ाइल नहीं बनाता। परिणाम में इनपुट हैश, प्रदाता, मॉडल और उपयोग शामिल हैं।"
        }
        ("image_understanding" | "document_understanding", "ar-SA") => {
            "تُعالَج سحابيًا المدخلات المحددة صراحةً والمصرح بها فقط؛ ولا تُرفع ملفات المشروع المحلية تلقائيًا. للتحضير حدود، وقد تُفرض رسوم على الاستدلال السحابي. الإرسال المضمن لا ينشئ ملفًا بعيدًا. تتضمن النتيجة بصمات المدخلات والمزوّد والنموذج والاستخدام."
        }
        ("image_understanding" | "document_understanding", "pt-BR") => {
            "Somente entradas informadas explicitamente e autorizadas são processadas na nuvem; arquivos locais do projeto não são enviados automaticamente. A preparação tem limites e a inferência pode ser cobrada. O envio inline não cria um arquivo remoto. O resultado inclui hashes das entradas, provedor, modelo e uso."
        }

        ("file_upload", "zh-CN") => {
            "此操作只创建远端文件，不会分析内容；单个输入上限为 20 MiB。文件会经过内容检查，可选校验版本，本地原件不会修改。返回句柄仅绑定当前工作区、会话和提供商连接，不能替换成任意厂商 ID。超时可能导致远端是否接收不明；先检查句柄，不要自动重试。文件处理完成前先查状态；不再需要时请明确删除。"
        }
        ("file_upload", "zh-TW") => {
            "此操作只建立遠端檔案，不會分析內容；單一輸入上限為 20 MiB。檔案會經過內容檢查，也可選擇驗證版本，本機原始檔不會變動。回傳代號只綁定目前工作區、工作階段與服務提供者連線，不可換成任意廠商 ID。逾時時遠端是否已接收可能不明；先檢查代號，不要自動重試。使用處理中的檔案前先查狀態；不再需要時請明確刪除。"
        }
        ("file_upload", "ja-JP") => {
            "これはファイルをリモートへ作成する操作で、内容の解析は行いません。入力は 1 件 20 MiB までで、内容確認と任意のリビジョン確認を行い、ローカル原本は変更しません。返されたハンドルはこのワークスペース・セッション・接続に紐づき、任意のベンダー ID には置き換えられません。タイムアウト時は受理されたか不明な場合があるため、自動再試行せずハンドルを確認してください。処理中ファイルは状態を確認してから使い、不要になったら明示的に削除します。"
        }
        ("file_upload", "ko-KR") => {
            "이 작업은 파일을 원격에 만들 뿐 내용을 분석하지 않습니다. 입력은 파일당 20 MiB까지이며 내용과 선택적 리비전을 확인하고 로컬 원본은 바꾸지 않습니다. 반환된 핸들은 현재 작업 공간·세션·제공자 연결에 묶이며 임의 공급자 ID로 대체할 수 없습니다. 시간 초과 시 원격 수신 여부가 불명확할 수 있으므로 자동 재시도하지 말고 핸들을 확인하세요. 처리 중인 파일은 상태를 확인한 뒤 사용하고, 필요 없으면 명시적으로 삭제하세요."
        }
        ("file_upload", "fr-FR") => {
            "Cette opération crée un fichier distant sans l’analyser. Chaque entrée est limitée à 20 Mio, son contenu est vérifié, sa révision peut l’être aussi, et l’original local reste intact. La référence est liée à cet espace, cette session et cette connexion ; elle ne peut pas être remplacée par un ID fournisseur arbitraire. Après un délai dépassé, l’acceptation distante peut être incertaine : vérifiez la référence au lieu de réessayer automatiquement. Consultez l’état avant d’utiliser un fichier en traitement et supprimez explicitement ceux qui ne sont plus nécessaires."
        }
        ("file_upload", "de-DE") => {
            "Dieser Vorgang legt eine Remote-Datei an, analysiert sie aber nicht. Eingaben sind auf 20 MiB begrenzt; Inhalt und optional Revision werden geprüft, das lokale Original bleibt unverändert. Die Referenz ist an Workspace, Sitzung und Anbieter-Verbindung gebunden und lässt sich nicht durch beliebige Anbieter-IDs ersetzen. Nach einem Timeout kann unklar sein, ob der Anbieter die Datei angenommen hat: Prüfen Sie die Referenz und wiederholen Sie nicht automatisch. Prüfen Sie den Status vor der Verwendung und löschen Sie nicht mehr benötigte Dateien ausdrücklich."
        }
        ("file_upload", "es-ES") => {
            "Esta operación crea un archivo remoto, pero no lo analiza. Cada entrada tiene un límite de 20 MiB; se comprueba el contenido y, opcionalmente, la revisión, sin modificar el original local. La referencia pertenece a este espacio de trabajo, sesión y conexión; no se puede sustituir por un ID arbitrario del proveedor. Tras un tiempo de espera, puede no saberse si se aceptó: comprueba la referencia y no repitas automáticamente. Consulta el estado antes de usar archivos en procesamiento y elimina expresamente los que ya no necesites."
        }
        ("file_upload", "hi-IN") => {
            "यह प्रक्रिया केवल रिमोट फ़ाइल बनाती है, उसका विश्लेषण नहीं करती। हर इनपुट अधिकतम 20 MiB है; सामग्री और वैकल्पिक revision की जाँच होती है और स्थानीय मूल फ़ाइल नहीं बदलती। हैंडल इसी workspace, session और provider connection से बँधा है; इसे मनमाने vendor ID से नहीं बदला जा सकता। timeout पर यह अस्पष्ट हो सकता है कि फ़ाइल स्वीकार हुई या नहीं—हैंडल जाँचें, अपने-आप दोबारा न भेजें। उपयोग से पहले प्रसंस्करण स्थिति देखें और अनुपयोगी फ़ाइल स्पष्ट रूप से हटाएँ।"
        }
        ("file_upload", "ar-SA") => {
            "تنشئ هذه العملية ملفًا بعيدًا ولا تحلله. الحد الأقصى لكل إدخال 20 MiB، ويُفحص المحتوى ويمكن التحقق من المراجعة، مع بقاء النسخة المحلية كما هي. يرتبط المعرّف بمساحة العمل والجلسة واتصال المزوّد، ولا يمكن استبداله بمعرّف عشوائي. قد يترك انتهاء المهلة قبول الملف غير معلوم؛ افحص المعرّف ولا تعِد المحاولة تلقائيًا. تحقق من الحالة قبل استخدام الملف أثناء معالجته، واحذف الملفات غير اللازمة صراحةً."
        }
        ("file_upload", "pt-BR") => {
            "Esta operação cria um arquivo remoto, mas não o analisa. Cada entrada pode ter até 20 MiB; o conteúdo é verificado e a revisão pode ser conferida, sem alterar o original local. A referência pertence a este workspace, sessão e conexão do provedor e não pode ser trocada por um ID arbitrário. Após um timeout, a aceitação remota pode ser desconhecida: confira a referência em vez de repetir automaticamente. Verifique o status antes de usar arquivos em processamento e exclua explicitamente os que não forem mais necessários."
        }

        ("file_status", "zh-CN") => {
            "只接受由同一工作区、会话和提供商连接创建的上传句柄。会返回远端就绪或过期状态，并刷新本地签名回执；不会下载文件内容，也不会重新提交状态未知的上传。"
        }
        ("file_status", "zh-TW") => {
            "只接受由同一工作區、工作階段與服務提供者連線建立的上傳代號。會回傳遠端就緒或到期狀態，並更新本機簽署收據；不會下載檔案內容，也不會重送狀態不明的上傳。"
        }
        ("file_status", "ja-JP") => {
            "同じワークスペース・セッション・接続で作成したアップロードハンドルだけを受け付けます。リモート側の準備状況や期限を返し、署名済みローカルレシートを更新します。内容のダウンロードや、結果不明のアップロード再送は行いません。"
        }
        ("file_status", "ko-KR") => {
            "같은 작업 공간·세션·제공자 연결에서 만든 업로드 핸들만 허용합니다. 원격 준비 상태와 만료를 알려 주고 서명된 로컬 영수증을 갱신합니다. 파일 내용을 내려받거나 결과가 불명확한 업로드를 다시 보내지 않습니다."
        }
        ("file_status", "fr-FR") => {
            "Accepte uniquement les références créées par le même espace, la même session et la même connexion. Indique si le fichier distant est prêt ou expiré et actualise le reçu local signé. Ne télécharge pas le contenu et ne renvoie pas un téléversement au résultat incertain."
        }
        ("file_status", "de-DE") => {
            "Akzeptiert nur Upload-Referenzen derselben Workspace-, Sitzungs- und Anbieter-Verbindung. Meldet Bereitschaft oder Ablauf der Remote-Datei und aktualisiert den signierten lokalen Beleg. Lädt keinen Inhalt herunter und sendet unklare Uploads nicht erneut."
        }
        ("file_status", "es-ES") => {
            "Solo acepta referencias creadas en el mismo espacio de trabajo, sesión y conexión. Informa de si el archivo remoto está listo o ha caducado y actualiza el recibo local firmado. No descarga el contenido ni vuelve a enviar una carga de resultado desconocido."
        }
        ("file_status", "hi-IN") => {
            "केवल उसी workspace, session और provider connection से बने upload handle स्वीकार होते हैं। दूरस्थ readiness या expiry बताता है और हस्ताक्षरित स्थानीय रसीद ताज़ा करता है। सामग्री डाउनलोड नहीं करता और अनिश्चित अपलोड दोबारा नहीं भेजता।"
        }
        ("file_status", "ar-SA") => {
            "لا يقبل إلا معرّفات رفع أُنشئت ضمن مساحة العمل والجلسة واتصال المزوّد نفسها. يعرض جاهزية الملف البعيد أو انتهاءه ويحدّث الإيصال المحلي الموقّع. لا ينزّل المحتوى ولا يعيد إرسال رفع نتيجته مجهولة."
        }
        ("file_status", "pt-BR") => {
            "Aceita somente referências criadas no mesmo workspace, sessão e conexão. Informa se o arquivo remoto está pronto ou expirou e atualiza o comprovante local assinado. Não baixa o conteúdo nem reenvia um upload de resultado desconhecido."
        }

        ("file_delete", "zh-CN") => {
            "只接受当前会话拥有的云端文件句柄。会请求删除远端资源并记录服务商回执，但不承诺清除服务商日志或备份。不会接受任意远端 ID 或跨服务商删除；请求失败时不会报告清理成功。"
        }
        ("file_delete", "zh-TW") => {
            "只接受目前工作階段擁有的雲端檔案代號。會要求刪除遠端資源並記錄服務提供者回覆，但不保證清除服務提供者的記錄或備份。不接受任意遠端 ID 或跨服務提供者刪除；請求失敗時不會回報清理成功。"
        }
        ("file_delete", "ja-JP") => {
            "このセッションが所有するクラウドファイルハンドルだけを受け付けます。リモート削除を依頼して応答を記録しますが、プロバイダーのログやバックアップの消去までは保証しません。任意 ID や別プロバイダーの削除はできず、失敗時に削除成功とは報告しません。"
        }
        ("file_delete", "ko-KR") => {
            "현재 세션이 소유한 클라우드 파일 핸들만 허용합니다. 원격 삭제를 요청하고 제공자 응답을 기록하지만 제공자 로그나 백업 삭제까지 보장하지는 않습니다. 임의 ID나 다른 제공자 파일은 삭제할 수 없고 실패를 성공으로 보고하지 않습니다."
        }
        ("file_delete", "fr-FR") => {
            "Accepte uniquement les références cloud appartenant à cette session. Demande la suppression distante et enregistre l’accusé du fournisseur, sans garantir l’effacement de ses journaux ou sauvegardes. Aucun ID arbitraire ni suppression inter-fournisseurs ; un échec n’est jamais présenté comme un nettoyage réussi."
        }
        ("file_delete", "de-DE") => {
            "Akzeptiert nur Cloud-Dateireferenzen dieser Sitzung. Fordert die Remote-Löschung an und speichert die Bestätigung, garantiert aber nicht die Entfernung von Anbieterprotokollen oder Sicherungen. Keine beliebigen IDs oder anbieterübergreifenden Löschungen; Fehler werden nicht als Erfolg gemeldet."
        }
        ("file_delete", "es-ES") => {
            "Solo acepta referencias cloud propiedad de esta sesión. Solicita borrar el recurso remoto y registra la respuesta, pero no garantiza que se eliminen los registros o copias de seguridad del proveedor. No admite IDs arbitrarios ni borrados entre proveedores; un fallo nunca se comunica como una limpieza correcta."
        }
        ("file_delete", "hi-IN") => {
            "केवल इस सत्र के स्वामित्व वाले क्लाउड फ़ाइल हैंडल स्वीकार होते हैं। दूरस्थ हटाने का अनुरोध और प्रदाता की पुष्टि दर्ज होती है, पर प्रदाता के लॉग या बैकअप मिटने की गारंटी नहीं है। मनमाने ID या दूसरे प्रदाता की फ़ाइल नहीं हटती; विफलता को सफल सफ़ाई नहीं बताया जाता।"
        }
        ("file_delete", "ar-SA") => {
            "لا يقبل إلا معرّفات الملفات السحابية المملوكة لهذه الجلسة. يطلب حذف المورد البعيد ويسجل إقرار المزوّد، لكنه لا يضمن محو السجلات أو النسخ الاحتياطية. لا يقبل معرّفات عشوائية أو حذفًا بين المزوّدين؛ ولا يُعرض الفشل على أنه نجاح."
        }
        ("file_delete", "pt-BR") => {
            "Aceita apenas referências de arquivos de nuvem pertencentes a esta sessão. Solicita a exclusão remota e registra a confirmação, sem garantir a remoção de logs ou backups do provedor. Não aceita IDs arbitrários nem exclusão entre provedores; uma falha não é apresentada como sucesso."
        }

        ("remote", "zh-CN") => {
            "此工具在云端运行；只发送明确提供的提示和输入，本地项目文件不会自动上传，也不会回退到本地执行。云端运行环境独立于 Agena 工作区，具体参数请查看工具输入结构。"
        }
        ("remote", "zh-TW") => {
            "此工具在雲端執行；只會傳送明確提供的提示與輸入，本機專案檔案不會自動上傳，也不會改在本機執行。雲端環境與 Agena 工作區分開，參數請參考工具輸入結構。"
        }
        ("remote", "ja-JP") => {
            "このツールはクラウドで動作します。明示したプロンプトと入力だけを送り、ローカルプロジェクトは自動送信せず、ローカル実行にも切り替わりません。クラウド環境は Agena の作業領域とは別です。指定可能な項目は入力スキーマを確認してください。"
        }
        ("remote", "ko-KR") => {
            "이 도구는 클라우드에서 실행됩니다. 명시적으로 제공한 프롬프트와 입력만 전송하며 로컬 프로젝트 파일은 자동 업로드되지 않고 로컬 실행으로 대체되지도 않습니다. 클라우드 환경은 Agena 작업 공간과 별개입니다. 옵션은 입력 스키마를 확인하세요."
        }
        ("remote", "fr-FR") => {
            "Cet outil s’exécute dans le cloud : seules les entrées explicitement fournies sont envoyées. Les fichiers du projet local ne sont pas téléversés automatiquement et aucune exécution locale de secours n’a lieu. L’environnement cloud est distinct d’Agena ; consultez le schéma pour les options acceptées."
        }
        ("remote", "de-DE") => {
            "Dieses Tool läuft in der Cloud und sendet nur ausdrücklich angegebene Eingaben. Lokale Projektdateien werden nicht automatisch hochgeladen; es gibt keinen lokalen Ausweichlauf. Die Cloud-Umgebung ist vom Agena-Workspace getrennt. Zulässige Optionen stehen im Eingabeschema."
        }
        ("remote", "es-ES") => {
            "Esta herramienta se ejecuta en la nube y solo envía las entradas indicadas expresamente. Los archivos locales no se suben automáticamente y no hay ejecución local alternativa. El entorno cloud está separado del espacio de trabajo de Agena. Consulta el esquema para ver las opciones admitidas."
        }
        ("remote", "hi-IN") => {
            "यह टूल क्लाउड में चलता है और केवल स्पष्ट रूप से दी गई सामग्री भेजता है। स्थानीय प्रोजेक्ट फ़ाइलें अपने-आप अपलोड नहीं होतीं और स्थानीय विकल्प पर स्विच नहीं होता। क्लाउड वातावरण Agena कार्यस्थान से अलग है। विकल्पों के लिए इनपुट स्कीमा देखें।"
        }
        ("remote", "ar-SA") => {
            "تعمل هذه الأداة في السحابة وترسل المدخلات المحددة صراحةً فقط. لا تُرفع ملفات المشروع المحلية تلقائيًا ولا يوجد تنفيذ محلي بديل. بيئة السحابة منفصلة عن مساحة عمل Agena؛ راجع مخطط الإدخال للخيارات المتاحة."
        }
        ("remote", "pt-BR") => {
            "Esta ferramenta roda na nuvem e envia somente as entradas informadas explicitamente. Arquivos locais não são enviados automaticamente e não há execução local alternativa. O ambiente é separado do workspace do Agena; consulte o esquema para ver as opções aceitas."
        }
        _ => "",
    }
}

fn help(key: &str, locale: &str) -> &'static str {
    match key {
        "image_understanding" | "document_understanding" => localized(key, locale),
        "file_upload" | "file_status" | "file_delete" => localized(key, locale),
        _ => localized("remote", locale),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_host::sdk::Plugin;

    #[test]
    fn every_provider_tool_has_complete_ui_translations() {
        let mut manifests = vec![
            crate::plugins::provided::chatgpt::ChatGptToolsPlugin::new().manifest(),
            crate::plugins::provided::claude::ClaudeToolsPlugin::new().manifest(),
            crate::plugins::provided::gemini::GeminiToolsPlugin::new().manifest(),
        ];
        for manifest in &mut manifests {
            localize_provider_manifest(manifest);
            assert!(!manifest.tools.is_empty());
            for tool in &manifest.tools {
                for locale in LOCALES {
                    let docs = tool.docs.translations.get(locale).unwrap_or_else(|| {
                        panic!("{}:{} missing {locale}", manifest.name, tool.name)
                    });
                    assert!(docs.summary.as_deref().is_some_and(|text| !text.is_empty()));
                    assert!(docs.help.as_deref().is_some_and(|text| !text.is_empty()));
                }
            }
        }
    }
}
