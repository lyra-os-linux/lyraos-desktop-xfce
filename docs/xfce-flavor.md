# Flavor XFCE oficial

O XFCE é um flavor oficial do Lyra OS, criado para oferecer uma experiência
mais leve sem abrir mão das políticas de estabilidade, atualização e
recuperação do projeto. Este repositório deriva da infraestrutura de imagem do
flavor KDE para preservar os mesmos gates e critérios de release.

## Etapas de integração

1. Base openSUSE Leap com XFCE e LightDM.
2. Wallpapers versionados e identidade visual do Lyra.
3. Integração do `vega-xfce`.
4. Qualificação de boot, instalação, atualização e rollback.

A edição GNOME continua sendo o flavor principal, enquanto KDE e XFCE são
flavors oficiais com integração, artefatos e validação próprios. A inclusão de
pacotes deve acompanhar os gates definidos para cada etapa do flavor.
