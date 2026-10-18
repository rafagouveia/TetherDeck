# Começar no Windows

[English](GETTING-STARTED.en.md) · [Español](GETTING-STARTED.es.md)

## Requisitos

- Windows x86_64 com ambiente gráfico compatível com DirectX 11.
- Drivers/serviço Apple para Windows e iPhone confiável, conectado e desbloqueado.
- FFmpeg externo com decodificador HEVC para a tela ao vivo.
- Para ativar elementos: runner WebDriverAgent XCTest compatível, assinado para sua equipe
  e aparelho. A inspeção AX não exige runner ou certificado WDA, mas exige pareamento
  e os serviços de desenvolvimento do iPhone.

Os programas do TetherDeck são escritos em Rust e não exigem interpretador Python.
Um túnel RSD administrado externamente pode ter requisitos próprios.

## Conectar por USB

1. Mantenha os executáveis do pacote juntos e abra **TetherDeck.exe**.
2. Conecte e desbloqueie o iPhone. Aceite a solicitação de confiança se aparecer.
3. Em **Conectar**, escolha USB, busque o aparelho e abra a sessão correspondente.
4. Use **Diagnóstico** se serviço Apple, usbmux, pareamento ou identidade falharem.

USB presente, serviço ativo e acesso pareado são camadas diferentes. Botões de
recuperação precisam da opção correspondente habilitada e de permissões do Windows.
Um resultado que exija reinicialização não reinicia o computador automaticamente.

## Conectar por Wi-Fi

Wi-Fi Apple/usbmux, Wi-Fi direto pareado e túnel RSD são transportes diferentes.
Prepare a confiança pelo USB quando necessário e mantenha computador e iPhone numa
rede local alcançável. Escolha o transporte e confira a descoberta/identidade antes
de abrir a sessão.

Se o caminho exigir registro de pareamento remoto, prepare-o pelo seu processo
confiável de pareamento. Não publique nem compartilhe esse registro. A descoberta
RSD consome somente um túnel local já ativo; não cria o túnel nem troca de transporte
silenciosamente após um comando que possa ter sido executado.

Após confirmar o caminho de rede, as funções compatíveis funcionam sem cabo.
Vídeo/toque e inspeção por elementos têm estados de disponibilidade separados.

## Tela ao vivo

Após conectar, escolha **Abrir tela**. A janela independente tem fundo externo
transparente, barra arrastável, quatro cantos de redimensionamento proporcional,
minimizar/maximizar/fechar e controles laterais flutuantes. Pressione o mouse para
tocar, mova enquanto segura para arrastar e solte para levantar o dedo. Pausa,
quadros antigos e cancelamentos liberam o contato. Comandos falhos não são repetidos.

As estatísticas mostram FPS de pintura GPUI, intervalo derivado, idade local do
quadro e resposta da API. Esses valores não medem o atraso completo entre mouse e aparelho.

Defina `TETHERDECK_FFMPEG` ou coloque `ffmpeg.exe` ao lado do programa. A busca segue
por `%ProgramFiles%\ffmpeg\ffmpeg.exe` e `PATH`. FFmpeg não é incluído nem baixado
pelo TetherDeck.

## Escolher como inspecionar e automatizar

Em **Elementos**, escolha **Automático**, **WebDriverAgent** ou **Acessibilidade iOS**.
AX lê os elementos pelo túnel pareado, inclusive em Wi-Fi, sem runner WDA; a travessia
pode rolar o app e não oferece ativação genérica. WDA oferece hierarquia e ativação
por referências recentes. A resposta informa qual provedor realmente foi usado.

WDA por Wi-Fi usa o túnel de desenvolvimento. Antes de iniciar, exige revisar que
seu runner respeita o endereço interno do túnel nos servidores HTTP e MJPEG e marcar
**Runner preparado para Wi-Fi isolado** para esse Bundle ID nas opções da sessão.
A opção começa desativada; trocar o Bundle ID ou importar outro IPA invalida a revisão.
Não habilite com base somente no nome ou número da versão. Consulte
[provedores e requisitos](ELEMENT-PROVIDERS.md). AX continua disponível sem essa opção.

## Elementos e assinatura

Nas opções da sessão, importe um WDA `.ipa` já assinado. Revise os metadados e clique
explicitamente para instalar com o iPhone selecionado conectado por USB pareado.
Após a confirmação, o Bundle ID preenche a configuração; abra uma nova sessão
compatível para utilizá-lo. Veja [importação do runner](RUNNER-IMPORT.md).

Um certificado sozinho não compila/assina o runner. O TetherDeck não recebe chave
privada ou senha, nem compila Xcode no Windows. Cada equipe usa sua própria identidade
e perfil para seus aparelhos; o certificado do autor não é compartilhado.

## Preferências e integrações

A interface oferece diagnóstico, elementos e integrações. Clientes MCP usam a mesma
API local autenticada. A configuração MCP exportada contém um token: trate-a como
credencial e não a anexe a um relato público.

Nas preferências, selecione idioma do sistema/português/inglês/espanhol, aparência,
movimento reduzido e descoberta automática. As opções ficam salvas localmente,
fora do repositório. Textos do aparelho e de acessibilidade permanecem no idioma original.

Para ajuda, abra uma issue com diagnóstico redigido e passos para reproduzir.
Não anexe registros de pareamento, chaves, perfis de desenvolvimento ou IPAs pessoais.
