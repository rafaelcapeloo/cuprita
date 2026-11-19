# Sua primeira placa

De um projeto vazio aos arquivos que a fábrica aceita. Para começar guiado, Ctrl+K › Primeira
placa leva você por uma que acende um LED com uma bateria de moeda, conferindo cada passo.

## 1. Comece um projeto
No Início, Novo projeto (Ctrl+N): dê um nome e escolha Em branco (2 a 16 camadas de cobre e o
tamanho da placa) ou um dos modelos do KiCad. O Cuprita grava os arquivos do KiCad na pasta do
projeto e abre o esquemático, com os Primeiros passos no canto superior direito: cada passo é
marcado quando você chega lá.

## 2. Desenhe o esquemático
- A adiciona uma peça: digite o que ela é ou o número (`R`, `LED`, `LM358DR`, `regulator 3.3`),
  Enter, e clique para colocá-la; R a gira. As referências se numeram sozinhas (R1, R2…).
- W desenha um fio: clique num pino, clique onde ele vira, termine num pino (ou clique duplo, ou
  Enter).
- L nomeia uma rede: digite `SDA` e clique num pino ou fio. Símbolos de alimentação vêm do A:
  `GND`, `+3V3`.
- D verifica o esquemático (ERC): cada problema em palavras simples, com a correção ao passar o
  ponteiro. Q marca um pino que você deixa desconectado de propósito.

## 3. Escolha os footprints
Clique numa peça e, em Inspeção, Footprint › Escolher…: os footprints que servem ao símbolo,
filtrados pelo que você digita (`0603`). Peças encontradas pelo número na Digi-Key já vêm com o
delas.

## 4. Leve para a placa
F8, do esquemático ou da placa, põe as peças abaixo da placa com suas redes como linhas finas:
as conexões que faltam rotear. A placa já tem contorno (o tamanho que você deu, ou o do modelo);
O desenha outro, um retângulo arrastado ou cantos clicados.

## 5. Posicione as peças
Arraste-as para dentro do contorno (uma guia tracejada aparece quando uma se alinha com outra, e a
distância entre elas); R gira, F põe uma no verso, as setas deslocam. Ctrl+E alterna entre o
footprint de uma peça e o símbolo dela.

## 6. Roteie
X, clique num pad e mova: a trilha mantém a folga, e as trilhas de outras redes abrem caminho (P
as contorna). Cada clique fixa a trilha até o ponteiro; ela termina num pad da sua rede. V põe uma
via para o outro lado, W a alarga, / curva para o outro lado, Esc descarta a parte não fixada.
A barra de status conta o que falta: `3 a rotear`.

## 7. Despeje cobre
Z, clique num pad do GND e clique nos cantos da área: Enter preenche, afastando-se das outras
redes. Ela se preenche de novo sozinha quando você move coisas perto dela.

## 8. Verifique e fabrique
D lista o que falta rotear e cada problema de regras de projeto, com a correção. Quando o
cabeçalho disser Pronto para fabricar, Fabricar (Ctrl+4) grava os arquivos (Ctrl+P): Gerbers e
furação num zip, a BOM e as posições das peças, que a JLCPCB, a PCBWay e a OSH Park aceitam como
estão. Seus botões de pedido abrem as páginas delas, e seus números preenchem os orçamentos.
