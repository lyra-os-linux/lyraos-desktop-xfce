# Flavor XFCE experimental

Este repositório é derivado da infraestrutura de imagem do flavor KDE para
preservar os mesmos gates, políticas de release e recuperação. Ele não é ainda
uma imagem XFCE funcional nem uma edição suportada do Lyra OS.

## Etapas

1. Alpha 1: openSUSE Leap, XFCE e LightDM exclusivamente com componentes da base.
2. Alpha 2: wallpapers versionados e identidade visual mínima do Lyra.
3. Alpha 3: integração experimental do `vega-xfce`.

A edição GNOME continua oficial. Este flavor não deve consumir pacotes próprios
do Lyra antes que a etapa correspondente tenha gates de boot, instalação,
atualização e rollback definidos e aprovados.
