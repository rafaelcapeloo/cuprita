# Peças

Uma peça é um símbolo no esquemático e um footprint na placa. As peças que um projeto traz ou
faz vão para a biblioteca própria dele (`libraries/` ao lado), que o KiCad também lê.

- A adiciona um símbolo das bibliotecas do KiCad, encontrado pelo nome, pelo que ele é ou pelo
  número da peça (`R`, `usb c`, `LM358DR`); R o gira antes do clique que o coloca. O Cuprita já
  vem com os símbolos, footprints e modelos 3D do KiCad; com o KiCad instalado, usa os dele.
- Um número de peça também pode vir da Digi-Key: o símbolo do KiCad para ela (ou um de resistor,
  capacitor ou indutor com o valor), o footprint, o datasheet, o fabricante e o número. Isso
  pede sua própria chave, gratuita: crie um app com Product Information V4 em
  developer.digikey.com e cole o Client ID e o Secret em Configurações (Ctrl+,).
- Uma peça que o KiCad não tem: Peças leva ao SnapMagic, ao Ultra Librarian e ao SamacSys; baixe
  lá o zip do KiCad dela, e Adicionar arquivos de peça… a põe na biblioteca própria do projeto,
  que o KiCad também lê, e Peças a mostra: Enter a adiciona, e F8 leva o footprint e o modelo 3D
  dela para a placa.
- Ou faça-a: Nova peça… (em Peças, ou Ctrl+K) pede o nome, os pinos (digite quantos, Preencher,
  depois dê os nomes e escolha o tipo e o lado de cada um) e o footprint, escolhido das
  bibliotecas ou feito a partir do encapsulamento (SOIC, QFN, uma barra de pinos…, com as medidas
  do datasheet); os dois aparecem enquanto você digita, e Criar peça os põe na biblioteca do
  projeto, prontos para adicionar.
- Clique numa peça para ver os campos dela em Inspeção e digitar neles (Valor, Datasheet, MPN,
  LCSC…); Footprint › Escolher… lista os footprints que servem a ela.
