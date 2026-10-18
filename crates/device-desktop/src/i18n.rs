//! Static application copy only. Names and accessibility content from devices stay unchanged.
use crate::settings::LanguagePreference;
use std::cell::Cell;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Portuguese,
    English,
    Spanish,
}
thread_local! { static LANGUAGE: Cell<Language> = const { Cell::new(Language::English) }; }
pub fn current_language() -> Language {
    LANGUAGE.with(Cell::get)
}
pub fn apply_language(preference: LanguagePreference) {
    let language = match preference {
        LanguagePreference::Portuguese => Language::Portuguese,
        LanguagePreference::English => Language::English,
        LanguagePreference::Spanish => Language::Spanish,
        LanguagePreference::System => system_language(),
    };
    LANGUAGE.with(|current| current.set(language));
}
fn system_language() -> Language {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        // Windows LANGID stores the primary display-language identifier in 10 bits.
        let primary = unsafe { GetUserDefaultUILanguage() } & 0x03ff;
        if primary == 0x16 {
            return Language::Portuguese;
        }
        if primary == 0x0a {
            return Language::Spanish;
        }
    }
    Language::English
}
pub fn tr(source: &str) -> String {
    let language = current_language();
    if language == Language::Portuguese {
        return source.to_owned();
    }
    let translated = match source {
        "Conectando ao iPhone" => ("Connecting to iPhone", "Conectando al iPhone"),
        "Reconectando ao iPhone" => ("Reconnecting to iPhone", "Reconectando al iPhone"),
        "Transmissão pausada" => ("Stream paused", "Transmisión pausada"),
        "Transmissão indisponível" => ("Stream unavailable", "Transmisión no disponible"),
        "A imagem volta automaticamente. Seus controles continuam no lugar." => ("Video resumes automatically. Your controls stay in place.", "El vídeo vuelve automáticamente. Los controles permanecen en su sitio."),
        "Use Retomar para abrir a transmissão." => ("Use Resume to open the stream.", "Usa Reanudar para abrir la transmisión."),
        "Teclado do computador · clique na tela para digitar" => ("Computer keyboard · click the screen to type", "Teclado del ordenador · haz clic en la pantalla para escribir"),
        "Alternar teclado do computador" => ("Toggle computer keyboard", "Alternar teclado del ordenador"),
        "Caractere não suportado. Use o teclado do iPhone para esse texto." => ("Unsupported character. Use the iPhone keyboard for this text.", "Carácter no compatible. Usa el teclado del iPhone para este texto."),
        "Entrada pausada: digitação rápida demais. Confira o texto antes de continuar." => ("Input paused: typing is too fast. Check the text before continuing.", "Entrada pausada: escritura demasiado rápida. Revisa el texto antes de continuar."),
        "iPhone de demonstração" => ("Demo iPhone", "iPhone de demostración"),
        "Oculto na apresentação" => ("Hidden in presentation", "Oculto en presentación"),
        "Apresentação · dados do aparelho ocultos" => (
            "Presentation · device details hidden",
            "Presentación · datos del dispositivo ocultos",
        ),
        "Use AX para inspecionar sem runner ou WDA assinado para ativar elementos. Vídeo e toque têm uma conexão independente." => {
            (
                "Use AX to inspect without a runner, or signed WDA to activate elements. Video and touch use an independent connection.",
                "Usa AX para inspeccionar sin runner o WDA firmado para activar elementos. Vídeo y contacto tienen una conexión independiente.",
            )
        }
        "Conectar" => ("Connect", "Conectar"),
        "Elementos" => ("Elements", "Elementos"),
        "Diagnóstico" => ("Diagnostics", "Diagnóstico"),
        "Integrações" => ("Integrations", "Integraciones"),
        "Configurações" => ("Settings", "Configuración"),
        "WORKSPACE" => ("WORKSPACE", "ESPACIO DE TRABAJO"),
        "FERRAMENTAS" => ("TOOLS", "HERRAMIENTAS"),
        "SESSÃO" => ("SESSION", "SESIÓN"),
        "Conectado" => ("Connected", "Conectado"),
        "Sem sessão" => ("No session", "Sin sesión"),
        "Abrir tela" => ("Open screen", "Abrir pantalla"),
        "Abrir sessão" => ("Open session", "Abrir sesión"),
        "Desconectar" => ("Disconnect", "Desconectar"),
        "Conexão ativa." => ("Active connection.", "Conexión activa."),
        "Conecte seu iPhone." => ("Connect your iPhone.", "Conecta tu iPhone."),
        "Acesse a tela remota e as ferramentas deste aparelho." => (
            "Access the remote screen and tools for this device.",
            "Accede a la pantalla remota y las herramientas de este dispositivo.",
        ),
        "Aparelho conectado" => ("Connected device", "Dispositivo conectado"),
        "Ferramentas da sessão" => ("Session tools", "Herramientas de la sesión"),
        "Inspecione a interface ou consulte a saúde da conexão. A tela permanece em sua própria janela." => {
            (
                "Inspect the interface or check connection health. The screen stays in its own window.",
                "Inspecciona la interfaz o revisa el estado de la conexión. La pantalla permanece en su propia ventana.",
            )
        }
        "Inspecionar elementos" => ("Inspect elements", "Inspeccionar elementos"),
        "Diagnóstico da conexão" => ("Connection diagnostics", "Diagnóstico de conexión"),
        "Escolha o transporte, encontre o aparelho e abra uma sessão local." => (
            "Choose a transport, find the device and open a local session.",
            "Elige el transporte, busca el dispositivo y abre una sesión local.",
        ),
        "Túnel RSD" => ("RSD tunnel", "Túnel RSD"),
        "Wi-Fi direto" => ("Direct Wi-Fi", "Wi-Fi directo"),
        "Conexão" => ("Connection", "Conexión"),
        "Conecte o cabo de dados e desbloqueie o iPhone. Confirme a confiança no aparelho se solicitada." => {
            (
                "Connect a data cable and unlock the iPhone. Confirm trust on the device if prompted.",
                "Conecta un cable de datos y desbloquea el iPhone. Confirma la confianza en el dispositivo si se solicita.",
            )
        }
        "Mesma rede do computador, iPhone desbloqueado e previamente pareado. O cabo não mantém esta conexão." => {
            (
                "Use the same network as the computer and an unlocked, previously paired iPhone. This connection does not depend on the cable.",
                "Usa la misma red del ordenador y un iPhone desbloqueado y previamente emparejado. Esta conexión no depende del cable.",
            )
        }
        "Use o serviço de túneis já ativo neste computador e o UDID do aparelho." => (
            "Use the tunnel service already running on this computer and the device UDID.",
            "Usa el servicio de túneles activo en este ordenador y el UDID del dispositivo.",
        ),
        "UDID do aparelho" => ("Device UDID", "UDID del dispositivo"),
        "UDID do iPhone pareado" => ("Paired iPhone UDID", "UDID del iPhone emparejado"),
        "IP do iPhone (opcional)" => ("iPhone IP (optional)", "IP del iPhone (opcional)"),
        "Descoberta automática" => ("Automatic discovery", "Detección automática"),
        "Serviço de túneis local" => ("Local tunnel service", "Servicio de túneles local"),
        "Buscar aparelho" => ("Find device", "Buscar dispositivo"),
        "Buscando aparelho…" => ("Finding device…", "Buscando dispositivo…"),
        "Opções da sessão" => ("Session options", "Opciones de sesión"),
        "A tela ao vivo usa o vídeo nativo do aparelho. Elementos precisam de um WebDriverAgent assinado para sua equipe." => {
            (
                "Live view uses the device's native video. Elements require a WebDriverAgent signed for your team.",
                "La vista en vivo usa el vídeo nativo del dispositivo. Los elementos requieren un WebDriverAgent firmado para tu equipo.",
            )
        }
        "Ocultar opções avançadas" => ("Hide advanced options", "Ocultar opciones avanzadas"),
        "Configurar automação e recuperação" => (
            "Configure automation and recovery",
            "Configurar automatización y recuperación",
        ),
        "Bundle ID do WebDriverAgent" => {
            ("WebDriverAgent bundle ID", "Bundle ID de WebDriverAgent")
        }
        "Permitir recuperação USB" => ("Allow USB recovery", "Permitir recuperación USB"),
        "Permitir reinício do serviço Apple" => (
            "Allow restarting the Apple service",
            "Permitir reiniciar el servicio Apple",
        ),
        "Recuperações exigem privilégios do sistema e são solicitadas manualmente. Não repetem ações no iPhone." => {
            (
                "Recovery requires system privileges and is requested manually. It does not replay iPhone actions.",
                "La recuperación requiere privilegios del sistema y se solicita manualmente. No repite acciones en el iPhone.",
            )
        }
        "Preparar Wi-Fi pelo USB pareado" => (
            "Prepare Wi-Fi over paired USB",
            "Preparar Wi-Fi por USB emparejado",
        ),
        "Aparelhos" => ("Devices", "Dispositivos"),
        "Nenhum aparelho disponível" => {
            ("No devices available", "No hay dispositivos disponibles")
        }
        "Desbloqueie o iPhone e faça uma busca no transporte selecionado. O diagnóstico detalha cada camada da conexão." => {
            (
                "Unlock the iPhone and search using the selected transport. Diagnostics details each connection layer.",
                "Desbloquea el iPhone y busca con el transporte seleccionado. El diagnóstico detalla cada capa de la conexión.",
            )
        }
        "Identidade verificada" => ("Identity verified", "Identidad verificada"),
        "Verifique o diagnóstico" => ("Check diagnostics", "Consulta el diagnóstico"),
        "Nenhum aparelho ativo" => ("No active device", "Ningún dispositivo activo"),
        "Modelo não informado" => ("Model unavailable", "Modelo no disponible"),
        "Elementos acessíveis." => ("Accessible elements.", "Elementos accesibles."),
        "Encontre controles por nome, identificador ou tipo. O servidor revalida a referência antes de ativar." => {
            (
                "Find controls by name, identifier or type. The server revalidates the reference before activation.",
                "Busca controles por nombre, identificador o tipo. El servidor vuelve a validar la referencia antes de activarla.",
            )
        }
        "Consultar elementos" => ("Inspect elements", "Consultar elementos"),
        "Filtrar elementos" => ("Filter elements", "Filtrar elementos"),
        "Filtrar nome, identificador ou tipo" => (
            "Filter by name, identifier or type",
            "Filtrar por nombre, identificador o tipo",
        ),
        "Elementos WDA indisponíveis no Wi-Fi direto" => (
            "WDA elements unavailable on direct Wi-Fi",
            "Elementos WDA no disponibles en Wi-Fi directo",
        ),
        "Esta sessão oferece vídeo e gestos. Para elementos, use um transporte Apple/usbmux com runner assinado disponível." => {
            (
                "This session offers video and gestures. For elements, use an Apple/usbmux transport with a signed runner available.",
                "Esta sesión ofrece vídeo y gestos. Para elementos, usa un transporte Apple/usbmux con un runner firmado disponible.",
            )
        }
        "Inspecione a tela atual" => (
            "Inspect the current screen",
            "Inspecciona la pantalla actual",
        ),
        "Abra uma sessão e consulte os elementos. AX pode oferecer somente leitura; a interface não simula uma ativação." => {
            (
                "Open a session and inspect elements. AX may be read-only; the interface does not simulate activation.",
                "Abre una sesión y consulta los elementos. AX puede ser de solo lectura; la interfaz no simula una activación.",
            )
        }
        "Elemento sem nome" => ("Unnamed element", "Elemento sin nombre"),
        "Copiar seletor" => ("Copy selector", "Copiar selector"),
        "Ativar elemento" => ("Activate element", "Activar elemento"),
        "Saúde da conexão." => ("Connection health.", "Estado de la conexión."),
        "Compare hardware, serviço, identidade e automação antes de escolher uma recuperação." => {
            (
                "Compare hardware, service, identity and automation before choosing a recovery action.",
                "Compara hardware, servicio, identidad y automatización antes de elegir una recuperación.",
            )
        }
        "Atualizar diagnóstico" => ("Refresh diagnostics", "Actualizar diagnóstico"),
        "Reiniciar serviço Apple" => ("Restart Apple service", "Reiniciar servicio Apple"),
        "Reiniciar automação" => ("Restart automation", "Reiniciar automatización"),
        "Diagnóstico ainda não disponível" => (
            "Diagnostics not yet available",
            "Diagnóstico aún no disponible",
        ),
        "Atualize o relatório para consultar o estado real deste computador e do aparelho." => (
            "Refresh the report to see the actual status of this computer and device.",
            "Actualiza el informe para ver el estado real de este ordenador y del dispositivo.",
        ),
        "Resumo" => ("Summary", "Resumen"),
        "Conexão pronta" => ("Connection ready", "Conexión lista"),
        "Requer atenção" => ("Needs attention", "Requiere atención"),
        "Verificar conexão" => ("Check connection", "Revisar conexión"),
        "Camadas verificadas" => ("Checked layers", "Capas verificadas"),
        "Histórico do transporte" => ("Transport history", "Historial de transporte"),
        "Instantes observados por consulta; não comprovam o momento físico de uma desconexão nem sua causa." => {
            (
                "Observations are sampled; they do not prove the exact time or cause of a physical disconnection.",
                "Las observaciones son muestreos; no demuestran el momento exacto ni la causa de una desconexión física.",
            )
        }
        "USB e interfaces Apple" => ("USB and Apple interfaces", "USB e interfaces Apple"),
        "Nenhum dispositivo USB retornado pelo inventário atual." => (
            "The current inventory returned no USB devices.",
            "El inventario actual no devolvió dispositivos USB.",
        ),
        "Dispositivo USB" => ("USB device", "Dispositivo USB"),
        "Conecte suas ferramentas." => ("Connect your tools.", "Conecta tus herramientas."),
        "MCP para agentes, servidor local para clientes e assinatura própria para elementos." => (
            "MCP for agents, a local server for clients and your own signing for elements.",
            "MCP para agentes, servidor local para clientes y firma propia para elementos.",
        ),
        "Automação com IA" => ("AI automation", "Automatización con IA"),
        "Conecte seu cliente MCP à sessão local. Consulte o estado antes e depois de cada ação." => {
            (
                "Connect your MCP client to the local session. Observe state before and after each action.",
                "Conecta tu cliente MCP a la sesión local. Consulta el estado antes y después de cada acción.",
            )
        }
        "Copiar configuração MCP" => ("Copy MCP configuration", "Copiar configuración MCP"),
        "Inclui o token temporário desta sessão. Cole somente no cliente de IA em que você confia." => {
            (
                "Includes this session's temporary token. Paste it only into an AI client you trust.",
                "Incluye el token temporal de esta sesión. Pégalo solo en un cliente de IA de confianza.",
            )
        }
        "Servidor existente" => ("Existing server", "Servidor existente"),
        "Endereço de loopback ou encaminhamento SSH. Token mantido somente em memória." => (
            "Loopback address or SSH forwarding. The token is kept in memory only.",
            "Dirección de loopback o reenvío SSH. El token se mantiene solo en memoria.",
        ),
        "Endereço da API" => ("API address", "Dirección de la API"),
        "Token do servidor" => ("Server token", "Token del servidor"),
        "Conectar ao servidor" => ("Connect to server", "Conectar al servidor"),
        "Assinatura do iOS" => ("iOS signing", "Firma de iOS"),
        "O aplicativo Windows não precisa de certificado Apple. Cada usuário ou equipe prepara um runner WDA assinado e um perfil que autorize seus próprios aparelhos." => {
            (
                "The Windows app needs no Apple certificate. Each user or team prepares a signed WDA runner and a profile authorizing their own devices.",
                "La aplicación Windows no necesita un certificado Apple. Cada usuario o equipo prepara un runner WDA firmado y un perfil que autorice sus dispositivos.",
            )
        }
        "O pacote da comunidade não inclui chave privada nem runner pessoal. Informe o Bundle ID em Conectar → Opções da sessão." => {
            (
                "The community package contains no private key or personal runner. Enter the bundle ID in Connect → Session options.",
                "El paquete de la comunidad no incluye claves privadas ni runners personales. Introduce el Bundle ID en Conectar → Opciones de sesión.",
            )
        }
        "Guia oficial de perfis Apple" => (
            "Official Apple profiles guide",
            "Guía oficial de perfiles Apple",
        ),
        "Atividade" => ("Activity", "Actividad"),
        "Open source · MIT" => ("Open source · MIT", "Código abierto · MIT"),
        "Alternar sidebar · Ctrl+B" => {
            ("Toggle sidebar · Ctrl+B", "Alternar barra lateral · Ctrl+B")
        }
        "Runner de automação" => ("Automation runner", "Runner de automatización"),
        "Importe um .ipa já assinado para este iPhone. A instalação usa o USB e depende de o iOS aceitar a assinatura." => {
            (
                "Import an IPA already signed for this iPhone. Installation uses USB and depends on iOS accepting the signature.",
                "Importa un IPA ya firmado para este iPhone. La instalación usa USB y depende de que iOS acepte la firma.",
            )
        }
        "Selecionar .ipa assinado" => ("Choose signed IPA", "Seleccionar IPA firmado"),
        "Selecionar runner iOS assinado (.ipa)" => (
            "Choose signed iOS runner (.ipa)",
            "Seleccionar runner iOS firmado (.ipa)",
        ),
        "Instalar no iPhone por USB" => (
            "Install on iPhone over USB",
            "Instalar en el iPhone por USB",
        ),
        "Metadados conferidos; o iOS confirma a assinatura na instalação." => (
            "Metadata checked; iOS confirms the signature during installation.",
            "Metadatos revisados; iOS confirma la firma durante la instalación.",
        ),
        "Runner instalado. Reconecte a sessão para ativar a automação." => (
            "Runner installed. Reconnect the session to enable automation.",
            "Runner instalado. Vuelve a conectar la sesión para activar la automatización.",
        ),
        "Informe o UDID do iPhone antes de importar o runner." => (
            "Enter the iPhone UDID before importing a runner.",
            "Introduce el UDID del iPhone antes de importar un runner.",
        ),
        "Aparência" => ("Appearance", "Apariencia"),
        "Personalize o aplicativo." => ("Make the app yours.", "Personaliza la aplicación."),
        "Tema, idioma e movimento são aplicados imediatamente e salvos neste computador." => (
            "Theme, language and motion apply immediately and are saved on this computer.",
            "El tema, idioma y movimiento se aplican inmediatamente y se guardan en este ordenador.",
        ),
        "Tema" => ("Theme", "Tema"),
        "Sistema" => ("System", "Sistema"),
        "Escuro" => ("Dark", "Oscuro"),
        "Claro" => ("Light", "Claro"),
        "Idioma" => ("Language", "Idioma"),
        "Usar o idioma do Windows" => ("Use the Windows language", "Usar el idioma de Windows"),
        "Movimento" => ("Motion", "Movimiento"),
        "Reduzir animações" => ("Reduce animations", "Reducir animaciones"),
        "Mantém transições discretas sem animações contínuas." => (
            "Keeps transitions subtle without continuous animations.",
            "Mantiene transiciones discretas sin animaciones continuas.",
        ),
        "Descoberta de aparelhos" => ("Device discovery", "Detección de dispositivos"),
        "Buscar automaticamente" => ("Find devices automatically", "Buscar automáticamente"),
        "Busca somente enquanto Conectar está visível e sem sessão. Operações em andamento pausam novas buscas; falhas aumentam o intervalo." => {
            (
                "Searches only while Connect is visible without a session. Active operations pause new searches; failures increase the interval.",
                "Busca solo mientras Conectar está visible y sin sesión. Las operaciones activas pausan nuevas búsquedas; los fallos aumentan el intervalo.",
            )
        }
        "Preferências salvas neste computador." => (
            "Preferences saved on this computer.",
            "Preferencias guardadas en este ordenador.",
        ),
        "Não foi possível salvar as preferências." => (
            "Could not save preferences.",
            "No se pudieron guardar las preferencias.",
        ),
        "Busca automática ativa" => ("Automatic discovery on", "Detección automática activa"),
        "Busca automática pausada" => {
            ("Automatic discovery paused", "Detección automática pausada")
        }
        "Minimizar" => ("Minimize", "Minimizar"),
        "Maximizar ou restaurar" => ("Maximize or restore", "Maximizar o restaurar"),
        "Fechar" => ("Close", "Cerrar"),
        "Provedor de elementos" => ("Element provider", "Proveedor de elementos"),
        "Automático" => ("Automatic", "Automático"),
        "Somente leitura" => ("Read-only", "Solo lectura"),
        "Pausar transmissão" => ("Pause stream", "Pausar transmisión"),
        "Retomar transmissão" => ("Resume stream", "Reanudar transmisión"),
        "Estatísticas da transmissão" => ("Stream statistics", "Estadísticas de transmisión"),
        "Controle ativo: mudar para somente visualizar" => (
            "Control active: switch to view only",
            "Control activo: cambiar a solo visualización",
        ),
        "Somente visualizar: ativar controle do iPhone" => (
            "View only: enable iPhone control",
            "Solo visualización: activar control del iPhone",
        ),
        "Controle temporariamente suspenso: mudar para somente visualizar" => (
            "Control temporarily suspended: switch to view only",
            "Control suspendido temporalmente: cambiar a solo visualización",
        ),
        "Minimizar janela" => ("Minimize window", "Minimizar ventana"),
        "Maximizar janela" => ("Maximize window", "Maximizar ventana"),
        "Restaurar tamanho da janela" => ("Restore window size", "Restaurar tamaño de ventana"),
        "Fechar tela remota" => ("Close remote screen", "Cerrar pantalla remota"),
        "Ir à tela inicial do iPhone" => (
            "Go to the iPhone Home screen",
            "Ir a la pantalla de inicio del iPhone",
        ),
        "Abrir alternador de aplicativos do iPhone" => (
            "Open the iPhone app switcher",
            "Abrir selector de aplicaciones del iPhone",
        ),
        "Aumentar volume do iPhone" => ("Increase iPhone volume", "Subir volumen del iPhone"),
        "Diminuir volume do iPhone" => ("Decrease iPhone volume", "Bajar volumen del iPhone"),
        "Bloquear a tela do iPhone" => ("Lock iPhone screen", "Bloquear pantalla del iPhone"),
        "Copiar quadro exibido" => ("Copy displayed frame", "Copiar fotograma mostrado"),
        "Transmissão" => ("Stream", "Transmisión"),
        "Pausado" => ("Paused", "Pausado"),
        "Transmissão encerrada" => ("Stream ended", "Transmisión finalizada"),
        "Aguardando vídeo" => ("Waiting for video", "Esperando vídeo"),
        "Reconectando automaticamente…" => (
            "Reconnecting automatically…",
            "Reconectando automáticamente…",
        ),
        "Reconectando…" => ("Reconnecting…", "Reconectando…"),
        "Pintura GPUI" => ("GPUI painting", "Pintado GPUI"),
        "Intervalo médio" => ("Average interval", "Intervalo medio"),
        "Idade local do quadro" => ("Local frame age", "Edad local del fotograma"),
        "Resposta do comando" => ("Command response", "Respuesta del comando"),
        "Tela de origem" => ("Source screen", "Pantalla de origen"),
        "Preview" => ("Preview", "Vista previa"),
        "Recuperações" => ("Recoveries", "Recuperaciones"),
        "Não confirmada" => ("Not confirmed", "No confirmada"),
        "Abrindo um novo fluxo…" => ("Opening a new stream…", "Abriendo una nueva transmisión…"),
        "Última interrupção" => ("Last interruption", "Última interrupción"),
        "Ao vivo" => ("Live", "En vivo"),
        "tentativas" => ("attempts", "intentos"),
        "Espera programada" => ("Scheduled delay", "Espera programada"),
        "Arrastar janela da tela remota" => (
            "Drag remote screen window",
            "Arrastrar ventana de pantalla remota",
        ),
        "Token de autenticação" => ("Authentication token", "Token de autenticación"),
        "Configuração MCP copiada por solicitação. Ela contém o token da sessão." => (
            "MCP configuration copied as requested. It contains the session token.",
            "Configuración MCP copiada a petición. Contiene el token de sesión.",
        ),
        "Perfil: expira em" => ("Profile expires", "El perfil vence"),
        "Equipe" => ("Team", "Equipo"),
        "iPhone autorizado" => ("Authorized iPhone", "iPhone autorizado"),
        "Provedor" => ("Provider", "Proveedor"),
        "Referência expirada" => ("Reference expired", "Referencia caducada"),
        "Nenhum elemento corresponde ao filtro." => (
            "No elements match the filter.",
            "Ningún elemento coincide con el filtro.",
        ),
        "Não foi possível salvar as preferências" => (
            "Could not save preferences",
            "No se pudieron guardar las preferencias",
        ),
        "Bloquear ou acender a tela do iPhone" => (
            "Lock or wake the iPhone screen",
            "Bloquear o encender la pantalla del iPhone",
        ),
        "Tela remota do iPhone" => ("iPhone remote screen", "Pantalla remota del iPhone"),
        "Pausar transmissão da tela" => ("Pause screen stream", "Pausar transmisión de pantalla"),
        "Retomar transmissão da tela" => {
            ("Resume screen stream", "Reanudar transmisión de pantalla")
        }
        "Mostrar estatísticas da transmissão" => (
            "Show stream statistics",
            "Mostrar estadísticas de transmisión",
        ),
        "Ocultar estatísticas da transmissão" => (
            "Hide stream statistics",
            "Ocultar estadísticas de transmisión",
        ),
        "Copiar captura da tela do iPhone" => (
            "Copy iPhone screen capture",
            "Copiar captura de pantalla del iPhone",
        ),
        "Minimizar janela da tela remota" => (
            "Minimize remote screen window",
            "Minimizar ventana de pantalla remota",
        ),
        "Fechar janela da tela remota" => (
            "Close remote screen window",
            "Cerrar ventana de pantalla remota",
        ),
        "Clique para tocar. Mantenha o mouse pressionado e mova para arrastar ao vivo; solte para levantar o dedo." => {
            (
                "Click to tap. Hold and move the mouse to drag live; release to lift your finger.",
                "Haz clic para tocar. Mantén pulsado y mueve el ratón para arrastrar en vivo; suelta para levantar el dedo.",
            )
        }
        "FPS de quadros distintos pintados pelo GPUI. Intervalo = 1000/FPS. Resposta é da API; não mede latência ponta a ponta." => {
            (
                "FPS counts distinct frames painted by GPUI. Interval = 1000/FPS. Response is API time, not end-to-end latency.",
                "FPS cuenta fotogramas distintos pintados por GPUI. Intervalo = 1000/FPS. Respuesta es tiempo de API, no latencia de extremo a extremo.",
            )
        }
        "Controle suspenso enquanto o vídeo atualiza. Retomará quando a imagem estiver recente." => {
            (
                "Control is suspended while video updates. It resumes when the image is fresh.",
                "El control está suspendido mientras se actualiza el vídeo. Se reanudará cuando la imagen sea reciente.",
            )
        }
        "Sem nome acessível" => ("No accessible name", "Sin nombre accesible"),
        "Sem identificador" => ("No identifier", "Sin identificador"),
        "Tipo desconhecido" => ("Unknown type", "Tipo desconocido"),
        "Ativação disponível" => ("Activation available", "Activación disponible"),
        "consulta parcial" => ("partial snapshot", "consulta parcial"),
        "visível" => ("visible", "visible"),
        "habilitado" => ("enabled", "habilitado"),
        "elementos" => ("elements", "elementos"),
        "de" => ("of", "de"),
        "consultas" => ("queries", "consultas"),
        "registros recentes" => ("recent records", "registros recientes"),
        "há" => ("age", "hace"),
        "aparelho(s)" => ("device(s)", "dispositivo(s)"),
        "interface(s)" => ("interface(s)", "interfaz(es)"),
        "classe" => ("class", "clase"),
        "Não foi possível abrir a tela" => {
            ("Could not open the screen", "No se pudo abrir la pantalla")
        }
        "Não foi possível abrir o arquivo" => {
            ("Could not open the file", "No se pudo abrir el archivo")
        }
        "Não foi possível copiar o seletor" => (
            "Could not copy the selector",
            "No se pudo copiar el selector",
        ),
        "Verificando conexão…" => ("Checking connection…", "Verificando conexión…"),
        "Abrindo sessão…" => ("Opening session…", "Abriendo sesión…"),
        "Sessão encerrada" => ("Session ended", "Sesión finalizada"),
        "Inspecionando elementos…" => ("Inspecting elements…", "Inspeccionando elementos…"),
        "Ativando elemento…" => ("Activating element…", "Activando elemento…"),
        "Operação não concluída" => ("Operation not completed", "Operación no completada"),
        "Recuperando serviço Apple…" => {
            ("Recovering Apple service…", "Recuperando servicio Apple…")
        }
        "Recuperando automação…" => ("Recovering automation…", "Recuperando automatización…"),
        "Recuperando interface USB…" => {
            ("Recovering USB interface…", "Recuperando interfaz USB…")
        }
        "Preparando Wi-Fi pelo USB pareado…" => (
            "Preparing Wi-Fi over paired USB…",
            "Preparando Wi-Fi por USB emparejado…",
        ),
        "Conectando ao servidor…" => ("Connecting to server…", "Conectando al servidor…"),
        "Instalando runner no iPhone por USB…" => (
            "Installing runner on iPhone over USB…",
            "Instalando runner en el iPhone por USB…",
        ),
        "IPA preparado. Revise o perfil e o aparelho antes de instalar." => (
            "IPA prepared. Review the profile and device before installing.",
            "IPA preparado. Revisa el perfil y el dispositivo antes de instalar.",
        ),
        "Runner instalado por USB. Reconecte a sessão para ativar a automação." => (
            "Runner installed over USB. Reconnect the session to enable automation.",
            "Runner instalado por USB. Vuelve a conectar la sesión para activar la automatización.",
        ),
        "Runner preparado para Wi-Fi isolado" => (
            "Runner prepared for isolated Wi-Fi",
            "Runner preparado para Wi-Fi aislado",
        ),
        "Confirme somente um runner revisado que respeite USE_IP nos servidores HTTP e MJPEG. Nome do IPA não comprova isso; WDA 16.15.3+ é uma referência conhecida." => {
            (
                "Confirm only a reviewed runner that respects USE_IP on both HTTP and MJPEG servers. The IPA name is not evidence; WDA 16.15.3+ is a known reference.",
                "Confirma solo un runner revisado que respete USE_IP en los servidores HTTP y MJPEG. El nombre del IPA no lo demuestra; WDA 16.15.3+ es una referencia conocida.",
            )
        }
        "AX permite inspeção e seleção de referências. WDA permite ações por elemento com runner disponível." => {
            (
                "AX supports inspection and reference selection. WDA supports element actions when a runner is available.",
                "AX permite inspección y selección de referencias. WDA permite acciones por elemento cuando hay un runner disponible.",
            )
        }
        "consulta concluída" => ("snapshot complete", "consulta completada"),
        "Reiniciar esta interface" => ("Restart this interface", "Reiniciar esta interfaz"),
        "Não foi possível ler as preferências" => (
            "Could not read preferences",
            "No se pudieron leer las preferencias",
        ),
        "Arquivo de preferências inválido" => (
            "Invalid preferences file",
            "Archivo de preferencias no válido",
        ),
        "Expandir barra lateral · Ctrl+B" => {
            ("Expand sidebar · Ctrl+B", "Expandir barra lateral · Ctrl+B")
        }
        "Recolher barra lateral · Ctrl+B" => (
            "Collapse sidebar · Ctrl+B",
            "Contraer barra lateral · Ctrl+B",
        ),
        _ => return translate_status(source),
    };
    match language {
        Language::English => translated.0,
        Language::Spanish => translated.1,
        Language::Portuguese => source,
    }
    .to_owned()
}

fn translate_status(source: &str) -> String {
    if let Some(transport) = source.strip_prefix("Conectado · ") {
        return format!("{} · {}", tr("Conectado"), tr(transport));
    }
    for prefix in [
        "Diagnóstico",
        "Conectado",
        "Não foi possível abrir a tela",
        "Não foi possível abrir o arquivo",
        "Não foi possível copiar o seletor",
        "Não foi possível ler as preferências",
        "Arquivo de preferências inválido",
    ] {
        if let Some(suffix) = source.strip_prefix(prefix) {
            return format!("{}{}", tr(prefix), suffix);
        }
    }
    if let Some((count, provider)) = source.split_once(" elementos · ") {
        if count.chars().all(|character| character.is_ascii_digit()) {
            return format!("{count} {} · {provider}", tr("elementos"));
        }
    }
    source.to_owned()
}
