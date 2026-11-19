# Placa

Arrastar move a vista, a roda dá zoom, Home ajusta a placa à tela, F (sem peça selecionada) vira
a vista para o verso. As ferramentas esperam na base da tela, a que está ligada diz o que fazer no
topo, e Esc a desliga.

## Selecionar, mover, girar
- Clique numa peça, trilha, via ou zona; Shift+clique acrescenta ou tira; Shift+arrastar desenha
  uma caixa (da esquerda para a direita pega o que ela contém, da direita para a esquerda o que
  ela toca).
- Arrastar move o que está selecionado, na grade (N troca o passo; Ctrl solta da grade); R gira
  90°, F põe peças no verso, as setas deslocam, Delete exclui. Cada um é um passo para o Ctrl+Z.
- Uma trilha selecionada sozinha é arrastada com as vizinhas seguindo a 45°; U seleciona o
  trecho inteiro; Inspeção define a largura das trilhas selecionadas (`0.3mm`, `12mil`).
- A adiciona um footprint das bibliotecas do KiCad: furos de fixação, pontos de teste, fiduciais,
  logos.

## Rotear
X, e clique num pad, via ou trilha: a trilha segue o ponteiro mantendo a folga (a faixa suave em
volta dela). Trilhas e vias de outras redes abrem caminho, e o que elas empurram também se move;
P as contorna em vez disso, também nas próximas trilhas.
- Um clique fixa a trilha até o ponteiro; ela termina num pad, via ou trilha da sua rede.
- V põe uma via e continua do outro lado; PgUp, PgDn, + e − continuam em outra camada.
- W e Shift+W passam pelas larguras do projeto; / curva para o outro lado.
- Um anel vermelho mostra onde ela não alcança, com o espaço que precisa. Esc descarta a parte
  não fixada.

## Zonas
Z, clique num pad da rede da zona e depois nos cantos (a 45°, a não ser com Shift); Enter fecha e
preenche. Selecione uma zona para arrastar os cantos, acrescentar cantos no meio das arestas,
definir prioridade, folga, alívios térmicos e mais em Inspeção, ou abrir um furo (Shift+C). As
zonas se preenchem de novo sozinhas perto do que você edita; B preenche todas de novo.

## Contorno
O, e arraste um retângulo ou clique nos cantos: o novo contorno da placa, mantendo os furos dele.

## Textos
Um clique na referência ou no valor de uma peça seleciona só esse texto: arrastar move, R gira, Inspeção define tamanho e traço, Delete oculta; selecione a peça e clique nele em Textos ocultos, na Inspeção, para mostrá-lo de novo.
T arruma os textos das peças selecionadas, ou de todas: cada referência fica legível, a 0° ou 90°, com 1 mm de altura (o menor que as fábricas
imprimem), logo fora da peça, longe dos pads, da outra serigrafia e da borda (oculta onde não cabe); cada valor vai para dentro da peça na
camada Fab, que aparece em "mais N, ocultas" em Camadas. Um passo para o Ctrl+Z.

## Camadas
A camada ativa (PgUp, PgDn, + e −, ou um clique em Camadas) é onde vão as trilhas, desenhada mais
clara; o olho ao lado de cada camada a mostra ou esconde. Camadas internas mostram seu plano:
`In1.Cu · GND`.

## Verificações
D lista as conexões que faltam rotear (linhas finas na placa, contadas na barra de status) e cada
problema de regras de projeto, pelas regras do projeto: passe o ponteiro para ver a correção,
clique para vê-lo. As edições são verificadas de novo enquanto você as faz.

## Medir e ver
M mede entre dois pontos (no cobre ou na grade). Alt+3 mostra a placa em 3D com os modelos das
peças: arrastar gira, a roda dá zoom, F vira, Esc volta. A vista desliza até o que você procura;
Configurações (Ctrl+,) tem Movimento reduzido.

## O esquemático ao lado
Ctrl+E alterna entre o footprint de uma peça e o símbolo dela, e entre uma rede na placa e nas
folhas. F8 leva as mudanças do esquemático para a placa.
