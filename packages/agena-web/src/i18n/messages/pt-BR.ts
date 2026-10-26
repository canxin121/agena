export default {
  browserNotifications: {
    "title": "Notificações do navegador",
    "description": "Receba uma notificação quando a IA terminar, fizer uma pergunta, precisar de aprovação ou falhar. Salvo neste navegador.",
    "enable": "Ativar notificações",
    "enabled": "As notificações do navegador estão ativadas.",
    "offer": "Ative notificações para respostas, perguntas e solicitações de aprovação da IA.",
    "test": "Enviar notificação de teste",
    "testBody": "As notificações estão funcionando. O Agena avisará quando precisar da sua atenção.",
    "deliveryFailed": "Não foi possível mostrar a notificação. Verifique o suporte do navegador, as permissões do site e as configurações do sistema.",
    "reconnect": "Reconectar notificações em segundo plano",
    "pushUnavailable": "O push em segundo plano não está conectado. Verifique a conexão e tente novamente; uma aba suspensa precisa de notificações push.",
    "testStatus": {"sending": "Enviando um teste pelo servidor e aguardando o recebimento pelo navegador…", "delivered": "O navegador recebeu a notificação e solicitou sua exibição. Se não aparecer no macOS, permita esse navegador em Ajustes do Sistema → Notificações e confira Foco.", "timeout": "O serviço push aceitou o teste, mas o navegador não confirmou o recebimento. Confira a conexão e as notificações do sistema.", "failed": "Não foi possível entregar o teste. Confira o erro e reconecte as notificações."},
    "iosInstall": "No iPhone/iPad, é necessário iOS/iPadOS 16.4 ou posterior. Adicione o Agena à Tela de Início, abra pelo ícone e ative as notificações.",
    "keepOpen": "O push funciona mesmo com a aba suspensa, descartada ou fechada. Permita notificações do seu navegador no sistema e confira Foco / Não Perturbe. Sair do navegador ou ficar sem conexão pode atrasar a entrega.",
    "status": {
      "default": "Permita notificações no navegador para receber alertas.",
      "granted": "Push conectado, inclusive com a aba suspensa ou descartada.",
      "localOnly": "Push desconectado. Somente uma página ativa pode enviar notificações.",
      "denied": "As notificações estão bloqueadas. Permita-as nas configurações do site no navegador e volte aqui.",
      "unsupported": "Este navegador não oferece suporte a notificações do sistema.",
      "insecure": "As notificações exigem HTTPS ou localhost. Abra o Agena usando um endereço seguro.",
      "disabled": "As notificações estão desativadas neste navegador."
    },
    "events": {
      "completed": "A IA terminou de responder.",
      "failed": "A IA parou com um erro. Abra a conversa para verificar.",
      "cancelled": "A execução da IA foi interrompida.",
      "permission": "A IA precisa da sua aprovação para continuar.",
      "question": "A IA está esperando sua resposta."
    }
  },
  keyboard: {
    workspaceTabs: "Abas do espaço de trabalho",
    tabActions: "Ações de {title}",
    duplicateTab: "Duplicar aba",
    splitTab: "Duplicar em um grupo à direita",
    detachTab: "Mover para um novo grupo",
    moveTabLeft: "Mover aba para a esquerda",
    moveTabRight: "Mover aba para a direita",
    moveGroupLeft: "Mover grupo para a esquerda",
    moveGroupRight: "Mover grupo para a direita",
    moveToGroup: "Mover para o grupo {number}",
    mergeIntoGroup: "Mesclar com o grupo {number}",
    replaceTab: "Substituir {title} por esta aba",
    moveFile: "Mover para…",
    panHint: "Shift + setas move a imagem ampliada. Esquerda/direita sem Shift troca a imagem.",
    panLeft: "Mover imagem para a esquerda",
    panRight: "Mover imagem para a direita",
    panUp: "Mover imagem para cima",
    panDown: "Mover imagem para baixo",
    transcriptNavigation: "Navegação de mensagens",
    previousMessage: "Mensagem anterior",
    nextMessage: "Próxima mensagem",
    copyMessage: "Copiar mensagem",
    selectMessageText: "Selecionar texto da mensagem",
    focusInput: "Ir para a entrada",
    openPlan: "Abrir plano",
    transcriptHint: "Alt+Shift+Cima/Baixo navega pelas mensagens; Home/End com Alt+Shift vai às extremidades. Na mensagem em foco, as setas navegam, Ctrl/Cmd+C copia e Esc volta à entrada.",
    enterPreview: "Interagir com a prévia",
    previewNewTab: "Interagir em uma nova aba",
    previewExitHint: "Na prévia, F6 ou Shift+F6 retorna a este controle.",
    previewExternalHint: "Esta prévia precisa de uma aba separada para uso do teclado. Use Ctrl/Cmd+W nela para voltar.",
    resizeWidth: "Ajustar largura da prévia",
    resizeHeight: "Ajustar altura da prévia",
    resizeBoth: "Ajustar prévia: {width} × {height}",
    resizeHint: "Na alça de tamanho, as setas ajustam 1 px; Shift+setas, 10 px; Home/End aplica os limites.",
    treeHint: "Árvores e listas: setas para navegar, esquerda/direita para recolher ou expandir, digite um nome para localizar, F2 renomeia linhas compatíveis e Shift+F10 abre ações. Tab acessa os botões da linha.",
    customizeShortcuts: "Personalizar atalhos do aplicativo",
    shortcutScope: "Salvo neste navegador. Foque Alterar e pressione o atalho; Esc cancela. Inclua Ctrl/Cmd ou Alt; Tab, Esc e F6 mantêm a navegação. O navegador e o sistema podem ter prioridade. O terminal mantém as sequências Ctrl; as alternativas Alt+Shift funcionam nele.",
    changeShortcut: "Alterar",
    disableShortcut: "Desativar",
    resetShortcut: "Restaurar",
    resetAllShortcuts: "Restaurar todos os atalhos",
    shortcutDisabled: "Desativado",
    shortcutRecording: "Pressione o atalho (Esc cancela)",
    shortcutSaved: "Atalhos salvos.",
    shortcutStorageError: "Não foi possível salvar neste navegador. Os atalhos anteriores continuam ativos.",
    shortcutDuplicate: "Já usado para “{action}”. Escolha outro atalho.",
    shortcutNeedsModifier: "Inclua Ctrl/Cmd ou Alt. Ctrl+Alt é reservado para entrada de texto.",
    shortcutReserved: "Esta tecla é reservada para navegação. Escolha outra combinação.",
    shortcutResetConflict: "Um atalho padrão conflita com outra personalização. Restaure essa ação primeiro ou restaure todos.",
    shortcutBrowserConflict: "Salvo. O navegador ou o sistema pode reservar este atalho; escolha Alt+Shift se o aplicativo não o receber.",
  },
  "content": {
    "live": "Saída em tempo real",
    "complete": "Saída concluída",
    "interrupted": "Saída interrompida",
    "follow": "Acompanhar saída",
    "copy": "Copiar",
    "label": "Saída da ferramenta",
    "waiting": "Aguardando saída…",
    "gap": "Parte da saída de origem não está disponível nos trechos retidos.",
    "window": "A saída anterior está fora desta janela de exibição; o conteúdo retido pode ser copiado.",
    "recovering": "Reconectando à saída…",
    "copyLimit": "Esta saída excede o limite de cópia de 16 MiB. Selecione um trecho menor para copiar."
  },
  "app": {
    "title": "Agena"
  },
  "common": {
    "add": "Adicionar",
    "cancel": "Cancelar",
    "close": "Fechar",
    "clear": "Limpar",
    "create": "Criar",
    "delete": "Excluir",
    "checkout": "Alternar para",
    "default": "Padrão",
    "other": "Outro",
    "save": "Salvar",
    "saving": "Salvando...",
    "saveFailed": "Falha ao salvar",
    "edit": "Editar",
    "open": "Abrir",
    "hide": "Ocultar",
    "reveal": "Mostrar",
    "from": "De",
    "to": "Para",
    "remove": "Remover",
    "rename": "Renomear",
    "confirm": "Confirmar",
    "working": "Processando...",
    "reloading": "Recarregando...",
    "search": "Buscar",
    "actions": "Ações",
    "searching": "Buscando...",
    "searchOptions": "Opções de busca",
    "all": "Todos",
    "selectAll": "Selecionar tudo",
    "invertSelection": "Inverter seleção",
    "replace": "Substituir",
    "discard": "Descartar",
    "undo": "Desfazer",
    "clean": "Limpar",
    "retry": "Tentar novamente",
    "connecting": "Conectando...",
    "loading": "Carregando...",
    "loadMore": "Carregar mais",
    "refresh": "Atualizar",
    "reset": "Redefinir",
    "reload": "Recarregar",
    "unlock": "Desbloquear",
    "continue": "Continuar",
    "back": "Voltar",
    "abort": "Interromper",
    "later": "Mais tarde",
    "nextSteps": "Próximas etapas",
    "useTerminal": "Usar o Terminal",
    "openTerminal": "Abrir o Terminal",
    "scanning": "Verificando...",
    "useDirectory": "Usar diretório",
    "use": "Usar",
    "searchActions": "Buscar ações",
    "select": "Selecionar",
    "noActionsFound": "Nenhuma ação encontrada.",
    "noOptionsFound": "Nenhuma opção encontrada.",
    "options": "Opções",
    "copy": "Copiar",
    "copied": "Copiado",
    "copyPath": "Copiar caminho",
    "view": "Visualizar",
    "apply": "Aplicar",
    "ignore": "Ignorar",
    "yes": "Sim",
    "no": "Não",
    "true": "true",
    "false": "false",
    "copyFailed": "Falha ao copiar",
    "failedToCopyToClipboard": "Falha ao copiar para a área de transferência",
    "previousPage": "Página anterior",
    "nextPage": "Próxima página",
    "currentPage": "Página atual",
    "previous": "Anterior",
    "next": "Próximo",
    "files": "Arquivos",
    "unknown": "Desconhecido",
    "moveUp": "Mover para cima",
    "moveDown": "Mover para baixo",
    "empty": "Vazio",
    "notSet": "(não definido)",
    "on": "Ativado",
    "key": "Chave",
    "value": "Valor",
    "integer": "Inteiro",
    "number": "Número",
    "optionPicker": {
      "searchPlaceholder": "Buscar...",
      "selectPlaceholder": "Selecionar...",
      "truncatedHelper": "Digite para buscar. Exibindo {shown} de {total}.",
      "emptyNoMatches": "Nenhum resultado.",
      "emptyNoPresetMatches": "Nenhuma opção predefinida encontrada. Use um valor personalizado abaixo.",
      "customGroupTitle": "Personalizado",
      "useValue": "Usar \"{value}\"",
      "setCustomValue": "Definir um valor personalizado"
    }
  },
  "codeBlock": {
    "copy": "Copiar",
    "hide": "Ocultar",
    "reveal": "Mostrar",
    "notSet": "(não definido)",
    "copyAria": "Copiar código",
    "copyDiagramSourceAria": "Copiar código-fonte do diagrama",
    "expand": "Expandir",
    "collapse": "Recolher",
    "expandLines": "Expandir ({lines} linhas)",
    "expandAria": "Expandir bloco de código",
    "collapseAria": "Recolher bloco de código",
    "toggleCodeAria": "Alternar bloco de código",
    "toggleDiagramAria": "Alternar diagrama",
    "expandDiagramAria": "Expandir diagrama"
  },
  "markdown": {
    "tableOfContents": "Índice"
  },
  "plugins": {
    "mountHost": {
      "loadingAria": "Carregando a interface do plugin",
      "errors": {
        "iframeLoadFailed": "Falha ao carregar o iframe da interface do plugin"
      }
    },
    "overlayLauncher": {
      "button": "Ferramentas de plugins",
      "openMenuAria": "Abrir o menu do painel de plugins",
      "closeMenuAria": "Fechar o menu do painel de plugins",
      "menuTitle": "Painéis de plugins",
      "hidePanel": "Ocultar o painel atual"
    }
  },
  "mcp": {
    "dialog": {
      "title": "Servidores MCP",
      "runtimeDescription": "Servidores MCP carregados pelo runtime Agena. O ciclo de conexão é gerenciado pela configuração do servidor.",
      "serversLabel": "servidores",
      "toolsLabel": "ferramentas",
      "description": "Inspecione os servidores do Model Context Protocol carregados pelo Agena",
      "loading": "Carregando o status do MCP...",
      "empty": "Nenhum servidor MCP configurado.",
      "actions": {
        "connect": "Conectar",
        "disconnect": "Desconectar",
        "authenticate": "Autenticar",
        "retry": "Tentar novamente",
        "refreshing": "Atualizando..."
      }
    },
    "status": {
      "connected": "conectado",
      "disabled": "desativado",
      "needsAuth": "needs_auth",
      "needsClientRegistration": "needs_client_registration",
      "failed": "falhou"
    }
  },
  "auth": {
    "serverAuthRequired": "É necessária a senha do servidor"
  },
  "login": {
    "title": "Agena",
    "unlockDescription": "Digite sua senha para desbloquear o Agena.",
    "subtitle": "Escolha um backend. Se estiver bloqueado, digite sua senha para continuar.",
    "backendLoadingTitle": "O backend está iniciando; aguarde",
    "backendLoadingDescription": "O backend para desktop ainda está inicializando. A página será restaurada automaticamente quando estiver pronto.",
    "backendLoadingRetry": "Tentar novamente agora",
    "backendLoadingToast": "O backend para desktop ainda está inicializando. Aguarde um momento.",
    "backendLabel": "Backend",
    "selectBackendTitle": "Backend",
    "searchBackends": "Buscar backends",
    "editBackend": "Editar",
    "newBackend": "Novo backend...",
    "newBackendHelp": "Adicionar a URL de um backend do Studio",
    "newBackendLabelPlaceholder": "ex.: Servidor de casa",
    "label": "Rótulo",
    "baseUrl": "URL base",
    "passwordPlaceholder": "Senha",
    "passwordRequired": "Senha obrigatória",
    "backendNotReachable": "Backend inacessível",
    "failedToUpdateBackend": "Falha ao atualizar o backend",
    "failedToRemoveBackend": "Falha ao remover o backend",
    "failedToAddBackend": "Falha ao adicionar o backend",
    "saveNewBackendFirst": "Salve o novo backend antes de continuar",
    "openRuntimeConfig": "Abrir a configuração do runtime",
    "removeBackendTitle": "Remover backend?"
  },
  "settings": {
    "title": "Configurações",
    "refresh": "Atualizar",
    "refreshAria": "Atualizar configurações",
    "unknownTab": "Aba de configurações desconhecida.",
    "emptyPlugins": "Nenhum plugin disponibiliza configurações.",
    "tabs": {
      "general": "Geral",
      "providers": "Provedores",
      "modelsProviders": "Modelos e provedores",
      "permissions": "Permissões",
      "activities": "Atividades",
      "memories": "Memórias",
      "usage": "Uso",
      "plugins": "Plugins",
      "pluginsTools": "Plugins e ferramentas",
      "mcpServer": "Servidor MCP",
      "runtimeSession": "Runtime e sessão",
      "interface": "Interface",
      "diagnostics": "Diagnósticos",
      "backends": "Backends",
      "appearance": "Aparência",
      "debug": "Depuração"
    },
    "groups": {
      "core": "Principal",
      "application": "Aplicativo",
      "system": "Sistema"
    },
    "debug": {
      "title": "Ferramentas de depuração",
      "intro": "Use esta página para verificar rapidamente o comportamento da interface e o funcionamento das interações.",
      "diagnostics": {
        "title": "Diagnósticos do runtime",
        "description": "Inspecione os caminhos do OpenCode/Studio, a versão e os valores de ambiente usados pelo backend.",
        "fields": {
          "directory": "Substituição do diretório",
          "studioSettingsPath": "Caminho das configurações do Studio"
        },
        "placeholders": {
          "directory": "Exemplo: D:\\git"
        },
        "actions": {
          "refresh": "Atualizar diagnósticos",
          "loading": "Carregando..."
        },
        "fullPayload": "Dados completos de diagnóstico"
      },
      "toast": {
        "title": "Bancada de testes de notificações",
        "description": "Acione diferentes tipos de notificação, callbacks de ações e o comportamento de deduplicação.",
        "fields": {
          "message": "Mensagem",
          "timeoutMs": "Duração (ms)",
          "actionLabel": "Rótulo da ação"
        },
        "placeholders": {
          "message": "Deixe vazio para gerar uma mensagem automaticamente",
          "actionLabel": "Desfazer"
        },
        "options": {
          "uniqueMessage": "Adicionar sufixo exclusivo"
        },
        "actions": {
          "info": "Exibir informação",
          "success": "Exibir sucesso",
          "error": "Exibir erro",
          "withAction": "Exibir notificação com ação",
          "persistent": "Exibir notificação persistente",
          "burst": "Disparar 3 vezes (deduplicação)"
        },
        "hints": {
          "dedupe": "Notificações do mesmo tipo e mensagem são deduplicadas em 1,5 s; até 4 itens ficam visíveis.",
          "action": "Ao clicar no botão da ação, uma notificação de sucesso é exibida em seguida."
        },
        "defaults": {
          "info": "Notificação de informação de depuração",
          "success": "Notificação de sucesso de depuração",
          "error": "Notificação de erro de depuração",
          "action": "Notificação de ação de depuração",
          "persistent": "Notificação persistente de depuração",
          "burst": "Notificação repetida de depuração",
          "actionFollowup": "Callback da ação executado"
        }
      }
    },
    "pluginSettings": {
      "title": "Configurações do plugin",
      "subtitle": "Cada plugin persiste sua própria configuração",
      "noSchemaPublished": "Nenhum plugin publicou um `settingsSchema` ainda.",
      "fields": {
        "plugin": "Plugin"
      },
      "searchPlugins": "Buscar plugins",
      "loadingConfig": "Carregando configuração do plugin...",
      "noFieldsDeclared": "Este plugin não declarou nenhum campo de configuração.",
      "editAsJson": "Editar como JSON.",
      "errors": {
        "failedToLoadConfig": "Falha ao carregar a configuração do plugin",
        "failedToSaveConfig": "Falha ao salvar a configuração do plugin",
        "fixInvalidListFields": "Corrija os campos de lista inválidos antes de salvar.",
        "fixInvalidJsonFields": "Corrija os campos JSON inválidos antes de salvar.",
        "invalidJson": "JSON inválido",
        "row": {
          "duplicateKey": "Linha {row}: chave \"{key}\" duplicada.",
          "invalidEnum": "Linha {row}: valor de enum inválido.",
          "invalidNumber": "Linha {row}: número inválido.",
          "invalidInteger": "Linha {row}: número inteiro inválido.",
          "useTrueFalse": "Linha {row}: use true ou false."
        }
      },
      "toasts": {
        "saved": "Configuração do plugin salva"
      }
    },
    "backendsPanel": {
      "title": "Backends",
      "description": "Gerencie os endpoints de backend do Studio. Ao trocar o backend ativo, o aplicativo será recarregado.",
      "activeBackend": "Backend ativo",
      "backendFallback": "Backend",
      "activeBadge": "Ativo",
      "noBackends": "Nenhum backend configurado.",
      "dialogs": {
        "add": {
          "title": "Adicionar backend",
          "description": "Adicione uma URL de backend do Studio e, se quiser, ative-a.",
          "label": "Nome",
          "labelPlaceholder": "Ex.: Studio local",
          "baseUrl": "URL base",
          "baseUrlPlaceholder": "http://127.0.0.1:3210",
          "examples": "Exemplos: `http://127.0.0.1:3210` ou `https://studio.example.com`",
          "setActive": "Definir como ativo (recarregar)"
        },
        "edit": {
          "title": "Editar backend",
          "description": "Atualize o nome ou a URL do backend. Alterar a URL do backend ativo recarrega o aplicativo.",
          "label": "Nome",
          "baseUrl": "URL base"
        }
      },
      "confirmRemove": {
        "title": "Remover backend?"
      },
      "toasts": {
        "failedToAdd": "Falha ao adicionar backend",
        "added": "Backend adicionado",
        "addedAndActivated": "Backend adicionado e ativado",
        "failedToUpdate": "Falha ao atualizar backend",
        "updated": "Backend atualizado",
        "switching": "Trocando de backend...",
        "failedToRemove": "Falha ao remover backend",
        "removed": "Backend removido",
        "refreshed": "Lista de backends atualizada"
      }
    },
    "desktopRuntime": {
      "title": "Runtime do backend para desktop",
      "description": "Configure o runtime do backend, a conexão com o OpenCode e os níveis de log. Ao salvar, o backend para desktop será reiniciado.",
      "fields": {
        "host": "Host",
        "port": "Porta",
        "corsOrigins": "Origens CORS",
        "corsAllowAll": "Permitir todas as origens (*)",
        "backendLogLevel": "Nível de log do backend",
        "uiPassword": "Senha de autenticação",
        "skipOpencodeStart": "Não iniciar o OpenCode automaticamente",
        "autostartOnBoot": "Iniciar ao entrar no sistema"
      },
      "placeholders": {
        "host": "127.0.0.1",
        "port": "3210",
        "corsOrigins": "Uma origem por linha, por exemplo: http://localhost:5173",
        "uiPassword": "Deixe vazio para desativar o login por senha"
      },
      "options": {
        "logLevel": {
          "default": "padrão (INFO)",
          "DEBUG": "DEBUG",
          "INFO": "INFO",
          "WARN": "WARN",
          "ERROR": "ERROR"
        }
      },
      "search": {
        "searchLogLevels": "Buscar níveis de log"
      },
      "corsHint": "Aceita separadores por vírgula ou quebra de linha. Deixe vazio para usar os valores padrão.",
      "saveAndRestart": "Salvar e reiniciar o backend",
      "toasts": {
        "invalidPort": "A porta deve ser um número inteiro entre 1 e 65535",
        "invalidOpencodePort": "A porta do OpenCode deve ser um número inteiro entre 1 e 65535",
        "loadFailed": "Falha ao carregar a configuração do runtime para desktop",
        "saveFailed": "Falha ao salvar a configuração do runtime para desktop",
        "savedAndRestarted": "Configuração do runtime para desktop salva. Reiniciando o backend..."
      },
      "updates": {
        "title": "Atualizações de versão",
        "description": "Verifique atualizações do serviço de backend e do instalador para desktop nas versões do GitHub. As atualizações do instalador são aplicadas com a instalação do novo pacote.",
        "fields": {
          "autoCheck": "Verificar atualizações automaticamente",
          "autoPrompt": "Exibir avisos de atualização automaticamente",
          "autoServiceInstall": "Instalar atualizações do serviço automaticamente",
          "autoInstallerInstall": "Baixar e iniciar automaticamente atualizações do instalador para desktop"
        },
        "sections": {
          "service": "Versão do serviço",
          "installer": "Versão do instalador"
        },
        "status": {
          "upToDate": "Atualizado",
          "updateAvailable": "Atualização disponível"
        },
        "actions": {
          "checkNow": "Verificar agora",
          "checking": "Verificando...",
          "openRelease": "Abrir notas da versão",
          "ignoreThisVersion": "Ignorar esta versão",
          "remindLater": "Lembrar mais tarde (24 h)",
          "clearSuppression": "Limpar ignorar/adiar",
          "updateServiceNow": "Atualizar serviço agora",
          "updatingService": "Atualizando serviço...",
          "installDesktopNow": "Instalar atualização para desktop",
          "preparingInstaller": "Preparando o instalador..."
        },
        "versionLine": "Versão atual {current} -> mais recente {latest}",
        "targetLine": "Destino: {target}",
        "channelLine": "Canal: {channel}",
        "serviceHint": "A atualização substitui o serviço de backend e o reinicia automaticamente.",
        "installerHint": "Ao iniciar o instalador, o aplicativo será fechado antes da abertura da interface de instalação.",
        "suppression": {
          "ignored": "Esta versão foi ignorada nos avisos e nas instalações automáticas.",
          "snoozed": "Os lembretes de atualização foram adiados até {until}."
        },
        "progress": {
          "preparing": "Preparando a atualização...",
          "indeterminate": "Baixando o pacote...",
          "percent": "{percent}% baixado"
        },
        "toasts": {
          "checkedNoUpdate": "Verificação concluída. Você está usando a versão mais recente.",
          "checkFailed": "Falha ao verificar atualizações: {error}",
          "bothAvailable": "A nova versão {version} está disponível para o serviço e o instalador.",
          "serviceAvailable": "A nova versão {version} do serviço está disponível.",
          "installerAvailable": "A nova versão {version} do instalador está disponível.",
          "ignoredVersion": "A versão atual será ignorada nos lembretes.",
          "ignoreVersionFailed": "Falha ao ignorar esta versão: {error}",
          "reminderSnoozed": "Os lembretes de atualização foram adiados por 24 horas.",
          "reminderSnoozeFailed": "Falha ao adiar os lembretes de atualização: {error}",
          "reminderSuppressionCleared": "As configurações para ignorar ou adiar atualizações foram limpas.",
          "reminderSuppressionClearFailed": "Falha ao limpar as configurações para ignorar ou adiar atualizações: {error}",
          "serviceUpdated": "Atualização do serviço aplicada e backend reiniciado.",
          "serviceUpdateFailed": "Falha ao atualizar o serviço: {error}",
          "installerLaunching": "Pacote do instalador baixado. Iniciando o instalador...",
          "installerUpdateFailed": "Falha ao atualizar o instalador para desktop: {error}",
          "autoServiceFailed": "Falha na atualização automática do serviço: {error}",
          "autoInstallerFailed": "Falha na atualização automática do instalador para desktop: {error}",
          "openReleaseFailed": "Não foi possível abrir a página da versão"
        }
      }
    },
    "tui": {
      "interfaceDescription": "Configurações de idioma, cores, gráficos e temas de plugins para clientes TUI.",
      "runtimeDescription": "Versões dos clientes provedores e configurações de compactação de sessão compartilhadas com o runtime TUI.",
      "diagnosticsDescription": "Configurações de rastreamento e diagnóstico do runtime disponíveis no Studio de configurações TUI.",
      "clientVersionsTitle": "Versões dos clientes provedores",
      "clientVersionsDescription": "Fixe ou atualize as versões instaladas dos clientes usadas pelas integrações de provedores.",
      "refreshing": "Atualizando…",
      "refreshFromNpm": "Atualizar pelo npm",
      "compactionTitle": "Compactação de sessão",
      "compactionDescription": "Controle a compactação automática do contexto e a reserva de tokens antes da compactação.",
      "effectiveReadOnly": "As configurações efetivas são somente para leitura. Escolha Global, Espaço de trabalho ou Sessão para editar uma camada persistida.",
      "fields": {
        "locale": "Idioma",
        "localeDescription": "Idioma usado pelo runtime e pela TUI.",
        "colorScheme": "Esquema de cores",
        "colorSchemeDescription": "Esquema de cores claro, escuro ou do sistema para a renderização da TUI.",
        "graphics": "Gráficos",
        "graphicsDescription": "Ative ou desative a renderização gráfica no terminal.",
        "theme": "Tema",
        "themeDescription": "Nome ou identificador do tema da TUI.",
        "codexVersion": "Versão do cliente Codex",
        "codexVersionDescription": "Restrição de versão ou lançamento selecionado para o cliente Codex.",
        "claudeVersion": "Versão do cliente Claude",
        "claudeVersionDescription": "Restrição de versão ou lançamento selecionado para o cliente Claude.",
        "geminiVersion": "Versão do cliente Gemini",
        "geminiVersionDescription": "Restrição de versão ou lançamento selecionado para o cliente Gemini.",
        "autoCompaction": "Compactação automática",
        "autoCompactionDescription": "Compacta a sessão automaticamente quando o limite de contexto exigir.",
        "reservedTokens": "Tokens reservados",
        "reservedTokensDescription": "Tokens mantidos disponíveis antes de iniciar a compactação automática.",
        "tracingFilter": "Filtro de rastreamento",
        "tracingFilterDescription": "Diretiva de filtro de rastreamento usada pelo runtime.",
        "tracingDatabase": "Banco de dados de rastreamento",
        "tracingDatabaseDescription": "Destino ou coletor do banco de dados de rastreamento.",
        "tracingAdapter": "Adaptador de rastreamento",
        "tracingAdapterDescription": "Adaptador de rastreamento usado para exportar eventos do runtime."
      }
    },
    "appearance": {
      "intro": "Configurações de tema e tipografia.",
      "language": {
        "label": "Idioma",
        "help": "Aplica-se à interface do Studio.",
        "options": {
          "enUS": "Inglês",
          "zhCN": "Chinês simplificado",
          "esES": "Espanhol (Espanha)",
          "hiIN": "Hindi (Índia)",
          "arSA": "Árabe (Arábia Saudita)",
          "ptBR": "Português (Brasil)",
          "frFR": "Francês (França)"
        }
      },
      "theme": {
        "label": "Tema",
        "useSystem": "Usar tema do sistema",
        "pickerTitle": "Tema",
        "pickerSearch": "Buscar temas",
        "options": {
          "light": "Claro",
          "dark": "Escuro"
        }
      },
      "fonts": {
        "label": "Fontes",
        "ui": "Fonte da interface",
        "mono": "Fonte monoespaçada",
        "search": "Buscar fontes",
        "options": {
          "system": "Sistema",
          "ibmPlexSans": "IBM Plex Sans",
          "atkinson": "Atkinson Hyperlegible",
          "serif": "Serifada",
          "ibmPlexMono": "IBM Plex Mono",
          "jetbrainsMono": "JetBrains Mono"
        }
      },
      "sizing": {
        "label": "Tamanhos",
        "fontSize": "Tamanho da fonte (%)",
        "padding": "Espaçamento interno (%)",
        "cornerRadius": "Raio dos cantos (px)",
        "inputBarOffset": "Deslocamento da barra de entrada (px)"
      },
      "chat": {
        "label": "Conversa",
        "showTimestamps": "Mostrar horários",
        "showReasoning": "Mostrar etapas de raciocínio",
        "autoCollapseActivity": "Recolher atividades automaticamente após a resposta do assistente",
        "transcriptVim": "Modo Vim na conversa",
        "transcriptVimHint": "Navegue pela conversa usando o teclado. Ativado por padrão no computador e desativado em dispositivos móveis.",
        "activityDetails": "Padrões de expansão de seções",
        "activityTable": {
          "type": "Tipo de seção",
          "expand": "Expandir por padrão"
        },
        "pluginActivityKind": "Tipo de apresentação de seção registrado por um plugin.",
        "activityKinds": {
          "reasoning": {
            "label": "Raciocínio",
            "description": "Conteúdo de raciocínio das seções think."
          },
          "operation": {
            "label": "Operação de ferramenta",
            "description": "Seções de chamadas de ferramentas; cada ferramenta pode substituir esta opção abaixo."
          },
          "resource": {
            "label": "Recurso",
            "description": "Arquivos, imagens e outros recursos das seções file_ref."
          },
          "skill_reference": {
            "label": "Referência de comando",
            "description": "Nomes, descrições e metadados de referência das seções skill_ref."
          },
          "interaction": {
            "label": "Interação do usuário",
            "description": "Seções de interação; solicitações pendentes permanecem abertas."
          },
          "hook": {
            "label": "Hook",
            "description": "Informações do ciclo de vida de plugins nas seções hook."
          },
          "error": {
            "label": "Erro",
            "description": "Detalhes estruturados de falhas nas seções error."
          },
          "notice": {
            "label": "Aviso",
            "description": "Seções notice, system_notification e compaction."
          },
          "text": {
            "label": "Texto intermediário",
            "description": "Trechos de texto do assistente antes da resposta final."
          }
        },
        "toolDetails": "Padrões das ferramentas",
        "toolDetailsTable": {
          "tool": "Ferramenta",
          "expand": "Expandir por padrão"
        },
        "searchTools": "Buscar ferramentas",
        "refreshTools": "Atualizar catálogo de ferramentas",
        "loadingTools": "Carregando ferramentas registradas...",
        "noTools": "Nenhuma ferramenta correspondente.",
        "resetToolDefault": "Usar padrão herdado",
        "toolDetailsCount": "{shown} de {total} ferramentas",
        "toolDetailsHint": "Cada ferramenta de execução registrada e cada função da Tool API têm seu próprio estado de expansão padrão. Redefinir remove a substituição específica e restaura o padrão herdado da categoria."
      }
    }
  },
  "help": {
    "dialog": {
      "title": "Ajuda",
      "description": "Atalhos de teclado e ações rápidas",
      "quickActions": {
        "title": "Ações rápidas",
        "badge": "Ação",
        "reloadConfig": "Recarregar o Agena Runtime",
        "themeLight": "Tema: claro",
        "themeDark": "Tema: escuro",
        "themeSystem": "Tema: sistema"
      },
      "sections": {
        "navigationCommands": "Navegação e comandos",
        "transcriptVim": "Navegação pela conversa",
        "transcriptSelection": "Seleção e cópia na conversa",
        "sessionManagement": "Gerenciamento de sessões",
        "interface": "Interface"
      },
      "shortcuts": {
        "showKeyboardShortcuts": "Mostrar atalhos de teclado",
        "toggleSessionSidebar": "Alternar barra lateral de sessões",
        keyboardRegions: 'Alternar entre navegação, barra lateral, abas e painéis',
        focusControls: 'Focar o próximo controle / controle anterior',
        optionNavigation: 'Navegar pelas opções e selecionar um item',
        scrollRegion: 'Rolar o diálogo ou a área de rolagem em foco',
        workspaceTabs: 'Alternar ou fechar a aba de trabalho em foco',
        tabContextMenu: 'Abrir as ações da aba de trabalho em foco',
        "openChat": "Abrir conversa",
        "openPreview": "Abrir pré-visualização",
        "createNewSession": "Criar nova sessão",
        "sendMessage": "Enviar mensagem",
        "focusChatInput": "Focar no campo de mensagem",
        "abortActiveRunDouble": "Limpar um rascunho ou interromper a execução ativa",
        "cycleTheme": "Alternar tema (claro → escuro → sistema)",
        "openFiles": "Abrir arquivos",
        "openTerminal": "Abrir Terminal",
        "openGitPanel": "Abrir painel do Git",
        "openSettings": "Abrir configurações",
        "vimModes": "Entrar no modo INSERT / voltar à navegação da conversa",
        "vimMotions": "Mover por caractere ou linha visual, com contagem opcional",
        "vimMessages": "Ir para a mensagem anterior / seguinte",
        "vimBounds": "Ir para o primeiro, o nó numerado ou o último nó da conversa",
        "vimViewport": "Ir para referências da área visível ou posicionar o cursor nelas",
        "vimScroll": "Rolar meia página, páginas inteiras ou linhas individuais",
        "vimSearch": "Buscar para frente / para trás e repetir a busca",
        "vimStructure": "Expandir ou recolher a parte atual da conversa",
        "vimVisual": "Iniciar seleção Visual por caractere, linha ou bloco",
        "vimYank": "Copiar uma seleção, o nó atual ou uma quantidade de nós",
        "vimTextObjects": "Selecionar palavras, parágrafos, trechos Markdown ou mensagens inteiras",
        "vimWords": "Mover entre palavras Unicode ou WORDs separadas por espaços em branco",
        "openPlan": "Abrir o plano da sessão atual",
        "vimReselect": "Trocar a extremidade da seleção Visual / restaurar a última seleção",
        "vimFind": "Localizar um caractere e repetir a busca",
        "vimJumps": "Navegar para trás / para frente no histórico de saltos"
      }
    }
  },
  "chat": {
    "messages": {
      "empty": {
        "title": "Selecione uma sessão",
        "description": "Use o painel de sessões para escolher ou criar uma.",
        "actionLabel": "Abrir painel de sessões",
        "desktopDescription": "Use o painel à esquerda para escolher ou criar uma."
      },
      "loadingOlder": "Carregando mensagens anteriores...",
      "loadOlder": "Carregar mensagens anteriores",
      "historyStart": "Início da conversa",
      "activity": {
        "moreCount": "+ mais {count}...",
        "toolInvocation": {
          "toolNames": {
            "bash": "Executar comando",
            "edit": "Editar arquivo",
            "applyPatch": "Aplicar patch",
            "replaceText": "Substituir texto",
            "read": "Ler arquivo",
            "write": "Gravar arquivo",
            "list": "Listar arquivos",
            "glob": "Localizar arquivos",
            "search": "Buscar",
            "grep": "Grep",
            "fetch": "Buscar URL",
            "question": "Pergunta",
            "agent": "Agente"
          },
          "summary": {
            "askedQuestions": "{count} pergunta(s) feita(s)"
          },
          "sections": {
            "input": "Entrada",
            "diff": "Diff",
            "diffCode": "diff",
            "error": "Erro",
            "output": "Saída"
          },
          "wrap": {
            "label": "Quebra de linha",
            "enable": "Ativar quebra de linha",
            "disable": "Desativar quebra de linha"
          },
          "status": {
            "failed": "Falhou",
            "completedWithoutOutput": "(Concluído sem saída)",
            "stoppedSessionEnded": "(Interrompido: sessão encerrada)",
            "running": "Em execução..."
          }
        },
        "metaInvocation": {
          "types": {
            "snapshot": "Snapshot",
            "patch": "Patch",
            "retry": "Nova tentativa",
            "compaction": "Compactação",
            "meta": "Metadados"
          },
          "summary": {
            "files": "{count} arquivo(s)",
            "attempt": "tentativa {count}",
            "auto": "automático",
            "manual": "manual"
          }
        },
        "reasoningInvocation": {
          "types": {
            "thinking": "Raciocínio"
          }
        }
      },
      "controls": {
        "expandNextCount": "Carregar mais {count} partes",
        "expandNextLead": "Carregar",
        "expandNextTail": "mais partes",
        "collectAll": "Carregar todas as partes",
        "collapseParts": "Recolher partes",
        "collapsePartsHint": "Manter as 5 partes mais recentes e interações pendentes; ocultar as partes anteriores",
        "pageSizeInputLabel": "Partes anteriores a carregar (1–50)"
      },
      "optimistic": {
        "sending": "enviando...",
        "queued": "na fila",
        "sent": "enviada"
      }
    },
    "skills": {
      "attach": "Anexar uma referência de Skill",
      "title": "Skills",
      "search": "Pesquisar Skills",
      "empty": "Nenhuma Skill encontrada",
      "referenceDescription": "Anexe uma referência de Skill. As instruções serão lidas quando necessário.",
      "loadFailed": "Falha ao carregar as Skills",
      "attachFailed": "Não foi possível anexar a referência de Skill",
      "countLimit": "Uma mensagem pode incluir no máximo {count} referências de Skill"
    },
    "page": {
      "nav": {
        "previousUserMessage": "Mensagem anterior do usuário",
        "nextUserMessage": "Próxima mensagem do usuário",
        "bottom": "Fim"
      },
      "transcriptSearch": {
        "open": "Buscar na conversa",
        "previous": "Correspondência anterior",
        "next": "Próxima correspondência"
      },
      "attachments": "Anexos",
      "attachmentsWithCount": "Anexos ({count})",
      "tools": "Ferramentas",
      "usage": {
        "tokensSuffix": "{value} tokens"
      },
      "queue": {
        "title": "{count} mensagens na fila",
        "hint": "Enviadas em ordem após a resposta atual",
        "order": "Ordem de envio",
        "loading": "Carregando mensagem…",
        paused: "Pausada",
        pausedHint: "As mensagens são mantidas. Ao retomar, serão enviadas em ordem. Novas mensagens também ficam na fila durante a pausa.",
        resume: "Retomar fila",
        pause: "Pausar fila",
        clear: "Limpar fila",
        edit: "Editar mensagem na fila",
        editHint: "Edite o texto e os anexos. Ao salvar, a posição na fila é mantida.",
        attachmentUnavailable: "Este anexo está indisponível. Substitua ou remova antes de salvar.",
        attachmentName: "Nome do anexo",
        attachmentDelivery: "Forma de envio",
        reference: "Referência ao arquivo",
        modelInput: "Entrada do modelo",
        replaceAttachment: "Substituir arquivo",
        addAttachment: "Adicionar anexo",
        remove: "Excluir mensagem pendente",
        stoppedTitle: "Resposta interrompida",
        stoppedHint: "Há {count} mensagens na fila. Enviar a próxima ou continuar parado?",
        keepStopped: "Continuar parado e manter fila",
        sendNext: "Enviar próxima",
      },
      "primary": {
        "send": "Enviar",
        "sendMessage": "Enviar mensagem",
        "stop": "Parar",
        "queuedMessages": "{count} na fila",
        "stopRun": "Parar execução"
      }
    },
    "sessionError": {
      "classification": {
        "contextOverflow": "Contexto excedido",
        "providerAuth": "Erro de autenticação",
        "network": "Erro de rede",
        "providerApi": "Erro do provedor",
        "sessionError": "Erro na sessão"
      },
      "body": {
        "withCode": "[{code}] Erro na sessão",
        "default": "Erro na sessão"
      },
      "actions": {
        "copyDetails": "Copiar detalhes",
        "dismiss": "Dispensar"
      }
    },
    "sidebar": {
      "errors": {
        "failedToLoadSessions": "Falha ao carregar as sessões"
      },
      "addDirectoryDialog": {
        "title": "Adicionar diretório",
        "description": "Adicione o caminho de um diretório para exibir as sessões.",
        "placeholders": {
          "path": "/path/to/directory"
        }
      },
      "header": {
        "directoriesTitle": "Diretórios",
        "prevPage": "Página anterior de diretórios",
        "nextPage": "Próxima página de diretórios",
        "addDirectory": "Adicionar diretório",
        "refresh": "Atualizar",
        "searchPlaceholder": "Buscar diretórios ou sessões",
        "searchAria": "Buscar diretórios ou sessões",
        "clearSearch": "Limpar busca"
      },
      "directoriesList": {
        "empty": {
          "title": "Nenhum diretório ainda",
          "description": "Adicione o caminho de um diretório para começar."
        },
        "search": {
          "sessionsTitle": "Sessões",
          "noMatchingSessions": "Nenhuma sessão correspondente.",
          "loadingDirectories": "Carregando diretórios..."
        },
        "noMatchingDirectories": {
          "title": "Nenhum diretório correspondente",
          "description": "Tente buscar outro termo."
        },
        "expandDirectory": "Expandir diretório",
        "collapseDirectory": "Recolher diretório",
        "activeSessions": "Sessões ativas",
        "activity": {
          "running": "Em execução",
          "attention": "Precisa de atenção",
          "mixed": "Em execução e precisa de atenção",
          "runningTooltip": "Este diretório tem sessões em execução",
          "attentionTooltip": "Este diretório tem sessões que precisam de atenção",
          "mixedTooltip": "Este diretório tem sessões em execução e sessões que precisam de atenção"
        },
        "directoryActions": "Ações do diretório",
        "loadingSessions": "Carregando sessões...",
        "noSessionsYet": "Ainda não há sessões.",
        "pinnedTitle": "Fixadas",
        "recentTitle": "Recentes",
        "prevSessionsPage": "Página anterior de sessões",
        "nextSessionsPage": "Próxima página de sessões"
      },
      "directoryActions": {
        "menuTitle": "Ações do diretório",
        "openFiles": {
          "label": "Abrir em Arquivos",
          "description": "Usar este diretório como espaço de trabalho atual em Arquivos"
        },
        "copyPath": {
          "description": "Copiar o caminho do diretório para a área de transferência"
        },
        "refresh": {
          "label": "Atualizar",
          "description": "Recarregar as sessões deste diretório"
        },
        "newSession": {
          "label": "Nova sessão",
          "description": "Criar uma nova sessão neste diretório"
        },
        "remove": {
          "label": "Remover diretório",
          "description": "Remover este diretório da barra lateral",
          "confirmTitle": "Remover diretório?",
          "confirmDescription": "Remover este diretório da barra lateral."
        }
      },
      "sessionActions": {
        "menuTitle": "Ações da sessão",
        "open": {
          "description": "Abrir esta sessão no espaço de trabalho"
        },
        "rename": {
          "label": "Renomear sessão",
          "description": "Alterar o título da sessão"
        },
        "copyTranscript": {
          "label": "Copiar transcrição",
          "description": "Copiar a conversa completa"
        },
        "exportTranscript": {
          "label": "Exportar transcrição",
          "description": "Baixar a transcrição em Markdown"
        },
        "fork": {
          "label": "Criar sessão derivada",
          "description": "Criar uma nova sessão a partir desta conversa"
        },
        "share": {
          "label": "Compartilhar sessão",
          "description": "Criar um link público"
        },
        "unshare": {
          "label": "Cancelar compartilhamento",
          "description": "Remover o link público"
        },
        "copyShareLink": {
          "label": "Copiar link de compartilhamento",
          "description": "Copiar o link de compartilhamento atual"
        },
        "openShareLink": {
          "label": "Abrir link de compartilhamento",
          "description": "Abrir o link de compartilhamento em uma nova guia"
        },
        "favorite": {
          "label": "Adicionar aos favoritos",
          "description": "Adicionar aos favoritos"
        },
        "unfavorite": {
          "label": "Remover dos favoritos",
          "description": "Remover dos favoritos"
        },
        "pin": {
          "label": "Fixar sessão",
          "description": "Manter no topo"
        },
        "unpin": {
          "label": "Desafixar sessão",
          "description": "Remover da lista de itens fixados"
        },
        "delete": {
          "label": "Excluir sessão",
          "description": "Esta ação não pode ser desfeita",
          "confirmTitle": "Excluir sessão?",
          "confirmDescription": "Esta ação não pode ser desfeita.",
          "confirmText": "Excluir"
        }
      },
      "contextMenu": {
        "menuTitle": "Ações da barra lateral",
        "expandAll": "Expandir todos os diretórios",
        "collapseAll": "Recolher todos os diretórios"
      },
      "footers": {
        "favorites": {
          "toggleAria": "Alternar sessões favoritas",
          "title": "Favoritas",
          "prevPage": "Página anterior de favoritas",
          "nextPage": "Próxima página de favoritas",
          "empty": "Nenhuma sessão favorita."
        },
        "recent": {
          "toggleAria": "Alternar sessões recentes",
          "title": "Recentes",
          "prevPage": "Página anterior de recentes",
          "nextPage": "Próxima página de recentes",
          "empty": "Nenhuma sessão recente."
        },
        "pinned": {
          "toggleAria": "Alternar sessões fixadas",
          "title": "Fixadas",
          "prevPage": "Página anterior de fixadas",
          "nextPage": "Próxima página de fixadas",
          "empty": "Nenhuma sessão fixada."
        },
        "running": {
          "toggleAria": "Alternar sessões em execução",
          "title": "Em execução",
          "prevPage": "Página anterior de sessões em execução",
          "nextPage": "Próxima página de sessões em execução",
          "empty": "Nenhuma sessão em execução."
        }
      },
      "multiSelect": {
        "on": "Seleção múltipla ativada",
        "off": "Seleção múltipla desativada",
        "selectedCount": "{count} selecionados",
        "sessionCount": "{count} sessões",
        "directoryCount": "{count} diretórios",
        "actions": {
          "enterMultiSelect": "Selecionar vários",
          "exitMultiSelect": "Concluir",
          "enterSessionMultiSelect": "Selecionar sessões",
          "exitSessionMultiSelect": "Concluir seleção de sessões",
          "enterSessionMultiSelectDescription": "Selecionar várias sessões neste diretório",
          "exitSessionMultiSelectDescription": "Sair da seleção de sessões deste diretório",
          "deleteSelected": "Excluir selecionados"
        },
        "confirmDelete": {
          "title": "Excluir itens selecionados?",
          "description": "Excluir {sessions} sessões e {directories} diretórios? Esta ação não pode ser desfeita.",
          "directoriesDescription": "Excluir {directories} diretórios selecionados? Esta ação não pode ser desfeita.",
          "sessionsDescription": "Excluir {sessions} sessões selecionadas? Esta ação não pode ser desfeita."
        }
      },
      "sessionRow": {
        "status": {
          "idle": "Inativa",
          "needsPermission": "Precisa de permissão",
          "needsReply": "Aguardando resposta",
          "needsRecovery": "Precisa de recuperação",
          "retrying": "Tentando novamente",
          "running": "Em execução",
          "coolingDown": "Em espera"
        },
        "threadToggle": {
          "expand": "Expandir conversa",
          "collapse": "Recolher conversa"
        },
        "loading": {
          "session": "Carregando sessão",
          "directory": "Carregando diretório"
        },
        "attention": {
          "questionAsked": "Pergunta feita"
        },
        "placeholders": {
          "sessionTitle": "Título da sessão"
        },
        "rename": {
          "cancel": "Cancelar renomeação",
          "save": "Salvar renomeação",
          "saving": "Salvando renomeação"
        }
      }
    },
    "attention": {
      "title": {
        "permissionRequired": "Permissão necessária",
        "question": "Pergunta"
      },
      "subtitle": {
        "answerAllToEnableSend": "Responda a todas as perguntas para habilitar o envio.",
        "blocksUntilRespond": "Esta execução aguarda sua resposta."
      },
      "ui": {
        requestNoLongerPending: 'Esta solicitação não está mais pendente.',
        checkingRequestStatus: 'Verificando o status da solicitação…',
        customAnswerFor: "Resposta personalizada: {question}",
        dismissRequest: "Adiar solicitação",
        sendFeedback: "Enviar comentário",
        permissionKeyboardHint: "H J K L / setas: mover · Enter: agir · Esc: título",
        reviewKeyboardHint: "H J K L / setas: mover · Enter: agir · Esc: título",
        questionKeyboardHint: "↑/↓ J/K: opções · ←/→ H/L: perguntas · Espaço: selecionar · Enter: próxima / enviar · Esc: título",
        "permissionStatus": {
          "allowedOnce": "Permitida uma vez",
          "allowedPersistently": "Permitida permanentemente",
          "approvedAutomatically": "Aprovada automaticamente",
          "deniedOnce": "Negada uma vez",
          "deniedPersistently": "Negada permanentemente",
          "awaitingApproval": "Aguardando sua aprovação",
          "replied": "Respondida ({kind})"
        },
        "rejectPermission": "Negar permissão",
        "allowOnce": "Permitir uma vez",
        "alwaysAllow": "Permitir sempre",
        "rejectQuestion": "Recusar pergunta",
        "sendAnswers": "Enviar respostas",
        "permissionLabel": "Permissão",
        "replyReasonLabel": "Motivo da resposta",
        "patternsLabel": "Padrões",
        "rememberedForLabel": "Será lembrada para",
        "previousQuestion": "Pergunta anterior",
        "nextQuestion": "Próxima pergunta",
        "reviewAnswers": "Revisar respostas",
        "unanswered": "Sem resposta",
        "editAnswer": "Editar",
        "questionPager": "Pergunta {current} / {total}",
        "numberedQuestion": "Pergunta {count}",
        "multiple": "múltiplas",
        "single": "única",
        "custom": "Personalizada",
        "typeYourOwnAnswer": "Digite sua própria resposta",
        "answerLabel": "Resposta",
        "answered": "Respondida",
        "awaitingUserInput": "Aguardando sua resposta",
        "customOnlySentHint": "Só será enviada se a opção personalizada for selecionada.",
        "noQuestionsAvailable": "Nenhuma pergunta disponível."
      },
      "toasts": {
        "pleaseAnswerAllQuestions": "Responda a todas as perguntas",
        "answerSent": "Resposta enviada",
        "questionRejected": "Pergunta recusada",
        "permissionRejected": "Permissão negada",
        "permissionGranted": "Permissão concedida"
      }
    },
    "errors": {
      "sideConversationMissing": "O servidor não retornou uma conversa paralela",
      "attachmentDataNotBase64": "Os dados do anexo não são uma data URL base64 válida",
      "attachmentPathEmpty": "O caminho do anexo está vazio",
      "workspaceRootNotAttachable": "A raiz do espaço de trabalho não pode ser anexada como arquivo",
      "attachmentOutsideWorkspace": "O anexo precisa estar dentro do espaço de trabalho ativo",
      "pluginNavigationMustBeRelative": "A navegação do plugin precisa usar um caminho relativo ao aplicativo.",
      "pluginUrlMustBeHttp": "As URLs de plugins precisam usar HTTP ou HTTPS.",
      "toolDetailSectionMismatch": "O servidor retornou uma seção de detalhes da ferramenta incompatível",
      "toolDetailChangedRetry": "Esta ferramenta mudou durante o carregamento dos detalhes. Tente novamente nesta seção.",
      "activityRequestTimedOut": "A solicitação de atividade expirou",
      "sidebarPayloadMissingPreferences": "O payload do estado da barra lateral do chat está sem preferências",
      "workspaceIdRequired": "É necessário um ID de espaço de trabalho válido",
      "sessionTitleRequired": "É necessário informar um título para a sessão",
      "sessionMissing": "O servidor não retornou uma sessão",
      "workspacePathRequired": "É necessário informar o caminho do espaço de trabalho",
      "workspaceMissing": "O servidor não retornou um espaço de trabalho",
      "workspaceRootMissing": "O servidor não informou a raiz do espaço de trabalho",
      "attachmentNameAndDataRequired": "O nome e os dados do anexo são obrigatórios",
      "workspaceIdMissing": "O servidor não retornou um ID de espaço de trabalho",
      "workspaceIdNotProvided": "Falta o ID do espaço de trabalho",
      "sessionAndPartIdRequired": "São necessários o ID da sessão e o ID da parte",
      "transcriptCountMissing": "A resposta da transcrição não contém user_message_count",
      "sessionIdRequired": "É necessário informar o ID da sessão",
      "historyPaginationStalled": "A paginação do histórico da sessão não avançou",
      "userMessageIdRequired": "É necessário informar um ID de mensagem de usuário válido",
      "rewoundSessionMissing": "O servidor não retornou uma sessão restaurada",
      "messageIdRequired": "É necessário informar um ID de mensagem válido",
      "forkedSessionMissing": "O servidor não retornou uma sessão derivada.",
      "replyPaginationStalled": "A paginação das respostas não avançou",
      "statusRefreshFailed": "Não foi possível atualizar o status da sessão",
      "sessionRequiredForAttachments": "É necessária uma sessão para anexar arquivos",
      "sessionWorkspaceInvalid": "A sessão não tem um espaço de trabalho válido",
      "sessionRequired": "É necessária uma sessão",
      "sessionAndUserMessageRequired": "São necessários o ID da sessão e um ID de mensagem de usuário válido",
      "rewindRequiresUserMessage": "Para restaurar, é necessária uma mensagem de usuário concluída nesta sessão",
      "btwUnexpectedEvent": "Evento inesperado no fluxo BTW",
      "btwInvalidAnswer": "Resposta BTW inválida",
      "btwNoStream": "A resposta BTW não contém um fluxo",
      "btwStreamStalled": "O fluxo BTW parou de avançar",
      "btwStreamEndedEarly": "O fluxo BTW foi encerrado antes de ser concluído",
      "btwStreamTooLarge": "O evento do fluxo BTW é grande demais",
      "attachmentReadFailed": "Falha ao ler o anexo",
      "failedDraftMustBeResolved": "É necessário restaurar ou descartar primeiro o rascunho anterior que falhou"
    },
    "attachments": {
      "errors": {
        "textPasteTooLarge": "O texto colado excede o limite de 1 MiB. Nada foi colado nem enviado; a área de transferência e o rascunho atual permanecem inalterados.",
        "failedToReadUnknown": "Não foi possível ler o arquivo",
        "fileTooLarge": "Arquivo grande demais: {name} ({size})",
        "totalTooLarge": "Anexos grandes demais (máximo: {size})",
        "failedToReadFile": "Não foi possível ler o arquivo: {name}",
        "unsupportedFile": "Arquivo não compatível: {name}",
        "tooMany": "Uma mensagem pode anexar no máximo {count} recursos."
      },
      "recoveryNeedsEmptyDraft": "Selecione a sessão original e limpe ou salve seu rascunho atual antes de restaurar.",
      "recoverBeforeSend": "Restaure ou descarte o rascunho com falha salvo antes de enviar outra mensagem.",
      "failedDraftSaved": "O envio anterior falhou. Seu rascunho mais recente permanece inalterado; o texto e os anexos do envio com falha foram salvos nesta página para recuperação manual.",
      "recoverySession": "Sessão original: {session}",
      "restoreFailedDraft": "Restaurar rascunho com falha",
      "discardFailedDraft": "Descartar rascunho com falha salvo",
      "preparing": "Os anexos ainda estão sendo preparados; o rascunho não foi enviado.",
      "preparationFailed": "Não foi possível preparar um anexo. Revise o rascunho e tente enviar novamente.",
      "sendContents": "Enviar conteúdo",
      "pastedText": "Texto colado",
      "whitespaceOnly": "Somente espaços em branco",
      "deliveryNotice": "Ao enviar, o conteúdo será encaminhado a {provider}. Apenas visualizar não envia o conteúdo.",
      "referenceNotice": "Apenas uma referência. O conteúdo dos arquivos não é incluído automaticamente.",
      "unavailable": "Conteúdo indisponível: {reason}. Anexe o arquivo novamente para usá-lo aqui.",
      "unavailableUnknown": "ele foi anexado em outro provedor ou modelo",
      "title": "Anexos",
      "rowTitle": "Anexo",
      "attaching": "Anexando…",
      "countLabelOne": "1 arquivo",
      "countLabelMany": "{count} arquivos",
      "repo": "repositório",
      "removeAttachmentAria": "Remover anexo",
      "actions": {
        "addFromComputer": "Adicionar do computador",
        "addFromProject": "Adicionar do projeto",
        "clearAll": "Remover todos",
        "clear": "Remover"
      },
      "empty": {
        "title": "Nenhum anexo",
        "description": "Use os botões acima para anexar arquivos."
      }
    },
    "toasts": {
      "noSessionErrorToCopy": "Não há erro de sessão para copiar",
      "copiedErrorDetails": "Detalhes do erro copiados",
      "failedToCopyErrorDetails": "Falha ao copiar os detalhes do erro",
      "titleCannotBeEmpty": "O título não pode ficar vazio",
      "sessionRenamed": "Sessão renomeada",
      "sessionForked": "Sessão derivada",
      "noTranscriptAvailable": "Nenhuma transcrição disponível",
      "transcriptCopied": "Transcrição copiada",
      "transcriptExportedAs": "Transcrição exportada como {filename}",
      "sharingDisabledInConfig": "O compartilhamento está desativado na configuração",
      "shareLinkCopied": "Link de compartilhamento copiado",
      "sessionShared": "Sessão compartilhada",
      "sessionUnshared": "Compartilhamento da sessão desativado",
      "selectModelToCompact": "Selecione um modelo para compactar esta sessão",
      "compactionStarted": "Compactação iniciada",
      "abortedRun": "Execução interrompida",
      "failedToAbortRun": "Falha ao interromper a execução",
      "pressEscAgainToAbort": "Pressione Esc novamente para interromper ({modKey}+.) nos atalhos",
      "answerAllQuestions": "Responda a todas as perguntas",
      "answerSent": "Resposta enviada",
      "questionRejected": "Pergunta rejeitada",
      "sessionRequiredForCommand": "É necessária uma sessão para /{command}.",
      "sessionContinuationStarted": "Continuação da sessão iniciada.",
      "pagerUnsupported": "A transcrição na web já permite navegar pelas páginas usando a rolagem.",
      "noAssistantMessageLoaded": "Nenhuma mensagem do assistente foi carregada.",
      "noVisibleTranscript": "Não há conteúdo de transcrição visível.",
      "noChildSessions": "Esta sessão não tem sessões filhas.",
      "noParentSession": "Esta sessão não tem sessão pai.",
      "selectModelBeforeSending": "Selecione um modelo antes de enviar",
      "modelHints": {
        "runtimeDefault": "Usando o padrão do runtime: {model}",
        "sessionModel": "Usando o modelo da sessão: {model}",
        "thinkingMode": "Usando o modo de raciocínio: {mode}",
        "speedMode": "Usando o modo de velocidade: {mode}"
      }
    },
    "composer": {
      "actions": {
        "compact": {
          "label": "Compactar sessão",
          "description": "Resumir com o modelo selecionado"
        },
        "init": {
          "label": "Inicializar AGENTS.md",
          "description": "Inserir o comando /init no campo de mensagem"
        },
        "review": {
          "label": "Revisar alterações",
          "description": "Inserir o comando /review no campo de mensagem"
        }
      },
      "picker": {
        "modelTitle": "Modelo",
        "thinkingTitle": "Raciocínio",
        "speedTitle": "Velocidade",
        "optionsTitle": "Opções",
        "searchModels": "Buscar modelos",
        "searchThinkingModes": "Buscar modos de raciocínio",
        "searchSpeedModes": "Buscar modos de velocidade",
        "searchOptions": "Buscar opções",
        "emptyModels": "Nenhum modelo encontrado.",
        "emptyThinkingModes": "Não há modos de raciocínio disponíveis para este modelo.",
        "emptySpeedModes": "Não há modos de velocidade disponíveis para este modelo.",
        "emptyOptions": "Nenhuma opção encontrada.",
        "availableCount": "{count} disponíveis"
      },
      "model": {
        "autoDefault": "Padrão do Agena runtime",
        "autoDefaultDescription": "Usar o modelo padrão configurado no servidor Agena",
        "defaultThinkingDescription": "Usar o modo de raciocínio configurado para este modelo",
        "defaultSpeedDescription": "Usar o modo de velocidade configurado para este modelo"
      },
      "editor": {
        "open": "Abrir editor",
        "collapse": "Recolher editor"
      },
      "input": {
        "placeholder": "Digite uma mensagem…"
      },
      "promptHistory": {
        "label": "histórico>",
        "search": "Buscar histórico de prompts",
        "empty": "O histórico de prompts está vazio",
        "noMatches": "Nenhum prompt corresponde ao histórico",
        "hint": "↓ abre o histórico | ↑/↓ navega | Enter usa | Esc fecha",
        "itemsHint": "Remova os anexos antes de recuperar o histórico de prompts"
      },
      "status": {
        "loading": "carregando",
        "slash": "barra"
      }
    },
    "sessionWork": {
      "backToFiles": "Voltar aos arquivos",
      "moreDiff": "Prévia do diff · carregar mais",
      "tasks": "Tarefas em segundo plano",
      "activeTasks": "{count} ativas",
      "changes": "Alterações da sessão",
      "files": "{count} arquivos",
      "recordingIncomplete": "O registro pode estar incompleto (por exemplo, gravações via shell são declarações, não edições de arquivo confirmadas).",
      "operationHistory": "Histórico de operações, não um diff consolidado da sessão (não há uma linha de base completa).",
      "operationDiff": "Diff da operação de patch inteira; pode incluir outros arquivos.",
      "recordedTruncated": "Diff truncado; somente o conteúdo persistido pode ser carregado.",
      "clean": "Nenhuma alteração registrada nesta sessão",
      "notRepository": "Este diretório não é um repositório Git",
      "loadFailed": "Não foi possível carregar as alterações",
      "pageRange": "{start}–{end} / {total}",
      "indexWorking": "Índice / árvore de trabalho",
      "working": "Árvore de trabalho",
      "staged": "Preparadas",
      "noDiff": "Nenhum diff textual nesta visualização",
      "previous": "Anterior",
      "next": "Próxima",
      "more": "Mostrar mais",
      "logs": "Logs",
      "close": "Fechar",
      "refresh": "Atualizar",
      "stop": "Parar",
      "pause": "Pausar",
      "resume": "Retomar",
      "delete": "Excluir",
      "dismiss": "Dispensar",
      "loading": "Carregando…",
      "noOutput": "Ainda não há saída",
      "logTail": "Últimas 200 linhas / 128 KiB · atualiza enquanto estiver aberto",
      "dropped": "{count} linhas não são mais mantidas pelo servidor",
      "nextWake": "Próxima ativação:",
      "permission": "{count} autorizações",
      "review": "{count} revisões de plano",
      "question": "{count} perguntas",
      "resolve": "Revisar solicitação",
      "waitingTools": "Aguardando ferramentas",
      "waitingInput": "Aguardando entrada",
      "retry": "Tentativa {attempt}",
      "status": {
        "pending": "Na fila",
        "running": "Em execução",
        "waiting": "Aguardando",
        "paused": "Pausada",
        "succeeded": "Concluída",
        "failed": "Falhou",
        "cancelled": "Cancelada",
        "stopped": "Interrompida"
      }
    },
    errorPart: {
      waiting: "Aguardando nova tentativa",
      retrying: "Tentando novamente",
      recovered: "Recuperado",
      failed: "Nova tentativa falhou",
      cancelled: "Nova tentativa cancelada",
      retryCount: "Tentativa {attempt}/{max}",
      retryNumber: "Tentativa {attempt}",
      inDuration: "Nova tentativa em {duration}",
      scheduledAt: "às {time}",
      awaitingStart: "Aguardando o início da nova tentativa",
      requesting: "Aguardando resposta",
      streaming: "Recebendo resposta",
      elapsed: "{duration} decorridos",
      recoveredAfter: "Recuperado após {attempt} tentativas",
      exhausted: "Todas as {max} tentativas foram usadas",
      stopped: "Nova tentativa automática interrompida",
      seconds: "{count} s",
      minutes: "{minutes} min {seconds} s",
      hours: "{hours} h {minutes} min",
    },
    "partTitles": {
      "answer": "Resposta",
      "reasoning": "raciocínio",
      "textSegment": "Texto",
      "operation": "Operação",
      "command": "Comando",
      "error": "Erro",
      "notice": "Aviso",
      "compaction": "Compactação",
      "responseRunning": "Resposta em andamento",
      "responseCompleted": "Resposta concluída",
      "responseCancelled": "Resposta cancelada",
      "responseFailed": "Falha na resposta",
      "runFailed": "A execução falhou.",
      "compactionUnchanged": "O contexto já está compactado; o histórico existente foi mantido.",
      "compactionTokens": "De {before} para {after} tokens"
    },
    "partStatus": {
      "running": "Em andamento",
      "completed": "Concluído",
      "policyDenied": "Negado pela política",
      "declined": "Recusado",
      "unavailable": "Indisponível",
      "failed": "Falhou",
      "cancelled": "Cancelado",
      "pending": "Pendente"
    },
    "activityLog": {
      "exitCode": "código de saída {code}",
      "errorBand": "Erro"
    },
    "btw": {
      "title": "BTW · pergunta somente leitura",
      "description": "Faça uma pergunta à parte enquanto a conversa principal continua. Ferramentas somente de leitura; sem alterações na conversa principal.",
      "placeholder": "O que você gostaria de perguntar?",
      "send": "Perguntar",
      "stop": "Parar",
      "loading": "Respondendo…",
      "inlineHint": "Perguntas temporárias somente de leitura para esta conversa",
      "clear": "Limpar perguntas BTW e interromper a solicitação",
      "stopped": "Interrompida. A resposta parcial foi preservada.",
      "returnToParent": "Paralela · voltar à principal",
      "sideDescription": "Abrir uma ramificação separada com o contexto atual",
      "sectionTitle": "BTW",
      "sideChip": "Paralela"
    },
    "toolDetails": {
      "copyDiff": "Copiar diff",
      "diffChangeA": "Adicionado",
      "diffChangeM": "Modificado",
      "diffChangeD": "Excluído",
      "diffChangeR": "Renomeado",
      "presentation": "Dados de apresentação",
      "diff": "Alterações",
      "expandDiff": "Mostrar mais {count} linhas",
      "collapseDiff": "Mostrar menos linhas",
      "label": "Detalhes técnicos",
      "input": "Entrada",
      "output": "Saída",
      "metadata": "Metadados"
    },
    planReview: {
      "creationTitle": "Aprovar novo plano",
      "statusTitle": "Revisar mudança de estado do plano",
      "decision": "Decisão",
      "approve": "Aprovar",
      "approveAutorunOn": "Aprovar e ativar a execução automática",
      "approveAutorunOff": "Aprovar e desativar a execução automática",
      "approveRequested": "Aprovar o estado solicitado",
      "approveRequestedPause": "Aprovar o estado e desativar a continuação automática",
      "keepPlanning": "Continuar planejando",
      "reject": "Rejeitar",
      "cancelPlan": "Cancelar plano",
      "approveAutorunOnDescription": "Aprovar o plano, ativá-lo e manter a execução automática ativada.",
      "approveAutorunOffDescription": "Aprovar o plano, ativá-lo e manter a execução automática desativada.",
      "approveBlockedDescription": "Aprovar o plano e marcá-lo como bloqueado.",
      "approveCompletedDescription": "Aprovar o plano e marcá-lo como concluído.",
      "approvePlanningDescription": "Aprovar o plano e voltar ao planejamento.",
      "approveCancelledDescription": "Aprovar e cancelar o plano.",
      "keepPlanningDescription": "Voltar ao planejamento para continuar editando o plano.",
      "rejectDescription": "Rejeitar o plano atual e marcar a revisão como rejeitada.",
      "cancelPlanDescription": "Cancelar todo o plano e interromper o trabalho nele.",
      "question": "Escolha se o plano deve passar para {phase} ou escreva comentários para o assistente revisá-lo.",
      "phases": {
        "planning": "em planejamento",
        "active": "ativo",
        "blocked": "bloqueado",
        "completed": "concluído",
        "cancelled": "cancelado"
      }
    },
    "planViewer": {
      "title": "Plano",
      "session": "Sessão {id}",
      "requiresSession": "Abra uma sessão antes de visualizar o plano.",
      "autorun": "Execução automática",
      "noAutorun": "Nenhuma configuração de execução automática ativa",
      "refresh": "Atualizar plano",
      "loading": "Carregando plano",
      "empty": "Não há um plano disponível para esta sessão."
    },
    "headerOverlay": {
      "title": {
        "permissionRequired": "Permissão necessária",
        "question": "Pergunta",
        "retrying": "Tentando novamente",
        "actionRequired": "Ação necessária"
      },
      "subtitle": {
        "respondToContinue": "Responda para continuar esta sessão."
      },
      "retryingIn": "Nova tentativa em {countdown}",
      "retryMeta": "tentativa {attempt} • próxima {next}"
    },
    "selectedModelFallback": "Modelo selecionado",
    "commandPalette": {
      "title": "Comandos",
      "search": "Buscar comandos",
      "empty": "Nenhum comando encontrado",
      "aliases": "Aliases",
      "hint": "Cima/Baixo para navegar | Enter para selecionar | Esc para fechar",
      "rewindPromptTitle": "Voltar para o ID da mensagem",
      "rewindPromptPlaceholder": "ID da mensagem"
    },
    "sessionDiff": {
      "toggleAria": "Alternar alterações nos arquivos da sessão",
      "panelTitle": "Alterações nos arquivos da sessão",
      "close": "Fechar",
      "backToList": "Voltar aos arquivos",
      "listTitle": "Arquivos alterados",
      "loading": "Carregando alterações nos arquivos...",
      "empty": "Ainda não há alterações em arquivos nesta sessão.",
      "wrap": {
        "label": "Quebra de linha",
        "enable": "Desativar quebra de linha",
        "disable": "Ativar quebra de linha"
      }
    },
    "lspStatus": {
      "toggleAria": "Alternar status do runtime do LSP",
      "panelTitle": "Status do runtime do LSP",
      "close": "Fechar",
      "listTitle": "Sessão atual",
      "noRootGroup": "Nenhuma raiz de workspace",
      "loading": "Carregando status do runtime do LSP...",
      "empty": "Não há sessões ativas do runtime do LSP."
    },
    "mcpStatus": {
      "toggleAria": "Alternar status do servidor MCP",
      "panelTitle": "Status do servidor MCP",
      "close": "Fechar",
      "listTitle": "Servidores",
      "loading": "Carregando status do servidor MCP...",
      "empty": "Nenhum servidor MCP configurado."
    },
    "attachProjectDialog": {
      "title": "Anexar arquivo do projeto",
      "description": "Escolha um arquivo do projeto atual para anexar",
      "pathPlaceholder": "Selecione um arquivo",
      "attachedCount": "Anexados: {count} arquivo(s)"
    },
    "renameSessionDialog": {
      "title": "Renomear sessão",
      "description": "Atualizar o título da sessão"
    },
    "roles": {
      "activity": "atividade",
      "user": "usuário",
      "assistant": "assistente",
      "system": "sistema"
    },
    "messageItem": {
      "interrupted": "interrompida",
      "errorDetails": "detalhes do erro",
      "fileFallback": "arquivo",
      "attachmentFallback": "anexo",
      "fork": {
        "confirmTitle": "Criar uma ramificação da sessão?",
        "confirmDescription": "Criar uma nova sessão a partir desta mensagem.",
        "confirmAction": "Criar ramificação",
        "actionTitle": "Criar ramificação a partir daqui"
      },
      "revert": {
        "confirmTitle": "Reverter a sessão?",
        "confirmDescription": "Reverter a sessão para este ponto. Isso descartará as mensagens posteriores.",
        "confirmAction": "Reverter",
        "actionTitle": "Reverter a partir daqui"
      },
      "copy": {
        "actionTitle": "Copiar mensagem"
      }
    }
  },
  "nav": {
    "chat": "Conversa",
    "files": "Arquivos",
    "preview": "Pré-visualização",
    "terminal": "Terminal",
    "git": "Git",
    "settings": "Configurações",
    "back": "Voltar",
    "diff": "Diferenças"
  },
  "aria": {
    "mainNavigation": "Navegação principal",
    "primaryNavigation": "Navegação primária"
  },
  "desktopSidebar": {
    "collapse": "Recolher barra lateral",
    "expand": "Expandir barra lateral"
  },
  "header": {
    "toggleSidebar": "Alternar barra lateral",
    "openSessions": "Abrir sessões",
    "openFilesPanel": "Abrir painel de arquivos",
    "openPreviewPanel": "Abrir painel de pré-visualização",
    "openTerminalPanel": "Abrir painel do Terminal",
    "openSourceControlPanel": "Abrir painel de controle de código-fonte",
    "openSettingsPanel": "Abrir painel de configurações",
    "showWorkspacePanel": "Mostrar painel do espaço de trabalho",
    "hideWorkspacePanel": "Ocultar painel do espaço de trabalho",
    "noDirectorySelected": "Nenhum diretório selecionado",
    "changeDirectory": "Alterar diretório",
    "changesAvailable": "Há alterações disponíveis",
    "connectionStatus": "Status da conexão",
    "status": {
      "online": "Conectado",
      "offline": "Desconectado"
    },
    "help": "Ajuda",
    "locateSession": "Localizar sessão",
    "locateCurrentSession": "Localizar a sessão atual na barra lateral",
    "windowTabs": {
      "newWindow": "Nova janela",
      "createWindowTitle": "Abrir em uma nova janela",
      "openInWindowDescription": "Criar e abrir esta visualização em uma guia de janela",
      "windowActions": "Ações da janela",
      "closeActions": "Ações de fechamento",
      "duplicateWindow": "Duplicar janela",
      "previousWindow": "Janela anterior",
      "nextWindow": "Próxima janela",
      "splitRight": "Dividir à direita",
      "closeSplit": "Fechar painel dividido",
      "dragToSplit": "Arraste para cá para dividir",
      "resizingSplit": "Redimensionando painéis…",
      "resizingSidebar": "Redimensionando barra lateral…",
      "focusInMain": "Focar no painel principal",
      "splitNoContent": "Nenhum conteúdo selecionado para a divisão.",
      "group": "Grupo",
      "newGroup": "Novo grupo",
      "toggleGroup": "Recolher ou expandir grupo",
      "groupMenuTitle": "Ações do grupo",
      "expandGroup": "Expandir grupo",
      "collapseGroup": "Recolher grupo",
      "equalizeSplit": "Igualar a largura dos painéis",
      "ungroupDrop": "Solte aqui para desagrupar",
      "splitDropAlreadyInGroup": "Esta sessão ou arquivo já está no grupo de destino.",
      "splitDropFailed": "Não foi possível dividir no grupo de destino.",
      "splitDropSameTab": "Arraste outra guia para dividir.",
      "splitDropNeedsTab": "Abra primeiro uma guia sem grupo e depois arraste outra guia para dividir.",
      "dropIntoTabs": "Soltar no grupo de guias",
      "dropInsertBefore": "Inserir à esquerda",
      "dropInsertAfter": "Inserir à direita",
      "dropReplace": "Substituir painel",
      "closeGroup": "Fechar grupo",
      "closeCurrent": "Fechar janela atual",
      "closeOthers": "Fechar outras janelas",
      "closeLeft": "Fechar janelas à esquerda",
      "closeRight": "Fechar janelas à direita",
      "closeAll": "Fechar todas as janelas"
    },
    "selectDirectory": {
      "title": "Selecionar diretório",
      "description": "Escolha um diretório para defini-lo como diretório atual",
      "placeholder": "/path/to/directory"
    }
  },
  "git": {
    sidebar: {
      changes: 'Alterações',
      stagedHint: '{count} preparados',
      noStaged: 'Prepare arquivos para fazer commit',
      stageAll: 'Preparar tudo',
      clean: 'Árvore de trabalho limpa',
      cleanDescription: 'Suas alterações foram confirmadas.',
      repository: 'Repositório',
      moreActions: 'Mais ações Git',
      branches: 'Branches',
      worktrees: 'Árvores de trabalho',
      compare: 'Comparar',
      stashes: 'Stashes',
      backToActions: 'Todas as ações',
    },
    navigation: {
      chooseFolder: 'Escolher pasta',
      scanRepositories: 'Buscar repositórios aninhados',
      remoteBranch: 'Branch remota',
      branchHistory: 'Ver histórico da branch',
    },
    "noCommitTemplateConfigured": "Nenhum modelo de commit configurado",
    "actions": {
      "fetch": "Buscar",
      "pull": "Fazer pull",
      "push": "Fazer push",
      "sync": "Sincronizar",
      "merge": "Mesclar",
      "rebase": "Fazer rebase",
      "stash": "Guardar alterações"
    },
    "ui": {
      "conflictEditor": {
        "title": "Resolver conflitos",
        "actions": {
          "takeOurs": "Aceitar nossa versão",
          "oursAndNext": "Nossa versão e próximo",
          "takeTheirs": "Aceitar versão deles",
          "theirsAndNext": "Versão deles e próximo",
          "takeBase": "Aceitar versão base",
          "takeBoth": "Aceitar ambas as versões",
          "applyAndNext": "Aplicar e avançar",
          "openDiff": "Abrir diff"
        },
        "blockCount": {
          "one": "{count} bloco",
          "many": "{count} blocos"
        },
        "unmergedWithoutMarkers": "Não mesclado, sem marcadores de texto",
        "empty": {
          "noMarkersTitle": "Nenhum marcador de conflito de texto encontrado",
          "unmergedNote": "Este conflito não tem marcadores (geralmente por ser binário). Ainda é possível resolvê-lo com \"Aceitar nossa versão/Aceitar versão deles\" ou abrir o diff padrão.",
          "resolvedNote": "Este arquivo talvez já esteja resolvido. Abra o diff padrão para verificar o estado final."
        },
        "blockTitle": "Bloco nº {index}",
        "labels": {
          "ours": "Nossa versão",
          "theirs": "Versão deles",
          "base": "Base",
          "both": "Ambas"
        }
      },
      "sequencer": {
        "mergeInProgressTitle": "Mesclagem em andamento",
        "mergeInProgressDescription": "Resolva os conflitos e faça um commit. Ou aborte a mesclagem.",
        "rebaseInProgressTitle": "Rebase em andamento",
        "rebaseInProgressDescription": "Continue ou pule no terminal, ou aborte o rebase.",
        "cherryPickInProgressTitle": "Cherry-pick em andamento",
        "cherryPickInProgressDescription": "Resolva os conflitos e depois continue ou pule, ou aborte a operação.",
        "revertInProgressTitle": "Reversão em andamento",
        "revertInProgressDescription": "Resolva os conflitos e depois continue ou pule, ou aborte a operação.",
        "openConflict": "Abrir conflito",
        "skip": "Pular",
        "mergeActions": "Ações de mesclagem",
        "rebaseActions": "Ações de rebase",
        "cherryPickActions": "Ações de cherry-pick",
        "revertActions": "Ações de reversão",
        "moreMergeActions": "Mais ações de mesclagem",
        "moreRebaseActions": "Mais ações de rebase",
        "moreCherryPickActions": "Mais ações de cherry-pick",
        "moreRevertActions": "Mais ações de reversão"
      },
      "diffViewer": {
        "labels": {
          "index": "Índice",
          "workingTree": "Árvore de trabalho"
        },
        "noOriginal": "(sem versão original)",
        "noModified": "(sem versão modificada)",
        "loading": "Carregando diff...",
        "confirmDiscardHunk": "Descartar este bloco de alterações? Esta ação não pode ser desfeita.",
        "actions": {
          "openFile": "Abrir arquivo",
          "revealInFiles": "Mostrar em Arquivos"
        },
        "wrap": {
          "label": "Quebra de linha",
          "enable": "Ativar quebra de linha",
          "disable": "Desativar quebra de linha"
        }
      },
      "stashPanel": {
        "actions": {
          "dropAll": "Excluir tudo",
          "stashEllipsis": "Guardar alterações...",
          "pop": "Aplicar e remover",
          "drop": "Excluir"
        },
        "confirmDropAll": {
          "title": "Excluir todos os stashes?",
          "description": "Todas as entradas do stash serão removidas permanentemente."
        },
        "empty": "Nenhum stash"
      },
      "dialogs": {
        "repoPicker": {
          "title": "Repositórios",
          "description": "Selecione um repositório Git dentro do projeto ativo",
          "actions": {
            "initializeRepo": "Inicializar repositório",
            "cloneRepo": "Clonar repositório",
            "closeRepo": "Fechar repositório",
            "reopenRepo": "Reabrir"
          },
          "empty": {
            "title": "Nenhum repositório encontrado",
            "description": "Crie um com \"Inicializar repositório\"."
          },
          "sections": {
            "closedRepos": "Repositórios fechados",
            "parentRepos": "Repositórios em pastas superiores",
            "discoveryLimited": "A busca atingiu o limite de segurança; alguns repositórios aninhados podem não aparecer."
          }
        },
        "remotes": {
          "title": "Remotos",
          "description": "Gerencie os remotos do repositório",
          "sections": {
            "addRemote": "Adicionar remoto",
            "manageRemote": "Gerenciar remoto"
          },
          "selectRemote": "Selecione um remoto",
          "placeholders": {
            "newRemoteName": "Nome do novo remoto",
            "newRemoteUrl": "URL do novo remoto"
          },
          "actions": {
            "setUrl": "Definir URL",
            "copyUrl": "Copiar URL"
          },
          "confirmRemove": {
            "title": "Remover remoto?",
            "description": "A configuração do remoto será removida."
          },
          "empty": "Nenhum remoto"
        },
        "lfs": {
          "title": "Git LFS",
          "description": "Armazenamento de arquivos grandes",
          "sections": {
            "status": "Status",
            "trackPatterns": "Padrões rastreados",
            "locks": "Bloqueios"
          },
          "installedLabel": "Instalado:",
          "actions": {
            "installForRepo": "Instalar neste repositório",
            "track": "Rastrear",
            "lock": "Bloquear",
            "unlock": "Desbloquear",
            "forceUnlock": "Forçar desbloqueio"
          },
          "placeholders": {
            "trackPattern": "*.psd",
            "lockPath": "caminho/para/arquivo.bin"
          },
          "emptyTrackedPatterns": "Nenhum padrão rastreado",
          "emptyLocks": "Nenhum bloqueio"
        },
        "branches": {
          "title": "Branches",
          "description": "Gerencie as branches do repositório",
          "sections": {
            "branches": "Branches"
          },
          "placeholders": {
            "newBranchName": "Nome da nova branch..."
          },
          "actions": {
            "checkout": "Checkout",
            "deleteBranch": "Excluir",
            "deleteRemote": "Excluir do remoto"
          },
          "confirmDeleteLocal": {
            "title": "Excluir branch?",
            "description": "Esta ação não pode ser desfeita."
          },
          "confirmDeleteRemote": {
            "title": "Excluir branch remota?",
            "description": "Isso excluirá a branch do remoto."
          }
        },
        "history": {
          "title": "Histórico",
          "description": "Commits recentes",
          "sections": {
            "commits": "Commits",
            "files": "Arquivos"
          },
          "compareBase": "Base de comparação:",
          "noMessage": "(sem mensagem)",
          "emptyCommits": "Nenhum commit",
          "selectCommitToViewDiff": "Selecione um commit para ver o diff.",
          "selectFileToViewDiff": "Selecione um arquivo alterado para ver o diff.",
          "fields": {
            "refType": "Tipo de referência",
            "ref": "Referência",
            "resetMode": "Modo de reset"
          },
          "search": {
            "refTypes": "Buscar tipos",
            "refs": "Buscar referências",
            "resetModes": "Buscar modos"
          },
          "placeholders": {
            "tagName": "Nome da tag",
            "branchName": "Nome da branch",
            "authorContains": "Autor contém",
            "messageContains": "Mensagem contém",
            "commitMetaContains": "Mensagem, autor ou e-mail"
          },
          "allTags": "Todas as tags",
          "allBranches": "Todas as branches",
          "refType": {
            "branch": "Branch",
            "tag": "Tag"
          },
          "resetModes": {
            "soft": "Reset suave",
            "mixed": "Reset misto",
            "hard": "Reset rígido"
          },
          "actions": {
            "copyHash": "Copiar hash",
            "checkout": "Checkout",
            "createBranch": "Criar branch",
            "selectForCompare": "Selecionar para comparação",
            "compareSelected": "Selecionado ({hash})",
            "compareWithParent": "Comparar com o commit pai",
            "compareWithSelected": "Comparar com o selecionado",
            "reset": "Reset",
            "cherryPick": "Cherry-pick",
            "revert": "Reverter",
            "allFiles": "Todos os arquivos"
          },
          "confirmReset": {
            "title": "Fazer reset para este commit?",
            "description": "Isso moverá o HEAD e atualizará a árvore de trabalho conforme o modo selecionado."
          },
          "confirmCherryPick": {
            "title": "Aplicar cherry-pick deste commit?",
            "description": "Isso aplicará o commit sobre a branch atual."
          },
          "confirmRevert": {
            "title": "Reverter este commit?",
            "description": "Isso criará um novo commit que reverte as alterações."
          },
          "loadingFiles": "Carregando arquivos...",
          "emptyFiles": "Nenhuma alteração em arquivos.",
          "diffTitleFile": "Diff - {file}",
          "diffTitleAllFiles": "Diff - todos os arquivos",
          "loadingDiff": "Carregando diff...",
          "emptyDiff": "Nenhum conteúdo de diff.",
          "wrap": {
            "label": "Quebra de linha",
            "enable": "Ativar quebra de linha",
            "disable": "Desativar quebra de linha"
          },
          "confirmHardReset": {
            "title": "Confirmar hard reset",
            "description": "Isso reescreverá a árvore de trabalho e descartará as alterações locais.",
            "confirmText": "Confirmar hard reset",
            "typeResetHintPrefix": "Digite",
            "typeResetHintSuffix": "para confirmar o hard reset."
          }
        },
        "submodules": {
          "title": "Submódulos",
          "description": "Gerencie os submódulos do Git",
          "sections": {
            "addSubmodule": "Adicionar submódulo"
          },
          "placeholders": {
            "url": "https://github.com/org/repo.git",
            "path": "caminho/do/submódulo",
            "branchOptional": "branch (opcional)"
          },
          "empty": "Nenhum submódulo",
          "branchLine": "Branch: {branch}",
          "actions": {
            "init": "Inicializar",
            "update": "Atualizar",
            "updateInitRecursive": "Atualizar (inicializar e recursivamente)"
          }
        },
        "worktrees": {
          "title": "Árvores de trabalho",
          "description": "Gerenciar worktrees do Git",
          "sections": {
            "addWorktree": "Adicionar worktree"
          },
          "actions": {
            "prune": "Limpar referências obsoletas",
            "addWorktree": "Adicionar worktree",
            "migrate": "Migrar",
            "migrateChanges": "Migrar alterações"
          },
          "hints": {
            "pathRelativeToRepoRoot": "Use um caminho absoluto ou relativo à raiz do repositório (por exemplo, ../feature)."
          },
          "placeholders": {
            "branchName": "Nome da branch",
            "startPoint": "Ponto de partida (HEAD)"
          },
          "createNewBranch": "Criar nova branch",
          "detached": "(HEAD desacoplado)",
          "status": {
            "locked": "Bloqueada",
            "prunable": "Pode ser removida"
          },
          "confirmMigrate": {
            "title": "Migrar alterações para cá?",
            "description": "Mover as alterações locais desta worktree para a worktree atual."
          },
          "confirmRemove": {
            "title": "Remover worktree?",
            "description": "Isso remove o diretório da worktree."
          },
          "empty": "Nenhuma worktree"
        },
        "clone": {
          "title": "Clonar repositório",
          "description": "Clonar um repositório remoto para o projeto",
          "fields": {
            "repositoryUrl": "URL do repositório",
            "targetPath": "Caminho de destino (relativo ao projeto)",
            "checkoutRefOptional": "Referência/branch para checkout (opcional)"
          },
          "placeholders": {
            "repositoryUrl": "https://github.com/org/repo.git",
            "targetPath": "(deixe em branco para derivar da URL)"
          },
          "hints": {
            "targetPathExample": "Exemplo: `repos/my-app` - por padrão, usa o nome do repositório se estiver vazio."
          },
          "cloneSubmodulesRecursively": "Clonar submódulos recursivamente",
          "actions": {
            "clone": "Clonar"
          }
        },
        "initRepo": {
          "title": "Inicializar repositório Git",
          "description": "Criar um novo diretório .git",
          "fields": {
            "path": "Caminho (relativo ao projeto)",
            "defaultBranchOptional": "Branch padrão (opcional)"
          },
          "placeholders": {
            "path": ". (raiz do projeto)"
          },
          "hints": {
            "pathExamples": "Exemplos: `.` ou `packages/api`",
            "initialBranchFlag": "Usa `git init --initial-branch` quando informado."
          },
          "actions": {
            "initialize": "Inicializar"
          }
        },
        "compare": {
          "title": "Comparar",
          "description": "Comparar duas referências ou branches",
          "placeholders": {
            "base": "Base (por exemplo, main)",
            "head": "Head (por exemplo, feature/foo)",
            "pathOptional": "Filtro de caminho opcional (por exemplo, src/app.ts)"
          },
          "actions": {
            "swapRefs": "Inverter referências",
            "compare": "Comparar"
          },
          "empty": "Nenhuma diferença para exibir.",
          "wrap": {
            "label": "Quebra de linha",
            "enable": "Ativar quebra de linha",
            "disable": "Desativar quebra de linha"
          }
        },
        "selectChangedFileTitle": "Selecione um arquivo alterado",
        "selectChangedFileDescription": "Escolha um arquivo no controle do código-fonte para inspecionar o diff.",
        "openSourceControlPanel": "Abrir o painel de controle do código-fonte",
        "mergeBranchTitle": "Mesclar branch",
        "mergeBranchDescription": "Mesclar uma branch à branch atual",
        "rebaseBranchTitle": "Rebase da branch",
        "rebaseBranchDescription": "Fazer rebase da branch atual sobre outra",
        "commitErrorDescription": "O Git informou um erro",
        "tags": {
          "title": "Tags",
          "description": "Criar e gerenciar tags",
          "sections": {
            "createTag": "Criar tag",
            "remoteForDeletingTags": "Remoto para excluir tags",
            "existingTags": "Tags existentes"
          },
          "placeholders": {
            "annotatedMessageOptional": "(mensagem anotada opcional)"
          },
          "actions": {
            "deleteRemote": "Excluir do remoto"
          },
          "confirmDeleteLocal": {
            "title": "Excluir tag local?",
            "description": "Isso remove a tag apenas localmente."
          },
          "confirmDeleteRemote": {
            "title": "Excluir tag remota?",
            "description": "Isso enviará uma solicitação de exclusão ao remoto."
          },
          "empty": "Nenhuma tag"
        },
        "autoFetchSync": {
          "title": "Busca / sincronização automática",
          "description": "Atualizações do Git em segundo plano",
          "autoFetch": "Busca automática",
          "fetchIntervalMinutes": "Intervalo de busca (minutos)",
          "autoSync": "Sincronização automática (pull + push)",
          "syncWarning": "Use com cuidado: a sincronização pode falhar se houver alterações locais ou conflitos.",
          "syncIntervalMinutes": "Intervalo de sincronização (minutos)",
          "postCommitAction": "Ação após o commit",
          "postCommitHint": "Após o commit, execute uma ação quando a branch tiver upstream e estiver à frente."
        },
        "stashDiff": {
          "title": "Diff do stash",
          "titleWithRef": "Diff do stash: {ref}",
          "description": "Inspecionar o conteúdo do patch do stash",
          "loading": "Carregando diff do stash...",
          "empty": "Nenhum diff disponível."
        },
        "createBranchFrom": {
          "title": "Criar branch a partir de",
          "description": "Criar uma branch em uma referência específica",
          "fields": {
            "branchName": "Nome da branch",
            "startPoint": "Ponto inicial"
          },
          "checkoutAfterCreate": "Fazer checkout da branch após criá-la"
        },
        "renameBranch": {
          "title": "Renomear branch",
          "description": "Renomear uma branch local"
        },
        "renameFile": {
          "title": "Renomear",
          "description": "Renomear um arquivo no repositório"
        },
        "checkoutDetached": {
          "title": "Checkout em estado destacado",
          "description": "Fazer checkout de uma referência em estado HEAD destacado",
          "placeholders": {
            "ref": "HEAD~1"
          }
        }
      },
      "workingTree": {
        "fileActionsTitle": "Ações do arquivo",
        "showMoreCount": "Mostrar mais ({shown}/{total})",
        "sections": {
          "changes": "Alterações",
          "stagedChanges": "Alterações preparadas",
          "untracked": "Não rastreados",
          "mergeChanges": "Alterações de merge",
          "stashChanges": "Alterações guardadas"
        },
        "empty": {
          "mergeChanges": "Nenhuma alteração de merge",
          "stagedChanges": "Nenhuma alteração preparada",
          "changes": "Nenhuma alteração",
          "untracked": "Nenhum arquivo não rastreado"
        },
        "actions": {
          "stageAll": "Preparar tudo",
          "stage": "Preparar",
          "stageSelected": "Preparar selecionados",
          "unstageAll": "Despreparar tudo",
          "unstage": "Despreparar",
          "unstageSelected": "Despreparar selecionados",
          "enterMultiSelect": "Seleção múltipla",
          "exitMultiSelect": "Concluir",
          "history": "Histórico",
          "ignore": "Ignorar",
          "discardChanges": "Descartar alterações",
          "discardSelected": "Descartar selecionados",
          "deleteFile": "Excluir arquivo",
          "deleteSelected": "Excluir selecionados"
        },
        "multiSelect": {
          "on": "Seleção múltipla ativada",
          "off": "Seleção múltipla desativada",
          "selectedCount": "{count} selecionados"
        },
        "confirmDiscard": {
          "title": "Descartar alterações?",
          "description": "Esta ação não pode ser desfeita."
        },
        "confirmDiscardSelected": {
          "title": "Descartar alterações selecionadas?",
          "description": "Descartar as alterações em {count} arquivo(s) selecionado(s)? Esta ação não pode ser desfeita."
        },
        "confirmDeleteSelected": {
          "title": "Excluir arquivos selecionados?",
          "description": "Excluir {count} arquivos selecionados? Esta ação não pode ser desfeita."
        },
        "confirmDeleteTracked": {
          "title": "Excluir arquivo?",
          "description": "Os arquivos rastreados serão removidos e preparados (git rm)."
        },
        "confirmDeleteUntracked": {
          "title": "Excluir arquivo?",
          "description": "Esta ação não pode ser desfeita."
        }
      },
      "sourceControl": "Controle de código-fonte",
      "sourceControlActions": "Ações de controle de código-fonte",
      "repository": "Repositório",
      "moreActions": "Mais ações",
      "noProjectSelectedTitle": "Nenhum projeto selecionado",
      "noProjectSelectedDescription": "Abra um projeto primeiro para usar o controle do código-fonte.",
      "noGitRepositoryTitle": "Nenhum repositório Git",
      "noGitRepositoryDescription": "Selecione um repositório ou inicialize um neste projeto.",
      "selectRepo": "Selecionar repositório",
      "initialize": "Inicializar",
      "historySidebar": {
        "title": "Histórico",
        "empty": "Ainda não há commits",
        "loadMore": "Carregar mais commits ({count})",
        "openDetails": "Abrir detalhes do histórico"
      },
      "branchAction": {
        "branchNamePlaceholder": "Nome da branch"
      },
      "unsafeRepo": {
        "trustRequiredTitle": "É necessário confiar no repositório",
        "blockedDefaultHint": "O Git bloqueou este repositório porque ele não está marcado como seguro nesta máquina.",
        "trustThisRepository": "Confiar neste repositório"
      },
      "signing": {
        "sshMayFailTitle": "A assinatura de commit por SSH pode falhar",
        "sshMayFailDescription": "O agente SSH não está disponível ou não tem chaves. Use um terminal para carregar as chaves (ssh-agent/ssh-add) e tente novamente."
      },
      "postCommit": {
        "description": "Próximas etapas",
        "globalDefault": "Padrão global: {label}",
        "rememberChoice": "Lembrar esta escolha para este repositório",
        "prompt": "Perguntar"
      },
      "searchRemotes": "Buscar remotos",
      "protectedBranchCommitConfirm": "A branch «{branch}» está protegida. Fazer o commit mesmo assim?",
      "protectedBranchNewBranchTitle": "Criar uma nova branch",
      "protectedBranchNewBranchDescription": "A branch «{branch}» está protegida. Informe o nome de uma nova branch para este commit.",
      "diffPane": {
        "selectFileHint": "Selecione um arquivo para ver as alterações",
        "openConflictEditor": "Abrir editor de conflitos"
      },
      "commitBox": {
        "messagePlaceholder": "Mensagem (Cmd+Enter para fazer commit)",
        "gitmojiTitle": "Gitmoji",
        "searchEmojis": "Buscar emojis",
        "emojiEmptyLabel": "Emoji"
      }
    },
    "toasts": {
      "copiedCommitHash": "Hash do commit copiado",
      "copiedRemoteUrl": "URL do remoto copiada",
      "copiedWorktreePath": "Caminho da worktree copiado",
      "selectedCommitForCompare": "Commit {hash} selecionado para comparação",
      "selectedCommitHasNoParent": "O commit selecionado não tem um commit pai para comparar",
      "noUpstreamTrackingBranch": "Nenhuma branch de acompanhamento upstream configurada",
      "noConflictsFound": "Nenhum conflito encontrado",
      "repositoryMarkedSafe": "Repositório marcado como seguro",
      "fetched": "Fetch concluído",
      "fetchedPrune": "Fetch concluído (limpeza de referências)",
      "fetchedAll": "Fetch concluído (todos os remotos)",
      "pulled": "Pull concluído",
      "pulledRebase": "Pull com rebase concluído",
      "pushed": "Push concluído",
      "pushedTags": "Tags enviadas",
      "pushedFollowTags": "Push concluído (incluindo tags alcançáveis)",
      "pushedForce": "Push forçado concluído",
      "pushedForceWithLease": "Push forçado com lease concluído",
      "synced": "Sincronização concluída",
      "syncedRebase": "Sincronização com rebase concluída",
      "committed": "Commit criado",
      "committedAll": "Commit de todas as alterações criado",
      "committedEmpty": "Commit vazio criado",
      "changesDiscarded": "Alterações descartadas",
      "alreadyIgnored": "Já está na lista de ignorados",
      "stashedChanges": "Alterações guardadas no stash",
      "prunedWorktrees": "Worktrees obsoletas removidas",
      "migratedWorktreeChanges": "Alterações da worktree migradas",
      "initializedGitRepository": "Repositório Git inicializado",
      "clonedRepository": "Repositório clonado",
      "noDifferencesFound": "Nenhuma diferença encontrada",
      "undidLastCommit": "Último commit desfeito",
      "mergeAborted": "Mesclagem abortada",
      "rebaseAborted": "Rebase abortado",
      "cherryPickContinued": "Cherry-pick continuado",
      "cherryPickSkipped": "Cherry-pick ignorado",
      "cherryPickAborted": "Cherry-pick abortado",
      "revertContinued": "Reversão continuada",
      "revertSkipped": "Reversão ignorada",
      "revertAborted": "Reversão abortada",
      "rebaseContinued": "Rebase continuado",
      "rebaseSkipped": "Rebase ignorado",
      "gitLfsInstalled": "Git LFS instalado neste repositório",
      "renamed": "Renomeado",
      "deleted": "Excluído",
      "tokenSaved": "Token salvo",
      "disabledGpgSigningForRepo": "Assinatura GPG desativada neste repositório",
      "updatedUserSigningKeyLocal": "Chave user.signingkey atualizada (local)",
      "preparedGpgAgentConfig": "Configuração do gpg-agent atualizada",
      "trackedPattern": "Padrão {pattern} rastreado",
      "lockedPath": "Caminho {path} bloqueado",
      "unlockedPath": "Bloqueio removido do caminho {path}",
      "cherryPickedShortHash": "Cherry-pick aplicado: {hash}",
      "revertedShortHash": "Commit {hash} revertido",
      "resetToShortHashWithMode": "Reset para {hash} ({mode})",
      "closedRepository": "Repositório {relative} fechado",
      "reopenedRepository": "Repositório {relative} reaberto",
      "addedWorktree": "Worktree {path} adicionada",
      "removedWorktree": "Worktree {path} removida",
      "switchedToBranch": "Alternado para a branch {branch}",
      "repositoryBusy": "Repositório ocupado ({op})",
      "authenticationRequiredForAction": "{action}: autenticação necessária",
      "sshAuthenticationRequiredForAction": "{action}: autenticação SSH necessária no terminal",
      "stashAppliedRef": "Stash {ref} aplicado",
      "stashPoppedRef": "Stash {ref} aplicado e removido",
      "stashDroppedRef": "Stash {ref} excluído",
      "droppedStashes": "{count} stashes excluídos",
      "noStashesToDrop": "Não há stashes para excluir",
      "ignoredPath": "Caminho {path} adicionado à lista de ignorados",
      "pushNeedsRemoteSetup": "Nenhum remoto/upstream encontrado. Criar um repositório no GitHub e publicar agora?",
      "createGithubRepoAction": "Criar e enviar",
      "createdGithubRepoAndPushed": "Repositório do GitHub {fullName} criado e enviado",
      "pushedToRemoteTarget": "Enviado para {remote}/{target}",
      "fetchedRemoteTarget": "Fetch de {remote}/{target} concluído",
      "pulledFromRemoteTarget": "Pull de {remote}/{target} concluído",
      "createdBranch": "Branch {name} criada",
      "checkedOutBranch": "Checkout da branch {name} concluído",
      "deletedBranch": "Branch {name} excluída",
      "deletedRemoteBranch": "Branch remota {name} excluída",
      "renamedFromTo": "{from} renomeado para {to}",
      "detachedHeadAt": "HEAD destacado em {ref}",
      "addedRemote": "Remoto {name} adicionado",
      "updatedRemoteUrl": "URL do remoto {name} atualizada",
      "removedRemote": "Remoto {name} removido",
      "mergedTarget": "{target} mesclado",
      "rebasedOntoTarget": "Rebase aplicado sobre {target}",
      "createdTag": "Tag {name} criada",
      "deletedTag": "Tag {name} excluída",
      "deletedRemoteTag": "Tag remota {name} excluída",
      "addedSubmodule": "Submódulo {path} adicionado",
      "initializedItem": "{name} inicializado",
      "updatedSubmodule": "Submódulo {path} atualizado",
      "updatedSubmodules": "Submódulos atualizados"
    },
    "remoteTargetDialogs": {
      "chooseRemoteAndBranch": "Escolha um remoto e uma branch",
      "push": {
        "title": "Enviar para...",
        "advancedHint": "Ou informe um refspec abaixo (para envio avançado).",
        "tipTerminal": "Dica: se forem solicitadas credenciais ou assinatura, use o Terminal.",
        "setUpstream": "Definir upstream (-u)"
      },
      "pull": {
        "title": "Baixar de...",
        "advancedHint": "Ou informe um refspec abaixo (para pull avançado)."
      },
      "fetch": {
        "title": "Buscar de...",
        "description": "Escolha um remoto e uma branch para buscar",
        "advancedHint": "Ou informe um refspec abaixo (para fetch avançado)."
      },
      "loadingRemoteBranches": "Carregando branches remotas...",
      "stash": {
        "title": "Guardar alterações",
        "description": "Guardar as alterações locais para usar depois",
        "stagedOnly": "Somente alterações preparadas",
        "includeUntracked": "Incluir arquivos não rastreados",
        "keepIndex": "Manter alterações preparadas (manter índice)"
      }
    },
    "authDialogs": {
      "useTerminal": "Usar o Terminal",
      "ssoBrowserHint": "Conclua a autorização SSO no navegador e tente a operação novamente.",
      "credentials": {
        "title": "Autenticação do Git",
        "description": "Informe as credenciais para continuar",
        "githubTokenOptionalTitle": "Token do GitHub (opcional)",
        "githubTokenOptionalDescription": "Se você salvar um token aqui, Fetch/Pull/Push poderão autenticar automaticamente, como no VS Code.",
        "rememberForRepo": "Lembrar neste repositório (localStorage)",
        "username": "Nome de usuário",
        "passwordOrToken": "Senha / Token"
      },
      "gitBusy": {
        "description": "Outra operação do Git está em andamento"
      },
      "sso": {
        "title": "Autorização SSO necessária",
        "description": "Sua organização exige SSO para este remoto"
      },
      "terminalHelp": {
        "title": "Requer o terminal",
        "description": "Esta ação requer um terminal interativo"
      },
      "gpgPassphrase": {
        "title": "Frase secreta GPG",
        "description": "Informe a frase secreta da sua chave GPG para assinar o commit",
        "passphraseLabel": "Frase secreta",
        "signAndCommit": "Assinar e fazer commit"
      },
      "gpgEnablePreset": {
        "title": "Ativar predefinição da frase secreta GPG?",
        "description": "Isso atualiza a configuração do gpg-agent",
        "confirmEnableRetry": "Ativar e tentar novamente",
        "body": "Seu gpg-agent não permite predefinir a frase secreta. Para permitir que o Agena assine commits depois que você informar a frase secreta na interface, podemos adicionar `allow-preset-passphrase` a `~/.gnupg/gpg-agent.conf` e reiniciar o gpg-agent."
      },
      "gpgMissingKey": {
        "title": "Nenhuma chave de assinatura GPG",
        "description": "A assinatura está ativada, mas nenhuma chave secreta está disponível",
        "defaultExplain": "Nenhuma chave secreta GPG encontrada para assinatura.",
        "setSigningKeyLocal": "Definir `user.signingkey` (local)",
        "placeholderKeyIdOrFingerprint": "ID da chave ou impressão digital",
        "setSigningKeyButton": "Definir chave de assinatura",
        "disableSigningButton": "Desativar assinatura"
      }
    },
    "actionsMenu": {
      "groups": {
        "commit": "Commit",
        "stage": "Preparação",
        "sync": "Sincronização",
        "inspect": "Inspeção",
        "branching": "Ramificações",
        "cleanup": "Limpeza"
      },
      "commit": {
        "noVerify": {
          "label": "Sem verificação",
          "description": "Ignorar os hooks de pré-commit"
        },
        "signoff": {
          "label": "Assinar",
          "description": "Adicionar o rodapé Signed-off-by"
        },
        "amend": {
          "label": "Emendar",
          "description": "Emendar o commit mais recente"
        },
        "noGpgSign": {
          "label": "Sem assinatura",
          "description": "Ignorar a assinatura GPG neste commit"
        },
        "restoreTemplate": {
          "label": "Restaurar modelo",
          "description": "Restaurar o modelo de mensagem do commit"
        },
        "discardMessage": {
          "label": "Descartar mensagem",
          "description": "Limpar a mensagem de commit atual",
          "confirmTitle": "Descartar mensagem de commit?",
          "confirmDescription": "Isso limpará o rascunho atual da mensagem de commit."
        },
        "undoLastCommit": {
          "label": "Desfazer último commit",
          "description": "Executar git reset --soft HEAD~1",
          "confirmTitle": "Desfazer o último commit?",
          "confirmDescription": "As alterações nos arquivos serão mantidas, mas o commit mais recente será removido."
        },
        "commitAll": {
          "label": "Commit de tudo",
          "description": "Criar commit com as alterações rastreadas e não rastreadas"
        },
        "commitEmpty": {
          "label": "Commit vazio",
          "description": "Criar um commit vazio com a mensagem atual"
        }
      },
      "stage": {
        "stageAll": {
          "label": "Preparar tudo",
          "description": "Preparar todas as alterações rastreadas e não rastreadas"
        },
        "stageAllTracked": {
          "label": "Preparar arquivos rastreados",
          "description": "Preparar somente as alterações em arquivos rastreados"
        },
        "stageAllUntracked": {
          "label": "Preparar arquivos não rastreados",
          "description": "Preparar somente arquivos não rastreados"
        },
        "stageAllMerge": {
          "label": "Preparar todas as resoluções de merge",
          "description": "Preparar as resoluções de conflitos de merge"
        }
      },
      "sync": {
        "fetch": {
          "label": "Fetch",
          "description": "Buscar atualizações do repositório remoto"
        },
        "fetchFrom": {
          "label": "Buscar de...",
          "description": "Escolher o remoto e a branch de onde buscar"
        },
        "fetchPrune": {
          "label": "Fetch (limpar referências)",
          "description": "Buscar atualizações e remover referências excluídas"
        },
        "fetchAll": {
          "label": "Fetch (todos)",
          "description": "Buscar atualizações de todos os remotos"
        },
        "pull": {
          "label": "Pull",
          "description": "Integrar alterações do remoto"
        },
        "pullRebase": {
          "label": "Pull (rebase)",
          "description": "Integrar alterações do remoto com rebase da branch atual"
        },
        "pullFrom": {
          "label": "Pull de...",
          "description": "Escolher o remoto e a branch de onde integrar alterações"
        },
        "push": {
          "label": "Push",
          "description": "Enviar commits locais para o remoto"
        },
        "pushForce": {
          "label": "Push (forçado)",
          "description": "Forçar o envio do histórico da branch para o remoto",
          "confirmTitle": "Forçar push?",
          "confirmDescription": "Isso pode sobrescrever o histórico remoto.",
          "confirmText": "Forçar push"
        },
        "pushForceWithLease": {
          "label": "Push (forçado com lease)",
          "description": "Forçar o envio protegendo contra referências remotas desatualizadas",
          "confirmTitle": "Forçar push (com lease)?",
          "confirmDescription": "Isso pode sobrescrever o histórico remoto.",
          "confirmText": "Forçar push"
        },
        "pushTags": {
          "label": "Enviar tags",
          "description": "Enviar tags locais para o remoto"
        },
        "pushFollowTags": {
          "label": "Push (incluir tags alcançáveis)",
          "description": "Enviar commits e tags alcançáveis"
        },
        "pushTo": {
          "label": "Enviar para...",
          "description": "Escolher o remoto e a branch de destino"
        },
        "syncNow": {
          "label": "Sincronizar",
          "description": "Buscar e enviar alterações em uma única operação"
        },
        "syncRebase": {
          "label": "Sincronizar (rebase)",
          "description": "Executar pull --rebase e push em uma única operação"
        }
      },
      "inspect": {
        "branches": {
          "label": "Branches...",
          "description": "Explorar e alternar entre branches"
        },
        "remotes": {
          "label": "Remotos...",
          "description": "Gerenciar remotos do Git"
        },
        "worktrees": {
          "label": "Worktrees...",
          "description": "Gerenciar worktrees vinculadas"
        },
        "history": {
          "label": "Histórico...",
          "description": "Inspecionar o histórico de commits"
        },
        "compare": {
          "label": "Comparar...",
          "description": "Comparar referências e branches"
        },
        "compareUpstream": {
          "label": "Comparar com upstream",
          "description": "Comparar a branch atual com a branch de acompanhamento"
        },
        "tags": {
          "label": "Tags...",
          "description": "Criar e gerenciar tags"
        },
        "submodules": {
          "label": "Submódulos...",
          "description": "Inspecionar o status dos submódulos"
        },
        "lfs": {
          "label": "Git LFS...",
          "description": "Inspecionar o status do Git LFS"
        },
        "autoFetch": {
          "label": "Busca automática...",
          "description": "Configurar a busca em segundo plano"
        },
        "stash": {
          "label": "Stash...",
          "description": "Abrir operações de stash"
        },
        "stashStagedOnly": {
          "label": "Stash (somente preparadas)",
          "description": "Guardar somente as alterações preparadas"
        },
        "stashKeepIndex": {
          "label": "Stash (manter índice)",
          "description": "Guardar somente as alterações não preparadas"
        },
        "stashIncludeUntracked": {
          "label": "Stash (incluir não rastreados)",
          "description": "Guardar alterações em arquivos rastreados e não rastreados"
        },
        "stashDropAll": {
          "label": "Stash (excluir todos)",
          "description": "Excluir todas as entradas de stash",
          "confirmTitle": "Excluir todos os stashes?",
          "confirmDescription": "Isso removerá permanentemente todas as entradas de stash.",
          "confirmText": "Excluir todos"
        }
      },
      "branching": {
        "mergeBranch": {
          "label": "Mesclar branch...",
          "description": "Mesclar outra branch na branch atual"
        },
        "rebaseBranch": {
          "label": "Rebase da branch...",
          "description": "Aplicar rebase da branch atual sobre outra branch"
        },
        "createBranchFrom": {
          "label": "Criar branch a partir de...",
          "description": "Criar uma branch a partir da referência selecionada"
        },
        "checkoutDetached": {
          "label": "Checkout destacado...",
          "description": "Fazer checkout de um commit ou referência em modo destacado"
        }
      },
      "cleanup": {
        "cleanUntracked": {
          "label": "Limpar não rastreados",
          "description": "Excluir arquivos e pastas não rastreados",
          "confirmTitle": "Limpar arquivos não rastreados?",
          "confirmDescription": "Isso excluirá arquivos e pastas não rastreados."
        },
        "cleanAll": {
          "label": "Limpar tudo (incluindo ignorados)",
          "description": "Excluir arquivos não rastreados e ignorados",
          "confirmTitle": "Limpar tudo (incluindo ignorados)?",
          "confirmDescription": "Isso excluirá arquivos não rastreados e ignorados."
        },
        "discardAllTracked": {
          "label": "Descartar alterações rastreadas",
          "description": "Redefinir o índice e a árvore de trabalho dos arquivos rastreados",
          "confirmTitle": "Descartar todas as alterações rastreadas?",
          "confirmDescription": "Isso redefine o índice e a árvore de trabalho dos arquivos rastreados."
        }
      }
    },
    "fields": {
      "remote": "Remoto",
      "branch": "Branch",
      "refspecOptional": "Refspec (opcional)",
      "messageOptional": "Mensagem (opcional)"
    },
    "errors": {
      "worktreeOutsideProjectRoot": "A árvore de trabalho está fora da raiz do projeto",
      "forcePushDisabledByPolicy": "Push forçado desativado pela política",
      "commitsWithoutVerificationDisabledByPolicy": "Commits sem verificação estão desativados pela política",
      "nowOnNewBranchCommitAgain": "Agora você está em uma nova branch. Faça o commit novamente para continuar.",
      "blockedTerminalUnsupportedRepoPath": "Comando do terminal bloqueado: caminho de repositório não compatível",
      "blockedTerminalOutsideAllowlist": "Comando do terminal bloqueado por estar fora da lista de permissões confiáveis",
      "failedToPrepareTerminalHandoff": "Falha ao preparar a transferência do comando para o terminal"
    }
  },
  "files": {
    "errors": {
      "noProjectSelected": "Nenhum projeto selecionado",
      "filenameRequired": "O nome do arquivo é obrigatório",
      "folderNameRequired": "O nome da pasta é obrigatório",
      "nameRequired": "O nome é obrigatório",
      "moveTargetOutsideWorkspace": "O destino da movimentação deve permanecer dentro do espaço de trabalho",
      "moveTargetInsideSelection": "Não é possível mover uma pasta para dentro dela mesma"
    },
    "actions": {
      "copyPath": "Copiar caminho",
      "copyAbsolutePath": "Copiar caminho absoluto"
    },
    "confirmDiscard": {
      "title": "Alterações não salvas",
      "description": "Salvar suas alterações antes de continuar?",
      "discard": "Descartar",
      "saveChanges": "Salvar alterações"
    },
    "refreshConflict": {
      "title": "Conflito ao atualizar",
      "description": "Este arquivo foi alterado no disco enquanto você ainda tinha edições de rascunho na web ({path}). Escolha qual versão manter.",
      "keepLocalDraft": "Manter rascunho da web",
      "useRefreshedContent": "Usar conteúdo atualizado",
      "remoteLargeHint": "A versão atualizada agora é grande ({size}) e será reaberta no modo de blocos."
    },
    "dialog": {
      "namePlaceholder": "Nome",
      "title": {
        "createFile": "Criar arquivo",
        "createFolder": "Criar pasta",
        "rename": "Renomear"
      },
      "rootFallback": "raiz",
      "description": {
        "createFile": "Criar um novo arquivo em {base}",
        "createFolder": "Criar uma nova pasta em {base}",
        "rename": "Renomear {name}"
      }
    },
    "empty": {
      "noProject": {
        "title": "Nenhum projeto selecionado",
        "mobileDescription": "Escolha primeiro uma pasta de projeto; a árvore de arquivos aparecerá aqui.",
        "desktopDescription": "Selecione um projeto (ou uma sessão) para navegar pelos arquivos."
      }
    },
    "search": {
      "mode": {
        "files": "Arquivos",
        "content": "Conteúdo"
      },
      "files": {
        "placeholder": "Buscar arquivos",
        "hint": "Digite para buscar nomes e caminhos de arquivos.",
        "noQuery": "Nenhuma consulta de busca.",
        "noFilesFound": "Nenhum arquivo encontrado.",
        "results": "{count} resultados",
        "truncatedResults": "{count} resultados exibidos · a busca foi parcial ou truncada"
      },
      "content": {
        "placeholder": "Buscar",
        "hint": "Digite para buscar no conteúdo dos arquivos.",
        "noMatchesFound": "Nenhuma correspondência encontrada.",
        "showReplace": "Mostrar substituição",
        "hideReplace": "Ocultar substituição",
        "replacePlaceholder": "Substituir",
        "matchCase": "Diferenciar maiúsculas de minúsculas",
        "matchWholeWord": "Corresponder à palavra inteira",
        "useRegex": "Usar expressão regular",
        "matchSingle": "correspondência",
        "matchPlural": "correspondências",
        "fileSingle": "arquivo",
        "filePlural": "arquivos",
        "resultSummary": "{matches} {matchWord} em {files} {fileWord}",
        "resultSummaryLimited": "{text} (limitado)"
      }
    },
    "scope": {
      "workspaceRoot": "raiz do espaço de trabalho",
      "options": {
        "workspace": {
          "label": "Espaço de trabalho"
        },
        "selected": {
          "label": "Pasta selecionada",
          "emptyDescription": "Selecione primeiro uma pasta no explorador"
        },
        "activeFile": {
          "label": "Pasta do arquivo atual",
          "emptyDescription": "Abra um arquivo primeiro"
        }
      }
    },
    "timeline": {
      "commits": "Commits",
      "loadedCount": "{loaded}/{total} carregados",
      "errors": {
        "onlyTextFiles": "A comparação da linha do tempo está disponível apenas para arquivos de texto",
        "cannotOpen": "Não é possível abrir a linha do tempo para este item",
        "missingInCommit": "O arquivo não existe neste commit.",
        "binaryUnavailable": "O commit selecionado armazena este arquivo como binário. A comparação de texto não está disponível.",
        "noBlameData": "Nenhum dado de blame foi retornado para o arquivo atual. Tente recarregar o blame."
      },
      "info": {
        "truncated": "O conteúdo do commit é grande e foi truncado para a pré-visualização."
      }
    },
    "toasts": {
      "refreshFileFailed": "Falha ao atualizar o arquivo",
      "fileRefreshed": "Arquivo atualizado",
      "fileAlreadyLatest": "O arquivo já está atualizado",
      "refreshKeptLocalDraft": "Rascunho da web mantido",
      "saveFileBeforeGitHunkActions": "Salve o arquivo antes de aplicar ações a blocos do Git",
      "hunkStaged": "Bloco preparado",
      "hunkUnstaged": "Bloco retirado da preparação",
      "hunkDiscarded": "Bloco descartado",
      "saveCurrentFileBeforeReplaceAcrossFiles": "Salve o arquivo atual antes de substituir em vários arquivos",
      "searchAgainBeforeReplace": "Execute a busca novamente antes de substituir.",
      "replaceAcrossFiles": {
        "partial": "{matches} correspondências substituídas em {files} arquivos antes de parar em {path}: {error}",
        "truncated": "Alguns arquivos ficaram fora deste lote de substituição. Faça outra busca para revisar as correspondências restantes.",
        "replacedMatch": "{count} correspondência substituída{suffix}",
        "replacedMatches": "{count} correspondências substituídas{suffix}",
        "skippedFileSuffix": ", {count} arquivo ignorado",
        "skippedFilesSuffix": ", {count} arquivos ignorados"
      },
      "matchReplaced": "Correspondência substituída",
      "noMatchesReplaced": "Nenhuma correspondência substituída",
      "uploadedFile": "{count} arquivo enviado",
      "uploadedFiles": "{count} arquivos enviados",
      "uploadPartial": "{success} arquivos enviados, {failed} falharam",
      "uploadFailed": "Falha ao enviar {count} arquivos",
      "movedCount": "{count} itens movidos",
      "movedPartial": "{success} itens movidos, {failed} falharam",
      "moveFailedCount": "Falha ao mover {count} itens",
      "savedFile": "Arquivo salvo",
      "fileCreated": "Arquivo criado",
      "folderCreated": "Pasta criada",
      "renamed": "Renomeado",
      "deleted": "Excluído",
      "deleteFailed": "Falha ao excluir",
      "batchDeletedSingle": "1 item excluído",
      "batchDeletedMultiple": "{count} itens excluídos",
      "batchDeletedPartial": "{success} itens excluídos, {failed} falharam",
      "batchDeleteFailed": "Falha ao excluir {count} itens",
      "selectSomeLinesToInsert": "Selecione algumas linhas para inserir",
      "selectChatSessionFirst": "Selecione primeiro uma sessão de conversa",
      "insertedIntoChatComposer": "Inserido no campo de mensagem da conversa"
    },
    "explorer": {
      "title": "Explorador",
      "navigation": {
        "goUp": "Ir para a pasta pai"
      },
      "toolbar": {
        "newFile": "Novo arquivo",
        "newFolder": "Nova pasta",
        "refreshTree": "Atualizar árvore",
        "actions": "Ações do explorador"
      },
      "view": {
        "tree": "Árvore",
        "search": "Buscar"
      },
      "actionMenu": {
        "groups": {
          "actions": "Ações do explorador",
          "visibility": "Visibilidade"
        },
        "uploadFiles": "Enviar arquivos",
        "uploading": "Enviando...",
        "collapseFolders": "Recolher pastas",
        "collapseFoldersDescription": "Recolher todos os diretórios expandidos",
        "showHiddenFiles": "Mostrar arquivos ocultos",
        "showHiddenFilesDescription": "Incluir arquivos ocultos no explorador",
        "showGitignoredFiles": "Mostrar arquivos ignorados pelo Git",
        "showGitignoredFilesDescription": "Desativar o filtro do .gitignore"
      },
      "actions": {
        "actions": "Ações",
        "fileActions": "Ações de arquivo",
        "folderActions": "Ações de pasta",
        "rename": "Renomear",
        "delete": "Excluir",
        "deleteFileTitle": "Excluir arquivo?",
        "deleteFolderTitle": "Excluir pasta?",
        "deleteDescription": "Excluir {name}? Esta ação não pode ser desfeita."
      },
      "inlineCreate": {
        "folderName": "nome da pasta",
        "fileName": "nome do arquivo"
      },
      "selection": {
        "selectedCount": "{count} selecionados",
        "multiSelectOn": "Seleção múltipla ativada",
        "multiSelectOff": "Seleção múltipla desativada",
        "enterMultiSelect": "Selecionar vários",
        "exitMultiSelect": "Concluir",
        "longPressHint": "Mantenha um item pressionado para adicioná-lo ou removê-lo da seleção",
        "clearSelection": "Limpar seleção",
        "bulkMove": "Mover selecionados",
        "bulkMoveTitle": "Mover itens selecionados",
        "bulkMoveDescription": "Mover {count} itens selecionados para o caminho da pasta:",
        "bulkMovePlaceholder": "Caminho da pasta de destino (relativo ou absoluto)",
        "bulkDelete": "Excluir selecionados",
        "bulkDeleteTitle": "Excluir itens selecionados?",
        "bulkDeleteDescription": "Excluir {count} itens selecionados? Esta ação não pode ser desfeita."
      },
      "searchUnavailable": "A busca está indisponível."
    },
    "viewer": {
      "title": {
        "selectFile": "Selecione um arquivo"
      },
      "viewMenu": {
        "title": "Configurações de visualização",
        "groupTitle": "Opções de visualização",
        "blame": {
          "enable": "Ativar blame",
          "disable": "Desativar blame",
          "description": "Alternar marcadores de autoria dos commits"
        },
        "timeline": {
          "enable": "Ativar linha do tempo",
          "disable": "Desativar linha do tempo",
          "description": "Comparar dois commits no editor"
        },
        "wrap": {
          "enable": "Ativar quebra de linha",
          "disable": "Desativar quebra de linha",
          "description": "Alternar a quebra de linha no editor"
        },
        "autosave": {
          "enable": "Ativar salvamento automático",
          "disable": "Desativar salvamento automático",
          "description": "Salvar automaticamente as alterações de texto"
        }
      },
      "save": {
        "titleDirty": "Salvar (Ctrl/Cmd+S)",
        "titleSaved": "Salvo"
      },
      "actions": {
        "refreshContents": "Atualizar conteúdo do arquivo",
        "refreshingContents": "Atualizando conteúdo do arquivo...",
        "copyContents": "Copiar conteúdo"
      },
      "largeFile": {
        "title": "Arquivo grande detectado",
        "description": "Este arquivo tem {size}. Carregar arquivos muito grandes pode afetar o desempenho do editor.",
        "initialLoadHint": "Comece carregando os primeiros {size} e, se necessário, carregue mais.",
        "continue": "Continuar carregando",
        "loadMore": "Carregar mais",
        "partialLoadedBanner": "Carregados {loaded} de {total}.",
        "pendingStatus": "Arquivo grande: {size}. Continue carregando para visualizar.",
        "partialLoadedStatus": "Carregados parcialmente {loaded}/{total}"
      },
      "status": {
        "loadingTimelineCommits": "Carregando commits da linha do tempo...",
        "loadingBlame": "Carregando blame...",
        "noBlameYet": "Ainda não há linhas de blame disponíveis para este arquivo.",
        "showingBlameLastSaved": "Exibindo o blame do último conteúdo salvo do arquivo.",
        "loadingChanges": "Carregando alterações de {scope}...",
        "noChanges": "Nenhuma alteração de {scope}.",
        "hunksSummary": "{count} blocos · {range}",
        "loadingImage": "Carregando imagem…",
        "loadingPdf": "Carregando PDF…",
        "loadingAudio": "Carregando áudio…",
        "loadingVideo": "Carregando vídeo…"
      },
      "markdown": {
        "mode": {
          "source": "Código-fonte",
          "preview": "Pré-visualização",
          "split": "Dividida"
        }
      },
      "gitDiffMode": {
        "staged": "preparado",
        "workingTree": "árvore de trabalho"
      },
      "gitDiffRange": {
        "headToIndex": "HEAD -> Índice",
        "indexToWorkingTree": "Índice -> Árvore de trabalho"
      },
      "empty": {
        "mobileDescription": "Escolha um arquivo no painel de arquivos para inspecioná-lo ou editá-lo.",
        "desktopDescription": "Escolha um arquivo na árvore."
      },
      "binaryPreviewUnavailable": "A pré-visualização de arquivos binários não está disponível.",
      "pdfPreviewTitle": "Pré-visualização de PDF",
      "imageAltFallback": "Imagem",
      "timeline": {
        "selectCommitTitle": "Selecionar commit",
        "selectCommitShort": "Selecionar commit",
        "searchPlaceholder": "Buscar hash, mensagem, autor, e-mail, referências",
        "emptyText": "Nenhum commit corresponde à busca.",
        "selectBothCommitsHint": "Selecione os dois commits nos menus suspensos acima para começar a comparar.",
        "noMessage": "(sem mensagem)"
      },
      "selection": {
        "clearAria": "Limpar seleção",
        "notePlaceholder": "Adicionar uma nota para o chat (opcional)",
        "insertHint": "Inserir as linhas selecionadas no campo de mensagem do chat",
        "insertButton": "Inserir"
      }
    }
  },
  "terminal": {
    "defaults": {
      "defaultFolder": "Padrão",
      "gitFolder": "Git",
      "gitTerminal": "Terminal do Git"
    },
    "keybar": {
      "titles": {
        "ctrlToggle": "Ctrl (alternar)",
        "altToggle": "Alt (alternar)",
        "shiftToggle": "Shift (alternar)"
      },
      "keys": {
        "ctrl": "Ctrl",
        "alt": "Alt",
        "shift": "Shift",
        "esc": "Esc",
        "tab": "Tab",
        "backspace": "Bksp",
        "enter": "Enter",
        "left": "Esquerda",
        "down": "Baixo",
        "up": "Cima",
        "right": "Direita",
        "home": "Home",
        "end": "End",
        "pageUp": "PgUp",
        "pageDown": "PgDn"
      }
    },
    "shortcuts": {
      "openHelp": "Mostrar atalhos do terminal",
      "title": "Atalhos do terminal",
      "description": "Atalhos de área de transferência e interrupção no estilo Linux enquanto o terminal está em foco.",
      "copyOrInterrupt": "Copiar o texto selecionado ou enviar interrupção (^C) quando nada estiver selecionado",
      "copySelection": "Copiar o texto selecionado no terminal",
      "pasteInput": "Colar o texto da área de transferência na entrada do terminal",
      "macMetaCompat": "Compatibilidade do macOS para copiar e colar"
    },
    "emptyState": {
      "title": "Selecione uma sessão do terminal",
      "description": "Use o painel de terminais para escolher uma sessão existente ou criar uma nova.",
      "actionLabel": "Abrir painel de terminais"
    },
    "sidebar": {
      "title": "Terminais",
      "newFolder": "Nova pasta",
      "refreshSessions": "Atualizar sessões",
      "searchPlaceholder": "Buscar pastas e sessões",
      "searchAria": "Buscar pastas e sessões do terminal",
      "clearSearch": "Limpar busca",
      "emptyTitle": "Nenhuma pasta ou sessão correspondente",
      "emptyHint": "Tente outra palavra-chave.",
      "multiSelect": {
        "on": "Seleção múltipla: ativada",
        "off": "Seleção múltipla: desativada",
        "selectedCount": "{count} selecionados"
      },
      "expandFolder": "Expandir pasta",
      "collapseFolder": "Recolher pasta",
      "pinned": "Fixados",
      "recent": "Recentes"
    },
    "folder": {
      "new": "Nova pasta",
      "namePlaceholder": "Nome da pasta",
      "create": "Criar pasta",
      "saveName": "Salvar nome da pasta",
      "rename": "Renomear pasta",
      "delete": "Excluir pasta",
      "newTerminalInFolder": "Novo terminal na pasta",
      "newTerminalInThisFolder": "Novo terminal nesta pasta",
      "deleteConfirmTitle": "Excluir pasta?",
      "deleteConfirmDescription": "As sessões desta pasta serão movidas para outra pasta.",
      "deleteConfirmText": "Excluir"
    },
    "active": {
      "noneSelected": "Nenhum terminal selecionado"
    },
    "session": {
      "namePlaceholder": "Nome do terminal",
      "create": "Criar",
      "creating": "Criando...",
      "cancelRename": "Cancelar renomeação",
      "saveRename": "Salvar renomeação"
    },
    "connection": {
      "connectLabel": "Iniciar sessão de terminal",
      "disconnectLabel": "Parar sessão de terminal",
      "disconnectDescription": "Isso encerra a sessão no backend. Você pode iniciá-la novamente a qualquer momento."
    },
    "dialogs": {
      "newTerminal": {
        "title": "Novo terminal",
        "description": "Criar terminal em {folder}"
      },
      "renameTerminal": {
        "title": "Renomear terminal",
        "description": "Atualizar o nome do terminal"
      }
    },
    "actions": {
      "title": "Ações do terminal",
      "titleWithName": "Ações do terminal: {name}",
      "enterMultiSelect": "Seleção múltipla",
      "exitMultiSelect": "Concluir",
      "deleteSelected": "Excluir selecionados",
      "deleteSelectedConfirmTitle": "Excluir as sessões de terminal selecionadas?",
      "deleteSelectedConfirmDescription": "Excluir {count} sessões de terminal selecionadas.",
      "rename": "Renomear terminal",
      "disconnectStream": "Parar sessão",
      "disconnectConfirmTitle": "Parar sessão de terminal?",
      "disconnectConfirmDescription": "Isso encerra o terminal no backend. Você poderá iniciá-lo novamente mais tarde.",
      "disconnect": "Parar",
      "pin": "Fixar terminal",
      "unpin": "Desafixar terminal",
      "delete": "Excluir terminal",
      "deleteConfirmTitle": "Excluir sessão de terminal?",
      "deleteConfirmDescription": "Esta ação encerrará e removerá esta sessão de terminal.",
      "deleteConfirmText": "Excluir"
    },
    "errors": {
      "deleteAllInFolder": "Falha ao excluir todos os terminais desta pasta",
      "streamRecoveredPartialHistory": "O fluxo do terminal foi recuperado com histórico parcial; algumas saídas mais antigas podem estar ausentes",
      "terminalExited": "O terminal foi encerrado",
      "sessionNoLongerExists": "A sessão de terminal não existe mais",
      "failedToStartSession": "Falha ao iniciar a sessão de terminal",
      "failedToStopSession": "Falha ao parar a sessão de terminal",
      "failedToSendInput": "Falha ao enviar entrada para o terminal",
      "failedToResize": "Falha ao redimensionar o terminal",
      "failedToCloseSession": "Falha ao fechar a sessão de terminal",
      "failedToRemoveSession": "Falha ao remover a sessão de terminal"
    }
  },
  "workspaceDock": {
    "loading": "Carregando...",
    "refresh": "Atualizar painel",
    "close": "Fechar painel",
    "noDirectory": "Selecione o diretório de um projeto para inspecionar os dados.",
    "placementRight": "Posicionar painel à direita",
    "placementBottom": "Posicionar painel na parte inferior",
    "openGit": "Abrir visualização completa do Git",
    "openTerminal": "Abrir visualização completa do Terminal",
    "git": {
      "tab": "Git",
      "branch": "Branch",
      "noRepo": "Nenhum repositório",
      "clean": "A árvore de trabalho está limpa.",
      "staged": "Preparadas",
      "unstaged": "Alteradas",
      "untracked": "Não rastreadas"
    },
    "changes": {
      "tab": "Alterações",
      "currentSession": "Sessão atual",
      "noSession": "Nenhuma sessão ativa",
      "noSessionHint": "Selecione uma sessão de conversa para inspecionar aqui as alterações nos arquivos.",
      "openFile": "Abrir arquivo no painel de arquivos"
    },
    "files": {
      "tab": "Arquivos"
    },
    "terminal": {
      "tab": "Terminal",
      "active": "Sessão ativa",
      "none": "Nenhuma sessão ativa",
      "sessions": "Sessões",
      "empty": "Ainda não há sessões de terminal.",
      "current": "Atual"
    },
    "preview": {
      "tab": "Pré-visualização",
      "refresh": "Atualizar pré-visualização",
      "openInWindow": "Abrir em uma nova janela",
      "addDialog": {
        "title": "Adicionar sessão de pré-visualização",
        "description": "Insira uma URL de destino para criar uma sessão de pré-visualização gerenciada pelo Studio."
      },
      "editDialog": {
        "title": "Editar sessão de pré-visualização",
        "description": "Atualize a URL de destino desta sessão de pré-visualização."
      },
      "renameDialog": {
        "title": "Renomear sessão de pré-visualização",
        "description": "Atualize o nome desta sessão de pré-visualização."
      },
      "sidebar": {
        "searchPlaceholder": "Buscar sessões de pré-visualização",
        "searchAria": "Buscar sessões de pré-visualização",
        "noMatchingSessions": "Nenhuma sessão de pré-visualização correspondente.",
        "multiSelect": {
          "on": "Seleção múltipla: ativada",
          "off": "Seleção múltipla: desativada",
          "selectedCount": "{count} selecionadas"
        },
        "confirmDeleteSelected": {
          "title": "Excluir as sessões de pré-visualização selecionadas?",
          "description": "Excluir {count} sessões de pré-visualização selecionadas. Esta ação não pode ser desfeita."
        },
        "sections": {
          "chat": {
            "title": "Conversa atual",
            "emptyNoSession": "Selecione uma sessão de conversa para ver as pré-visualizações.",
            "empty": "Ainda não há pré-visualizações para esta sessão de conversa."
          },
          "directory": {
            "title": "Diretório atual",
            "emptyNoDirectory": "Selecione um diretório para ver as pré-visualizações.",
            "empty": "Ainda não há pré-visualizações para este diretório."
          },
          "all": {
            "title": "Todas as pré-visualizações",
            "empty": "Ainda não há pré-visualizações."
          }
        },
        "actions": {
          "enterMultiSelect": "Selecionar vários",
          "exitMultiSelect": "Concluir",
          "deleteSelected": "Excluir selecionadas",
          "rename": "Renomear",
          "renameDescription": "Alterar o nome desta sessão de pré-visualização.",
          "start": "Iniciar",
          "startDescription": "Iniciar o comando de pré-visualização desta sessão.",
          "startFailed": "Falha ao iniciar a sessão de pré-visualização. Detalhes: {detail}",
          "stop": "Parar",
          "stopDescription": "Parar o comando de pré-visualização desta sessão.",
          "stopFailed": "Falha ao parar a sessão de pré-visualização. Detalhes: {detail}",
          "editUrl": "Editar URL de destino",
          "editUrlDescription": "Atualizar o destino para o qual esta sessão encaminha as solicitações.",
          "editUrlUnavailable": "Esta sessão não tem uma URL de destino para editar.",
          "deleteDescription": "Remover esta sessão de pré-visualização.",
          "deleteFailed": "Falha ao excluir a sessão de pré-visualização. Detalhes: {detail}",
          "deleteConfirmTitle": "Excluir sessão de pré-visualização?",
          "deleteConfirmDescription": "Isso removerá a sessão de pré-visualização do Studio. Esta ação não pode ser desfeita."
        }
      },
      "sessionsTitle": "Sessões",
      "directoryLabel": "Diretório:",
      "targetLabel": "URL de destino (apenas exibição):",
      "proxyPathLabel": "Via proxy:",
      "urlEmpty": "Nenhuma sessão de pré-visualização selecionada",
      "scope": {
        "current": "Atual",
        "all": "Todas"
      },
      "viewportDesktop": "Computador",
      "viewportMobile": "Celular",
      "touchSimulationOn": "Simulação de toque: ativada",
      "touchSimulationOff": "Simulação de toque: desativada",
      "viewportWidth": "Largura da área de visualização",
      "viewportHeight": "Altura da área de visualização",
      "rotateViewport": "Girar área de visualização",
      "zoomOut": "Diminuir zoom",
      "zoomIn": "Aumentar zoom",
      "zoomReset": "Redefinir zoom",
      "resizeViewport": "Redimensionar área de visualização",
      "iframeTitle": "Pré-visualização ao vivo do espaço de trabalho",
      "emptyState": {
        "sessionIdLabel": "Nome",
        "sessionIdPlaceholder": "vite-main",
        "sessionIdHelp": "Use letras, números, '_' ou '-'.",
        "runDirectoryLabel": "Diretório de execução",
        "runDirectoryPlaceholder": "/path/to/project",
        "commandLabel": "Comando",
        "commandPlaceholder": "bun",
        "argsLabel": "Argumentos",
        "argsPlaceholder": "run dev -- --port 5173",
        "logsPathLabel": "Arquivo de log",
        "logsPathPlaceholder": ".agena/preview/vite-main.log",
        "targetUrlLabel": "URL de destino",
        "targetUrlPlaceholder": "http://127.0.0.1:8000",
        "addAction": "Adicionar",
        "autoDetectAction": "Detectar automaticamente",
        "createLoading": "Criando sessão de pré-visualização...",
        "discoverLoading": "Detectando pré-visualização local...",
        "directoryEmpty": "Nenhum diretório atual selecionado",
        "directoryRequired": "Selecione primeiro um diretório do espaço de trabalho.",
        "sessionIdRequired": "Informe um nome primeiro.",
        "sessionIdInvalid": "Nome inválido. Use letras, números, '_' ou '-'.",
        "runDirectoryRequired": "Informe um diretório de execução.",
        "commandRequired": "Informe um comando.",
        "argsRequired": "Informe os argumentos.",
        "logsPathRequired": "Informe o caminho do arquivo de log.",
        "targetUrlRequired": "Informe primeiro uma URL de destino.",
        "createFailed": "Falha ao criar a sessão de pré-visualização. Detalhes: {detail}",
        "discoverFailed": "Falha ao detectar automaticamente o destino da pré-visualização. Detalhes: {detail}"
      },
      "states": {
        "loading": "Carregando pré-visualização...",
        "emptyTitle": "Ainda não há sessões de pré-visualização",
        "emptyDescription": "Use o botão + para adicionar uma URL de destino e criar uma sessão de pré-visualização gerenciada pelo Studio.",
        "errorTitle": "Pré-visualização indisponível",
        "errorDescription": "Não foi possível carregar a pré-visualização. Verifique se o servidor de desenvolvimento está acessível.",
        "noSessionsForScope": "Ainda não há sessões de pré-visualização neste escopo.",
        "activeSessionMissing": "A sessão de pré-visualização selecionada não está mais disponível.",
        "missingProxyBasePath": "A sessão de pré-visualização selecionada não tem um caminho de proxy.",
        "sessionsFetchFailed": "Falha ao carregar as sessões de pré-visualização. Detalhes: {detail}",
        "sessionsFetchFailedNoDetail": "Nenhum detalhe adicional",
        "proxyHttpError": "O proxy de pré-visualização retornou um erro (HTTP {status}). Detalhes: {detail}",
        "proxyHttpErrorNoDetail": "O proxy não forneceu mais detalhes",
        "proxyRequestFailed": "Não foi possível acessar o proxy de pré-visualização. Verifique o status do serviço."
      }
    }
  },
  "editor": {
    "loadingDiff": "Carregando editor de diferenças…",
    "find": {
      "placeholderFind": "Localizar",
      "placeholderReplace": "Substituir",
      "previousMatch": "Correspondência anterior",
      "nextMatch": "Próxima correspondência",
      "matchCase": "Diferenciar maiúsculas de minúsculas",
      "wholeWord": "Palavra inteira",
      "useRegex": "Usar expressão regular",
      "close": "Fechar busca",
      "invalid": "Inválido",
      "showReplace": "Mostrar controles de substituição",
      "hideReplace": "Ocultar controles de substituição",
      "replaceCurrent": "Substituir correspondência atual",
      "replaceAll": "Substituir todas as correspondências"
    }
  },
  "ui": {
    "inlineSearch": {
      "placeholder": "Buscar…",
      "useValue": "Usar \"{value}\"",
      "truncated": "Digite para buscar. Exibindo {shown} de {total}.",
      "noMatches": "Nenhum resultado."
    },
    "imageViewer": {
      "title": "Visualizador de imagens"
    },
    "codeEditor": {
      "loading": "Carregando editor...",
      "loadFailed": "Falha ao carregar o editor."
    },
    "diffViewer": {
      "empty": "Nenhum conteúdo de diferenças.",
      "renderFailed": "Falha ao renderizar as diferenças"
    },
    "pathPicker": {
      "actions": {
        "browse": "Procurar",
        "go": "Ir",
        "createFolder": "Criar pasta",
        "foldersOnly": "Somente pastas"
      },
      "titles": {
        "hiddenShown": "Ocultos: exibidos",
        "hiddenHidden": "Ocultos: ocultos",
        "gitignoredShown": "Ignorados pelo Git: exibidos",
        "gitignoredHidden": "Ignorados pelo Git: ocultos"
      },
      "aria": {
        "toggleHidden": "Alternar arquivos ocultos",
        "toggleGitignored": "Alternar arquivos ignorados pelo Git"
      },
      "placeholders": {
        "path": "/path/to/folder",
        "filter": "Filtrar...",
        "newFolderName": "Nome da nova pasta"
      },
      "errors": {
        "invalidFolderName": "Informe um nome de pasta válido (sem / ou \\).",
        "selectDirectoryFirst": "Abra primeiro um diretório pai.",
        "createFolderFailed": "Falha ao criar a pasta: {error}"
      },
      "empty": {
        "noMatches": "Nenhum resultado"
      }
    }
  },
  "errors": {
    "preview": {
      "sessionIdRequired": "O ID da sessão é obrigatório",
      "directoryRequired": "O diretório é obrigatório",
      "runDirectoryRequired": "O diretório de execução é obrigatório",
      "commandRequired": "O comando é obrigatório",
      "logsPathRequired": "O caminho do arquivo de log é obrigatório",
      "argsRequired": "Os argumentos são obrigatórios",
      "targetUrlRequired": "A URL de destino é obrigatória",
      "invalidSessionResponse": "Resposta inválida da sessão de pré-visualização",
      "newSessionIdRequired": "O novo ID da sessão é obrigatório"
    },
    "content": {
      "cursorRequiresRecovery": "O cursor do conteúdo precisa ser recuperado",
      "connectionClosedBeforeCursor": "A conexão do conteúdo foi encerrada antes do cursor final",
      "bufferIdentityChanged": "Um buffer de conteúdo não pode mudar a identidade do recurso",
      "mixedResourceEpochs": "Épocas de recursos misturadas",
      "mixedContentEpochs": "Épocas de conteúdo misturadas",
      "streamingUnavailable": "O fluxo de conteúdo não está disponível",
      "backendChangedDuringRevision": "O backend mudou durante a verificação da revisão",
      "missingResourceRevision": "Revisão do recurso ausente",
      "backendOrAuthChanged": "O backend ou a autenticação mudou durante a leitura",
      "notModifiedWithoutCache": "Uma resposta 304 não tem representação em cache",
      "generationChangedDuringCopy": "A geração do conteúdo mudou durante a cópia"
    },
    "plugins": {
      "commandResultMissing": "O servidor não retornou o resultado do comando do plugin."
    },
    "stream": {
      "sseRequired": "É necessário usar fluxo SSE",
      "invalidEventJson": "O evento SSE contém JSON inválido; é necessário reconciliar o snapshot",
      "connectionClosed": "A conexão SSE foi encerrada",
      "unsupportedEnvironment": "SSE requer fetch + TextDecoder"
    },
    "task": {
      "backendChanged": "O backend ou a autenticação mudou enquanto a tarefa era aguardada"
    },
    "document": {
      "invalidTable": "Tabela de documento inválida",
      "invalidProgressCounts": "Contagens de progresso inválidas",
      "byteBudgetExhausted": "Limite de bytes do documento esgotado",
      "invalidProgressIdentity": "Identidade de progresso inválida"
    },
    "attachments": {
      "memoryBudgetExceeded": "Limite de memória para anexos locais excedido"
    },
    "settings": {
      "batchOmittedPath": "O lote de configurações omitiu um caminho solicitado",
      "providerAndModelRequired": "É necessário informar um provedor e um modelo."
    },
    "markdown": {
      "workerFailed": "Falha no processo de Markdown",
      "mermaidEmptyOutput": "A renderização Mermaid retornou uma saída vazia"
    },
    "files": {
      "paginationStalled": "A paginação do diretório não avançou"
    },
    "resource": {
      "requestTimedOut": "A solicitação expirou"
    },
    "storage": {
      "transactionFailed": "Falha na transação do IndexedDB",
      "transactionAborted": "Transação do IndexedDB abortada",
      "requestFailed": "Falha na solicitação do IndexedDB",
      "notAvailable": "IndexedDB indisponível",
      "openFailed": "Falha ao abrir o IndexedDB"
    }
  },
  "command-activities-summary": "Abrir o painel de atividades em segundo plano",
  "command-attach-summary": "Anexar um arquivo usando o seletor interativo",
  "command-background-summary": "Voltar ao painel de sessões; a sessão continuará em execução",
  "command-children-summary": "Abrir o seletor de sessões filhas",
  "command-commands-summary": "Abrir a paleta de comandos",
  "command-commit-summary": "Criar um commit com as alterações preparadas",
  "command-compact-summary": "Compactar o contexto da sessão atual em um resumo",
  "command-continue-summary": "Continuar a sessão atual que está bloqueada ou pendente",
  "command-copy-message-summary": "Copiar a última mensagem do assistente",
  "command-copy-summary": "Copiar a transcrição carregada",
  "command-copy-visible-summary": "Copiar a área visível da transcrição",
  "command-diagnostics-summary": "Exibir um resumo de diagnóstico sanitizado para enviar como feedback",
  "command-editor-summary": "Abrir o editor externo do campo de mensagem",
  "command-export-summary": "Exportar a transcrição carregada em Markdown e abri-la no editor",
  "command-favorite-summary": "Adicionar ou remover dos favoritos a sessão atual ou selecionada",
  "command-fork-summary": "Abrir uma ramificação da sessão atual com todo o histórico",
  "command-help-summary": "Exibir a ajuda da interface atual",
  "command-hub-summary": "Abrir o painel de sessões",
  "command-image-summary": "Anexar uma imagem usando o seletor interativo",
  "command-lineage-summary": "Abrir o seletor do histórico de ramificações da sessão atual",
  "command-model-summary": "Abrir o seletor de modelo da sessão para escolher provedor/adaptador/modelo",
  "command-new-summary": "Criar uma nova sessão",
  "command-pager-summary": "Abrir a transcrição carregada em um paginador do Terminal",
  "command-parent-summary": "Ir para a sessão pai",
  "command-plan-summary": "Abrir o visualizador do plano da sessão atual",
  "command-pr-summary": "Criar um pull request com opções mínimas usando o gh",
  "command-rename-summary": "Renomear a sessão atual ou selecionada",
  "command-rewind-summary": "Retirar uma mensagem do usuário e todas as posteriores, criando uma nova ramificação",
  "command-sessions-summary": "Abrir o seletor de sessões",
  "command-settings-summary": "Abrir o painel unificado de configurações de modelos, permissões, plugins, runtime, sessões, interface e diagnósticos",
  "command-btw-summary": "Fazer uma pergunta à parte enquanto a conversa principal continua. Ferramentas somente de leitura; sem alterações na conversa principal.",
  "command-side-summary": "Abrir uma conversa paralela derivada sem interromper a execução atual",
  "command-status-summary": "Exibir o status atual da sessão, execução, espaço de trabalho e interface",
  "command-timeline-summary": "Abrir uma linha do tempo pesquisável de eventos da sessão atual ou selecionada",
  "command-usage-summary": "Abrir análises interativas de tokens e custos",
  interactive: {
    run: 'Executar',
    stop: 'Parar',
    starting: 'Iniciando…',
    reload: 'Recarregar',
    source: 'Código',
    logs: 'Logs',
    openPreview: 'Abrir prévia',
    fullscreen: 'Tela cheia',
    services: '{count} serviços',
    running: 'Executando',
    stopped: 'Parado',
    noLogs: 'Sem logs',
    loading: 'Carregando…',
    missingSession: 'Este aplicativo precisa de uma conversa.',
    chart: 'Gráfico',
    data: 'Tabela de dados',
    label: 'Rótulo',
    download: 'Baixar SVG',
  },
} as const;
