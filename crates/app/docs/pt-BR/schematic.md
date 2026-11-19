# Esquemático

Ctrl+E mostra o esquemático do projeto (de novo, a placa). Arrastar move a vista, a roda dá zoom,
Home ajusta a folha à tela. Cada folha aparece como o KiCad a desenha, no papel dela com a moldura
e o carimbo como Ctrl+Shift+P a imprime, e é salva como o KiCad grava.

## Peças
A adiciona um símbolo (R o gira antes do clique que o coloca); encontrar peças, trazê-las da
web e fazer as suas: [Peças](parts.md).

## Fios e redes
- W escolhe a ferramenta de fio: um clique começa o fio, cada clique seguinte acrescenta um
  trecho, dobrado uma vez pelo lado mais longo primeiro (/ dobra para o outro lado, Shift vai
  reto em qualquer ângulo); ele termina num pino, fio ou rótulo, ou com clique duplo ou Enter.
  As junções vão onde o KiCad precisa delas.
- L nomeia uma rede: digite o nome (`SDA`, `~{RESET}`), Enter, e clique num pino ou fio. Ctrl+L
  o torna global (ligado em todas as folhas); H, numa folha abaixo da raiz, hierárquico (ligado à
  folha de cima por um pino com o mesmo nome na caixa da folha).
- B desenha um barramento, Z suas entradas a 45°; um rótulo nomeia os membros: `D[0..7]`,
  `{SDA SCL}`.
- Q, e um clique, marca um pino deixado desconectado de propósito; um clique numa marca a retira.
  Clique num fio ou rótulo para acender a rede em todas as folhas; Ctrl+E então a acende na placa.

## Folhas
Nova folha, na aba Projeto, adiciona uma folha abaixo da mostrada, com a caixa dela onde houver
espaço: digite o nome, Enter. Shift+S faz isso onde você arrastar a caixa. A aba Projeto lista as
folhas, um clique mostra uma; um clique duplo numa caixa abre a folha dela, Backspace (ou o caminho
no canto superior esquerdo) volta para cima.

## Editar
Clique seleciona um símbolo, fio, rótulo ou marca de não conectado; Shift+clique acrescenta ou
tira, Shift+arrastar desenha uma caixa (da esquerda para a direita pega o que ela contém, da
direita para a esquerda o que ela toca). Arrastar move o que está selecionado, e os fios
continuam ligados aos pinos; R gira, Delete exclui. Ctrl+Z desfaz qualquer passo. A referência e
o valor de um símbolo vão ao lado dele quando ele entra, num lado sem pinos; T (Arrumar textos)
põe ali de novo os de todos os símbolos da folha.

## Verificar
D lista cada problema de regras elétricas, como o KiCad os verifica com as severidades do
projeto: erros em vermelho, avisos em âmbar. Passe o ponteiro sobre um para ver a correção;
clique para vê-lo na folha.

## Simular
S roda o esquemático no ngspice como o simulador do KiCad: o modelo de cada peça (senão, o valor),
as diretivas das folhas (`.tran 1u 10m`) e as análises. Os gráficos aparecem abaixo da folha:
arrastar ou a roda os move, Ctrl+roda dá zoom, passar o ponteiro lê um ponto. S de novo para.

## Para a placa
F8 leva as peças e redes para a placa: peças novas em fileiras abaixo dela, as de cada folha
juntas, footprints trocados no lugar, valores e campos levados junto. Salvar (Ctrl+S) grava as
folhas e a placa.
