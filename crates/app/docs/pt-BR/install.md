# Instalar o Cuprita

As versões publicadas do projeto têm um instalador para cada sistema: Windows
(`…-windows-x64-setup.exe`), macOS (`.dmg`: arraste o Cuprita para Aplicativos; a primeira vez,
abra com clique direito › Abrir, pois ele não é notarizado), Debian e Ubuntu (`.deb`:
`sudo apt install ./cuprita_*.deb`), Fedora (`.rpm`) e qualquer Linux com Flatpak
(`flatpak install --user cuprita.flatpak`).

## Pelo código
O Cuprita é compilado a partir do código com Rust (1.95 ou mais novo) e um linker de C. Instale o
Rust uma vez com `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh` e baixe o
código: `git clone <o repositório> && cd cuprita` (depois, `git pull` atualiza).

## Linux
- Só para você: `./install.sh` compila o Cuprita e o coloca na grade de apps, com `cuprita` em
  `~/.local/bin`. Rode de novo depois de `git pull` para atualizar; `./install.sh remove` o tira.
- Para todos os usuários, como pacote: `./package.sh` gera um para o sistema em que roda, em
  `target/package`:
  - Fedora, RHEL, openSUSE: `sudo dnf install target/package/cuprita-*.rpm`
  - Debian, Ubuntu: `sudo apt install ./target/package/cuprita_*.deb`

  Gere cada pacote no sistema em que ele será instalado: o app usa a biblioteca C desse sistema.
- Ou rode a partir do código: `cargo run --release -p cuprita -- [projeto ou placa]`.

A primeira compilação leva alguns minutos; as seguintes, alguns segundos.

## KiCad, ngspice e Ollama (todos opcionais)
O Cuprita encontra estes sem nada a configurar, e o Início lista o que encontrou:
- KiCad (de preferência o 10): as bibliotecas de símbolos e footprints (Adicionar peça), os
  modelos 3D e o ngspice. Sem ele o Cuprita abre e edita projetos, mas não tem bibliotecas de onde
  adicionar peças.
- ngspice, para simular: vem com o KiCad. Quando o do KiCad em Flatpak é feito para um sistema
  mais novo que o seu, o Início avisa: instale o do próprio sistema (`libngspice`).
- Ollama, para Perguntar: instale de ollama.com e depois `ollama pull gemma4`.

## No navegador
`./web.sh serve`, abra http://127.0.0.1:8080 e solte um `.kicad_pcb` (com seu `.kicad_pro`) na
página: a placa, suas verificações e análises; sem esquemático, bibliotecas ou arquivos gravados
ao lado (Salvar e os arquivos de fabricação são baixados).

Para mostrar um projeto a outras pessoas, Compartilhar (o botão de QR no alto à direita da janela)
põe o esquemático, a placa e a vista 3D numa página web, só para ver: leia o código com um celular
na sua rede, ou abra o endereço em outro computador. Nada ali edita o projeto, e a página acompanha
os arquivos conforme você salva. Sem janela, `cuprita --serve projeto.kicad_pro` faz o mesmo em
http://127.0.0.1:8080 (`--port` escolhe outra porta, `--bind 0.0.0.0` abre para a sua rede).

## macOS e Windows, pelo código
`cargo run --release -p cuprita` compila e roda.

## Onde o Cuprita guarda as coisas dele
Configurações (Ctrl+,) mostra a pasta: os projetos recentes e suas imagens, as unidades, o idioma
e a sua chave da Digi-Key (um arquivo que só você lê). Seus projetos ficam onde estão; ao lado
deles o Cuprita grava só `<arquivo>-bak` (a versão anterior a cada salvamento) e
`_autosave-<arquivo>` (até você salvar).
