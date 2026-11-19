# Perguntar e agentes

## Perguntar
? abre Perguntar ao lado da placa ou da folha: pergunte sobre o projeto em palavras simples ("O
que é U3?", "Algum erro de ERC?", "Qual a largura da trilha de 5 V?"). Um modelo neste computador
responde, no Ollama: instale de ollama.com e depois `ollama pull gemma4`. Ele consulta as coisas
enquanto responde (as peças, redes, verificações, queda IR, alterações, simulação, os documentos
do projeto e os datasheets das peças), cada consulta mostrada como um chip, e as peças e redes
que ele cita são links para elas.

Ele só olha: nada muda. Enter pergunta, Shift+Enter começa uma nova linha, Esc para uma resposta,
Nova conversa começa de novo, e a lista ao lado de Perguntar escolhe outro modelo (os modelos na
nuvem do Ollama enviam a conversa para ollama.com; os outros ficam neste computador).

## Agentes
Agentes de IA guiam o Cuprita pelos mesmos comandos que a janela usa:
- `cuprita --mcp` os serve por MCP (stdio), para o Claude Code, o Claude Desktop e outros clientes
  MCP;
- `cuprita --json <placa> '{"cmd":"drc"}' '{"cmd":"route",…}'` os roda de um script, uma resposta
  JSON por comando; `cuprita --help` os lista.

Eles abrem ou começam projetos, consultam coisas (peças, redes, a BOM, verificações, queda IR,
alterações), editam (posicionar, rotear empurrando, zonas, preencher, arrumar textos, excluir,
desfazer e refazer), desenham o contorno, definem as regras de projeto, buscam nas bibliotecas do
KiCad, trazem os arquivos de uma peça baixados do SnapMagic, Ultra Librarian ou SamacSys (o modelo
3D também), salvam, gravam os arquivos de fabricação e geram imagens; no esquemático, adicionam
símbolos (também pelo número da peça), fios, barramentos, rótulos e folhas (com os pinos de cada
peça listados, uma rede unida nomeando seus pinos), verificam, simulam e atualizam a placa. Cada
comando e seus argumentos: `crates/app/agent-tools.md`. Deixe o projeto aberto no Cuprita para
acompanhar: o que um agente salva aparece lá em até um segundo, a vista mantida (com edições suas
não salvas, a barra de status avisa em vez disso).
