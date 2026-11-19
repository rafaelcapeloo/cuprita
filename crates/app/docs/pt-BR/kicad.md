# Trabalhar com o KiCad

Os arquivos do Cuprita são os do KiCad: não há nada a importar ou exportar, e um projeto vai e
volta entre os dois como você quiser.

## O que ele abre
Placas do KiCad 4 ao 10, esquemáticos do KiCad 6 ao 10, com o `.kicad_pro` (regras de projeto,
classes de rede, larguras de trilha). Um `.kicad_pro`, `.kicad_pcb` ou `.kicad_sch` abre com
Abrir (Ctrl+O) ou soltando-o na janela (Windows, macOS, X11: o Wayland ainda não passa o que se
solta para o Cuprita).

## O que ele salva
- Tudo o que você não mudou fica byte a byte como estava, campos desconhecidos inclusive; o que
  você mudou é gravado como o KiCad grava, e o KiCad o lê de volta igual.
- Antes de cada salvamento, a versão anterior do arquivo é guardada como `<arquivo>-bak`.
- As edições são salvas automaticamente 5 s depois da última em `_autosave-<arquivo>` (o nome do
  KiCad), até você salvar; abrir um arquivo com um salvamento automático mais novo pergunta se
  deve restaurá-lo.

## As mesmas teclas
O Cuprita mantém as teclas do KiCad onde o KiCad tem uma: X roteia, V via, W e Shift+W larguras,
/ curva, B preenche, F vira, R gira, U trecho, N grade, F8 atualiza, Ctrl+E alterna, PgUp e PgDn
camadas, Ctrl+, configurações. F1 lista todas.

## Onde ele difere
- Empurrar fica ligado ao rotear; P contorna em vez disso (o KiCad tem os dois como modos do
  roteador).
- As verificações de regras de projeto e as conexões que faltam são refeitas enquanto você edita:
  a verificação completa de uma placa grande leva milissegundos. As zonas se preenchem de novo
  perto de cada edição, no mesmo passo de desfazer (o KiCad espera pelo B).
- A ajuda, as verificações e os erros dizem o que fazer a seguir, em palavras simples.

## Ainda é com o KiCad
Desenhar um símbolo ou footprint à mão (Nova peça… faz um a partir dos pinos e do
encapsulamento), gerenciar bibliotecas, vias cegas e enterradas, ajuste de comprimento e pares
diferenciais, e plotar qualquer coisa além dos arquivos de fabricação. Use o KiCad para isso: ele
abre o que o Cuprita salva, e o Cuprita o que ele salva.
