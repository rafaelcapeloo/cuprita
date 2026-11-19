# Fabricar

Fabricar (Ctrl+4, ou o selo no cabeçalho: `2 a corrigir`, `Pronto para fabricar`) reúne numa
página o que um pedido precisa.

## Antes de pedir
Um anel conta as verificações que passam, e uma frase diz o que falta ("Roteie as últimas 3
conexões e corrija 2 problemas de regras de projeto"). Cada verificação diz do que trata, e uma
que falha tem a correção a um clique:
- as regras elétricas do esquemático;
- a placa igual ao esquemático (senão Atualizar traz as mudanças, como o F8);
- peças na placa, um contorno fechado (com o tamanho);
- todas as conexões roteadas, nenhum problema de regras de projeto;
- as zonas preenchidas;
- números de peça (MPN ou LCSC) para a montagem: Preencher mostra a primeira peça sem um.

## Os arquivos de fabricação
Exportar (Ctrl+P) preenche as zonas e grava ao lado da placa:
- `<placa>-gerbers.zip`: cobre, máscara, pasta, serigrafia e contorno em Gerber X2, e a furação
  em Excellon, com os nomes que o KiCad dá;
- `<placa>-bom.csv` e `<placa>-positions.csv`: a lista de peças e onde cada uma fica, nas colunas
  da JLCPCB, com os campos MPN e LCSC.

Um arquivo que já estava lá é guardado como `<arquivo>-bak`. A JLCPCB, a PCBWay e a OSH Park
aceitam os arquivos como estão; Mostrar abre a pasta deles.

Ctrl+Shift+P (PDF do esquemático) grava todas as folhas em `<projeto>.pdf` ao lado dele,
numeradas como no KiCad, cada uma com moldura e legenda, e o abre: para quem monta ou revisa a
placa.

## Pedir
Fabricar lista os números que o orçamento de uma fábrica pede (tamanho, camadas de cobre,
espessura, menor trilha e espaço, menor furo, furos, peças em cada lado) e abre as páginas de
pedido da JLCPCB, da PCBWay e da OSH Park, onde você envia o zip. Também mostra a placa em 3D
como a fábrica a faz.

## Montar à mão
Shift+B abre a lista de montagem (BOM): uma linha por valor e footprint. Uma linha marca as peças
dela na placa; Espaço a marca como feita e passa para a próxima. As marcas ficam para a próxima
vez que a placa abrir.
