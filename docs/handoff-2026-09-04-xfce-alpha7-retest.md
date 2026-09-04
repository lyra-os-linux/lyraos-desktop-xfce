# Handoff — reteste XFCE 1.1 Alpha 7

Data do handoff: 2026-09-04, fuso America/Sao_Paulo.

## Objetivo e decisão atual

A candidata XFCE deve ser reconstruída amanhã. A ISO testada hoje não pode ser
publicada porque foi construída quando o canal release do OBS ainda fornecia
`lyra-release` 1.0. O pacote já foi corrigido, testado, promovido e publicado
como 1.1. Não repetir o ciclo completo de atualização/rollback: esse fluxo já
passou e nenhum código dele mudou. O reteste da nova ISO deve se limitar aos
gates do novo artefato, live/instalação, primeiro boot e Secure Boot.

## Estado do Git

Repositório: `lyraos-desktop-xfce`, branch `main`.

- Estado observado antes do commit deste handoff:
  `main...origin/main [ahead 3]`.
- Commits locais já existentes:
  - `19e1734 Fix XFCE live display manager smoke check`;
  - `ce16ced Align OBS release inventory with published projects`;
  - `6781078 Seed Packman signing key in installed image`.
- Alterações incluídas no commit deste handoff:
  - `AGENTS.md`: execução dos testes pelo agente e lição aprendida sobre
    identidade publicada;
  - `kiwi/test/build-and-run-vm.sh`: novo gate semântico de `lyra-release`;
  - `tests/test_image_build.py`: cobertura do novo gate;
  - este documento.
- Validação pré-commit concluída: 185/185 testes Python passaram,
  `bash -n kiwi/test/build-and-run-vm.sh` passou e `git diff --check` não
  encontrou erros. Antes do próximo build, apenas confirme que o repositório
  continua limpo para que a evidência não marque `dirty=true`.

A fonte canônica de `lyra-release` está em `lyraos-desktop`, branch
`alpha8-fail-closed-gates`, limpa e sincronizada com
`origin/alpha8-fail-closed-gates`. O commit `87e0b1c` alterou o pacote de 1.0
para 1.1.

## ISO antiga — preservar apenas como evidência do defeito

- Caminho:
  `/tmp/lyraos-xfce-current-6781078/iso/LyraOS-Desktop-XFCE-1.1-alpha.7-x86_64.iso`
- Tamanho: `2726895616` bytes.
- SHA-256:
  `c57bbfef7c061bc39b03c81d21e21cda6d7befe9642339080fc02d6d26dc8cd8`.
- Fonte: `678107866197651c2d8321c973379904be5d6786`, árvore limpa no momento
  daquele build.
- Não publicar essa ISO. Ela contém `/usr/lib/lyra-os/product-release` com
  `LYRA_VERSION_ID='1.0'` e `LYRA_BUILD_ID='lyra-release-1.0'`.
- `/etc/os-release` e `/usr/lib/os-release` apareciam corretamente como 1.1
  porque o overlay KIWI os reescreve; isso mascarou o RPM antigo.
- A VM foi fechada após a coleta das evidências.

## Testes concluídos na ISO antiga

Estes resultados continuam úteis para os componentes que não mudaram:

- 185 testes Python passaram.
- Instalador Rust: `cargo fmt` e `clippy` passaram; 109 testes passaram e um
  teste dependente do sistema ficou ignorado.
- `./scripts/release.py check` passou.
- `./scripts/image-build.py validate` passou.
- Build KIWI limpo com instalador publicado passou.
- SquashFS foi extraído integralmente e auditado.
- 1.340 pacotes: nenhuma dependência quebrada e nenhum pacote real sem
  assinatura; apenas os pseudo-pacotes `gpg-pubkey` aparecem como unsigned.
- Os seis repositórios atualizaram sem importação automática de chave.
- Não havia atualização pendente.
- A chave Packman esperada estava no rootfs e no banco RPM.
- Instalação gráfica em Btrfs com ZRAM concluída.
- Primeiro boot passou com reconhecimento explícito de duas mensagens benignas
  do journal no QEMU: TDX não suportado e mensagem inicial do gkr-pam; o daemon
  `gnome-keyring-daemon` estava ativo.
- Smoke de UEFI/Secure Boot: `status: passed`.
- Atualização e rollback estruturados concluíram toda a sequência:
  `baseline` → `updated` → `updated-verified` → `rollback-prepared` →
  `rollback-verified`, com resultado final `status: passed`.
