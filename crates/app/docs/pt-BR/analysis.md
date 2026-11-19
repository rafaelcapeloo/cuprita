# Análise

Números sobre a placa como ela está, incluindo as edições ainda não salvas.

## Uma rede
Clique numa trilha, via ou pad para acender a rede dela: Inspeção dá o comprimento, a largura
mais estreita, a corrente que ela leva com 10 °C de aquecimento (IPC-2152) e a impedância como
microstrip ou stripline sobre o empilhamento. Localizar (Ctrl+F) acende uma rede pelo nome.

## Queda IR
I mostra um mapa de calor da queda de tensão em CC ao longo da rede selecionada (senão, a rede de
alimentação principal) com uma corrente de teste: resistência, potência perdida e densidade de
corrente, por trilhas, vias, pads e zonas. Um clique num pad dá zoom nele; um clique com o botão
direito o torna a fonte.

## Alterações desde um commit
G compara a placa com o último commit dela no git: cobre acrescentado em verde, removido em
vermelho; um clique numa alteração dá zoom nela. Na linha de comando,
`cuprita --diff [commit ou arquivo] <placa>` faz o mesmo.

## As conexões que faltam
As linhas finas ligam o cobre de cada rede ainda separado, pelo caminho mais curto. D as lista
como o DRC do KiCad lista os itens não conectados (`GND: C1 pad 2 – U1 pad 2`); um clique dá zoom
numa delas.

## Simulação
No esquemático, S roda o ngspice: veja [Esquemático](schematic.md).
