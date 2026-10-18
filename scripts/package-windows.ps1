[CmdletBinding()]
param([Parameter(Mandatory)][string]$BinariesDirectory)
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$destination=Join-Path $workspace 'outputs\windows-gpui'
New-Item -ItemType Directory -Path $destination -Force|Out-Null
$names=@('rust-ios-device.exe','rust-ios-device-daemon.exe','rust-ios-device-mcp.exe')
foreach($name in $names) {
    $source=Join-Path $BinariesDirectory $name
    if(-not (Test-Path -LiteralPath $source -PathType Leaf)){throw "Binario ausente: $source"}
    Copy-Item -LiteralPath $source -Destination (Join-Path $destination $name) -Force
}
Copy-Item -LiteralPath (Join-Path $BinariesDirectory 'tetherdeck-desktop.exe') -Destination (Join-Path $destination 'TetherDeck.exe') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'start-studio.ps1') -Destination (Join-Path $destination 'Start-Studio.ps1') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'docs\GETTING-STARTED.md') -Destination (Join-Path $destination 'COMECE-AQUI.md') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'docs\GETTING-STARTED.md') -Destination (Join-Path $destination 'GETTING-STARTED.md') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'docs\GETTING-STARTED.en.md') -Destination (Join-Path $destination 'GETTING-STARTED.en.md') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'docs\GETTING-STARTED.es.md') -Destination (Join-Path $destination 'GETTING-STARTED.es.md') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'docs\RUNNER-IMPORT.md') -Destination (Join-Path $destination 'RUNNER-IMPORT.md') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'docs\ELEMENT-PROVIDERS.md') -Destination (Join-Path $destination 'ELEMENT-PROVIDERS.md') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'LICENSE') -Destination (Join-Path $destination 'LICENSE') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'crates\device-desktop\assets\icons\LICENSE-LUCIDE') -Destination (Join-Path $destination 'THIRD-PARTY-ICONS.txt') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'docs\THIRD-PARTY-GPUI.txt') -Destination (Join-Path $destination 'THIRD-PARTY-GPUI.txt') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'vendor\idevice\LICENSE-MIT') -Destination (Join-Path $destination 'THIRD-PARTY-IDEVICE.txt') -Force
@'
TetherDeck - Windows x86_64 - GPUI Preview 0.1

Abra TetherDeck.exe. Mantenha todos os executaveis na mesma pasta.
Em Conectar, escolha USB ou Wi-Fi e busque o aparelho. O daemon local
usa uma credencial temporaria em memoria. Fechar a sessao encerra o daemon.

Elementos: inspecao e seletores. Abrir tela: janela independente do aparelho,
com toque, arraste e controles laterais. Ctrl+B alterna a barra lateral.
Diagnostico: camadas da conexao e recuperacao. Integracoes: MCP e assinatura.

Ativacao por elementos exige WebDriverAgent assinado pela sua equipe
Apple e instalado para o aparelho. Este pacote nao inclui runner pessoal,
perfil de desenvolvimento ou chave privada. A interface importa um IPA WDA
ja assinado e instala por USB pareado. O iOS valida a assinatura final.
Um certificado .cer sozinho nao assina o runner. Veja COMECE-AQUI.md.

Wi-Fi direto usa pareamento Lockdown e registro remoto existentes.
Tela ao vivo usa FFmpeg externo com decoder HEVC. Configure TETHERDECK_FFMPEG,
coloque ffmpeg.exe ao lado do aplicativo, em ProgramFiles/ffmpeg, ou no PATH.
FFmpeg nao esta incluido. A nova interface usa GPUI Kit e renderizacao DirectX 11.
O preview limita a largura a 720 px para reduzir trafego de pixels e atraso.
As coordenadas de controle continuam na resolucao original do aparelho.
Pressionar inicia o toque, mover acompanha o cursor e soltar libera o contato.
Posicoes pendentes sao substituidas pela mais recente, sem replay do trajeto.
Controle ativo habilita toque e arraste em quadros com geometria valida.
Falhas recuperaveis do video reconectam automaticamente, descartando o fluxo
anterior. Espera progressiva ate 10 s; pausa e fechar cancelam as tentativas.
Gestos nunca sao repetidos. Falhas de preparo/autorizacao pedem correcao.
Sidebar com transicoes nativas; movimento reduzido em Configuracoes.
Tema One Dark, botoes com relevo discreto e nova identidade visual.
Moldura proporcional e controles laterais acompanham a imagem do aparelho.
Janela remota com fundo externo transparente, barra arrastavel, controles de
minimizar/maximizar/restaurar/fechar e redimensionamento pelos quatro cantos.
Estatisticas mostra FPS, intervalo, idade local do quadro e resposta da API;
esses contadores nao medem atraso completo entre mouse e tela do aparelho.
Capturas e revisao manual complementam a compilacao; latencia ponta a ponta
e fluidez percebida ainda dependem da avaliacao no aparelho.
As medicoes anteriores do egui nao comprovam o desempenho desta interface GPUI.
Elementos: escolha Auto, WDA ou Acessibilidade iOS (AX). AX le sem runner.
WDA Wi-Fi exige runner revisado para bind HTTP/MJPEG no tunel pareado.
Configuracoes oferece tema, idioma PT/EN/ES, movimento e busca automatica.
Veja ELEMENT-PROVIDERS.md para requisitos e limites.

Codigo open source sob MIT. Nomes tecnicos rust-ios-device-* e variaveis
RUST_IOS_DEVICE_* continuam compativeis. Start-Studio.ps1 e opcional.
'@ | Set-Content -LiteralPath (Join-Path $destination 'README.txt') -Encoding utf8
$names+=@('TetherDeck.exe','Start-Studio.ps1','README.txt','COMECE-AQUI.md','GETTING-STARTED.md','GETTING-STARTED.en.md','GETTING-STARTED.es.md','RUNNER-IMPORT.md','ELEMENT-PROVIDERS.md','LICENSE','THIRD-PARTY-ICONS.txt','THIRD-PARTY-GPUI.txt','THIRD-PARTY-IDEVICE.txt')
$manifest=[ordered]@{product='TetherDeck';platform='windows-x86_64';generated_at=[DateTime]::UtcNow.ToString('o');files=@($names|ForEach-Object {$hash=Get-FileHash -LiteralPath (Join-Path $destination $_) -Algorithm SHA256;[ordered]@{name=$_;sha256=$hash.Hash.ToLowerInvariant()}})}
$manifest|ConvertTo-Json -Depth 4|Set-Content -LiteralPath (Join-Path $destination 'manifest.json') -Encoding utf8
# Package only release files; personal provisioning and extra files are excluded.
$files=@(($names+@('manifest.json'))|ForEach-Object {Join-Path $destination $_})
Compress-Archive -LiteralPath $files -DestinationPath (Join-Path $workspace 'outputs\tetherdeck-windows-x86_64.zip') -Force