- O smoke estruturado da sessão live não foi produzido. A tela live e o
  instalador foram observados, mas o helper ainda não oferece canal de comando
  no guest live e o instalador em primeiro plano impediu a coleta confiável.

## Defeito encontrado e lição aplicada

O gate anterior verificava existência do pacote, assinatura, dependências,
publicação e `/etc/os-release`, mas não comparava a identidade semântica do RPM
`lyra-release` com `release.toml`. Por isso o pacote 1.0 passou pelo gate e o
fluxo caro de VM foi executado antes da descoberta.

Foi adicionada uma regra permanente em `AGENTS.md` e um gate em
`kiwi/test/build-and-run-vm.sh`. Depois do build, ele agora exige
simultaneamente:

- versão do RPM `lyra-release` igual ao `product_version` de `release.toml`;
- `LYRA_VERSION_ID` correspondente em
  `/usr/lib/lyra-os/product-release`;
- `LYRA_BUILD_ID='lyra-release-<product_version>'` no mesmo arquivo.

O helper passou em `bash -n`. O teste direcionado
`ImagePolicyTests.test_vm_helper_validates_product_and_artifact_versions`
também passou, seguido pela suíte Python completa com 185/185 testes aprovados.

## Estado final do OBS

Pacote: `lyra-release`, alvo `openSUSE_Leap_16.1/x86_64`.

- Staging: `home:rodrigosbrito:lyra:staging`, revisão 8, publicado e
  `succeeded`.
- Revisão-fonte promovida (`srcmd5`):
  `6652a6472f6c3c9851d58db3d65d7e7e`.
- Build local limpo com `osc build --clean --checks`: exit 0; `%check`,
  instalação do RPM e checks pós-build passaram.
- Gate completo do staging passou:
  - Lyra: 16 pacotes-fonte;
  - Vega: 6 pacotes-fonte;
  - Fina: 1 pacote-fonte;
  - todos os alvos publicados.
- Request de promoção: `1375841`, revisado com diff e source buildstatus e
  aceito em 2026-09-04.
- Release: `home:rodrigosbrito:lyra`, revisão 2, publicado e `succeeded`.
- Gate completo do release passou para Lyra, Vega e Fina.
- RPM final verificado:
  `lyra-release-1.1-lp161.1.1.noarch.rpm`.
- Assinatura/digests: OK.
- Payload final:

  ```text
  LYRA_VERSION_ID='1.1'
  LYRA_EDITION='desktop'
  LYRA_ARCHITECTURE='x86_64'
  LYRA_BUILD_ID='lyra-release-1.1'
  ```

## Sequência de retomada amanhã

1. Confirmar `git status` limpo e a branch `main` sincronizada no
   `lyraos-desktop-xfce`.
2. Executar o health gate do OBS e preservar o JSON de evidência. O check
   básico já passou, mas o health profundo ainda não foi concluído nesta
   sessão.
3. Fazer um único build KIWI novo em diretório de trabalho novo, consumindo o
   canal release e o instalador publicado.
4. Antes de iniciar a VM, confirmar no rootfs construído:
   - RPM `lyra-release` em `1.1-lp161.1.1` ou revisão posterior equivalente;
   - `product-release` em 1.1/`lyra-release-1.1`;
   - assinaturas, dependências, repositórios, ausência de updates e auditoria
     completa do SquashFS.
5. Somente se todos os gates anteriores passarem, executar na nova ISO:
   sessão live, instalação, primeiro boot e Secure Boot.
6. Não repetir atualização/rollback, salvo se surgir mudança em updater,
   Snapper, identidade de upgrade ou política de repositórios.
7. Gerar o manifesto/evidência final ligado ao novo SHA-256. A ISO antiga não
   pode ser reutilizada nem publicada.

## Observações operacionais

- GNOME e KDE já publicados tiveram seus artefatos locais removidos a pedido;
  nenhum artefato remoto foi apagado.
- O layout da VM é pt-BR. No monitor QEMU, `sendkey slash` produz `;`; para `/`
  deve ser usado `sendkey kp_divide`.
- O host ainda possui repositórios Lyra/Fina/Vega antigos de Leap 16.0. Isso não
  afetou a VM ou o OBS, mas pode causar exit 106 em operações `zypper` do host.
