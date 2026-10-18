# Elementos: AX e WDA no Windows

A escolha de provedor é explícita na interface, na API, na CLI e no MCP. O
transporte de vídeo/HID não implica que o serviço de elementos esteja disponível.

| Provedor | Consulta | Hierarquia e limites | Ativação por elemento |
| --- | --- | --- | --- |
| `ios_ax` | AX nativo por CoreDevice/RSD autenticado, USB ou Wi-Fi pareado | Travessia plana; sem inferir papel, bounds ou hierarquia. Pode rolar o app. | Não; nenhuma ativação simulada por coordenadas |
| `wda` | XCTest/WebDriverAgent assinado e iniciado | Árvore acessível do XCTest; bounds em pontos, cobertura limitada ao snapshot | Sim; referência recente, app/PID e identidade de sessão conferidos |
| `auto` | Tenta WDA pronto; se indisponível usa AX | A resposta informa o provedor real e avisos | Somente se a resposta WDA declarar suporte |

AX não precisa de certificado ou runner WDA. Ainda exige pareamento, aparelho
desbloqueado, Developer Mode e serviços de desenvolvimento/DDI compatíveis com o
iOS. Não é OCR e não garante a presença de todos os controles visuais.

## Wi-Fi WDA isolado

O caminho implementado usa o registro RemotePairing já existente, verifica a
identidade do aparelho e abre túnel TLS pareado. DVT e testmanagerd usam esse stack
RSD. Os metadados do runner são consultados pelo serviço
`com.apple.mobile.installation_proxy.shim.remote`, com check-in RSD no mesmo
túnel. Quando esse stack é fornecido explicitamente, uma falha no serviço DVT não
habilita fallback para fora do túnel: a inicialização falha de forma explícita.
O HTTP do WDA também usa o AdapterHandle do túnel interno; não há acesso
a `http://IP-do-iPhone:8100`, bind no IP Wi-Fi ou publicação de proxy WDA na LAN.
O daemon TetherDeck continua em loopback, com token e escopo de aparelho.

O caminho legado WDA por usbmux fica restrito a USB. Uma entrada Network no
usbmux não é aceita como prova de canal HTTP autenticado; use Wi-Fi direto
pareado. A inspeção AX continua independente do runner.

A inicialização configura `USE_IP` com o IPv6 ULA do **interior do túnel**.
Isso exige um runner compatível que respeite esse endereço em **ambos** os
servidores HTTP e MJPEG. WDA 16.15.3 contém a correção de binding MJPEG; versões
anteriores podem ignorar o endereço ou deixar um dos servidores em outras
interfaces. Uma versão futura/customizada também precisa de revisão de origem.

Por isso, o padrão bloqueia o lançamento WDA Wi-Fi. A opção **Runner preparado
para Wi-Fi isolado** registra uma revisão do Bundle ID exato informado. Alterar
esse identificador ou importar outro IPA invalida a revisão. A CLI do daemon
exige `--wda-runner <bundle> --wda-tunnel-binding-reviewed`. A declaração vale
somente para esse runner selecionado; não é atestação criptográfica do IPA nem
prova automática de isolamento de um binário arbitrário. Não marque com base
somente no nome, `Info.plist`, presença da string `USE_IP` ou número da versão.

Sem revisão, AX continua disponível e o diagnóstico informa
`wda_wifi_binding_review_required`. Sem um runner iniciado por esta sessão, a
seleção WDA responde indisponível; ela não tenta HTTP desautenticado na rede.
A recuperação do runner é explícita, mantém o cooldown e não repete ativações.
Ao encerrar a sessão, o host aborta a tarefa DTX e solta seus recursos; isso não
comprova que o processo WDA terminou no iPhone. O app pode permanecer aberto.
Uma recuperação posterior tenta parar somente o executável do runner configurado
antes de iniciar outra execução; não encerra outros apps.
A compatibilidade desta implementação Wi-Fi com cada iOS/runner precisa ser
confirmada no aparelho; compilar não comprova execução XCTest ou isolamento.

O registro Lockdown sozinho não substitui o registro RemotePairing. Preparar a
rede e instalar/assinar um runner são operações distintas. A instalação de IPA
continua pelo fluxo USB existente. Cada usuário utiliza a própria assinatura.

## API e automação

- `GET /v1/devices/{id}/ui?provider=ios_ax` (ou `wda`, `auto`).
- `POST /v1/devices/{id}/ui/find` recebe `provider` junto de `selector` e `timeout_ms`.
- Sem esse campo, a API mantém `auto` para compatibilidade. O modo global
  `--require-wda` continua recusando leitura AX explícita e fallback AX.
- CLI: `rust-ios-device ui <id> --provider ios_ax` e
  `rust-ios-device find-element <id> --provider wda --identifier <identifier>`.
- MCP: ferramentas `ui_snapshot` e `find_element` expõem `provider` tipado.

Trocar provedor descarta a sessão de elementos e referências anteriores. A
ativação consome a referência antes do envio, exige snapshot de até 30 segundos,
confere app/PID, estado e unicidade do seletor. Resultado incerto não é repetido.
Após a submissão, observe uma consulta nova para conferir a mudança esperada.

## Fontes primárias da compatibilidade

- [Appium: WDA pré-instalado](https://github.com/appium/appium-xcuitest-driver/blob/master/docs/guides/run-preinstalled-wda.md): XCTest, assinatura e requisitos por versão.
- [WDA: binding HTTP e MJPEG](https://github.com/appium/WebDriverAgent/blob/master/WebDriverAgentLib/Routing/FBWebServer.m): `bindingIPAddress` aplicado à interface HTTP e ao socket MJPEG.
- [Correção MJPEG USE_IP](https://github.com/appium/WebDriverAgent/commit/53573cf3a74fe8db87cb5f38b2cdc7f17e856e4f): origem da exigência de revisão das duas interfaces.
- [idevice](https://github.com/jkcoxson/idevice): provider/RSD, XCTest e WDA usados como base nativa.
- [InstallationProxy remoto](https://idevice.wiki/docs/service-reference/files-apps/installation-proxy/): serviço shim usado para consultar os metadados do runner pelo túnel.
- [libusbmuxd iproxy](https://github.com/libimobiledevice/libusbmuxd/blob/master/tools/iproxy.c): transporte Network não é tratado como prova de túnel autenticado pela simples presença no usbmux.
