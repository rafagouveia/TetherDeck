# Importar seu runner de automação

O TetherDeck para Windows importa um **WebDriverAgent XCTest runner já compilado e
assinado**, empacotado como `.ipa`. A análise do arquivo não instala nem executa
o app; a instalação exige clicar em **Instalar no iPhone por USB**.

## Certificado próprio

O certificado do autor do projeto não é distribuído. Cada usuário ou equipe usa
sua própria identidade Apple de desenvolvimento e um perfil que autorize seus
aparelhos. Um `.cer` isolado contém a parte pública e não consegue assinar um app.
Seu processo de assinatura precisa da chave privada correspondente — normalmente
exportada em `.p12` — e do perfil apropriado, além do runner compilado.

Prepare o IPA no seu processo de build/assinatura, incluindo uma máquina ou serviço
macOS quando precisar compilar o projeto Xcode. Esta versão do TetherDeck não
compila Xcode no Windows nem recebe `.p12`, senha, chave privada ou login Apple.
Não coloque esses materiais no repositório ou nos pacotes públicos.

Os requisitos de perfil, aparelhos autorizados, validade e entitlements são
descritos pela [Apple, TN3125](https://developer.apple.com/documentation/technotes/tn3125-inside-code-signing-provisioning-profiles).
A documentação oficial do [Appium sobre WDA pré-instalado](https://github.com/appium/appium-xcuitest-driver/blob/master/docs/guides/run-preinstalled-wda.md)
explica a preparação de um runner previamente assinado e diferenças entre versões.
O formato e a versão do runner precisam ser compatíveis com o iOS usado.

## Fluxo no Windows

1. Informe o UDID do iPhone nas opções da sessão.
2. Escolha **Selecionar .ipa assinado**.
3. Revise nome, Bundle ID, equipe, validade e aparelho autorizado.
4. Conecte esse iPhone por USB, desbloqueado e já pareado com o computador.
5. Clique em **Instalar no iPhone por USB**. O iOS decide se a assinatura é aceita.
6. Após a confirmação, o Bundle ID preenche a configuração. Abra uma nova sessão
   com transporte compatível com WDA para ativar a inspeção por elementos.

Se houver erro ou prazo excedido, confira o resultado no aparelho e o diagnóstico
antes de tentar novamente. O programa não repete automaticamente a instalação.
Instalar o pacote não comprova que o runner consegue iniciar uma sessão XCTest;
essa etapa é confirmada quando a sessão de automação é aberta.

## O que é conferido

- Um único `Payload/*.app`, Bundle ID terminado em `.xctrunner`, executável,
  recursos de assinatura e plug-in `WebDriverAgentRunner` de tipo XCTest.
- Plataforma de aparelho físico `iPhoneOS`.
- Estrutura CMS do perfil embarcado e metadados de desenvolvimento: validade,
  UDID, equipe, App ID autorizado e `get-task-allow=true`.
- Nomes e cabeçalhos ZIP consistentes, CRC e tamanho real de cada entrada.

**A conferência local é de estrutura e metadados.** Não verifica a assinatura
criptográfica do executável, a cadeia Apple, revogação ou todos os entitlements
efetivos do binário. A validação de instalação é feita pelo próprio iOS.

O importador aceita ZIP comum com Store/Deflate, até 256 MiB comprimidos,
512 MiB descomprimidos, 8.192 entradas e 128 MiB por entrada. Metadados têm limites
menores. Rejeita ZIP64, múltiplos volumes, conteúdo fora de `Payload`, caminhos
ambíguos, travessia de diretórios, entradas duplicadas, links e arquivos especiais.
Arquivos de chave como `.p12`, `.p8`, `.pem` e `.key` dentro do IPA são rejeitados.

O pacote fica somente na memória do processo e não é extraído no Windows. Durante
a instalação é enviado via AFC para um nome aleatório próprio em `PublicStaging`,
e esse arquivo é removido ao concluir ou falhar, quando o USB permite. Se a limpeza
não for confirmada, a interface informa isso. Nenhum diretório ou outro app é
removido nessa limpeza. Não há dependência de Python para este fluxo.
