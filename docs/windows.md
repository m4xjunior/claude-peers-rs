# Cliente claude-peers en Windows

Guía para conectar una máquina **Windows** a la red claude-peers. Dos partes:
(A) generar el binario `peers-client.exe` (una vez, desde macOS/Linux), y
(B) instalarlo como MCP en el Claude Code de Windows.

El broker sigue siendo central (p. ej. el Mac de Max en la LAN); Windows es solo un cliente
más que se conecta a ese broker.

---

## A. Generar `peers-client.exe` (cross-compilar desde macOS)

Rust cross-compila a Windows sin necesidad de una máquina Windows. Usamos `cargo-xwin`
(SDK de MSVC, sin mingw). El cliente usa `reqwest` con `rustls` → sin OpenSSL nativo.

```bash
# 1. herramientas (una vez)
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
brew install llvm            # aporta llvm-lib, que necesita el crate `ring` (de rustls)

# 2. compilar el cliente para Windows
PATH="/opt/homebrew/opt/llvm/bin:$PATH" \
  cargo xwin build --release --target x86_64-pc-windows-msvc -p peers-client
# → target/x86_64-pc-windows-msvc/release/peers-client.exe  (PE32+ x86-64)
```

> **Portabilidad:** el cliente separa por plataforma con `#[cfg(unix)]`/`#[cfg(windows)]` las
> APIs que no existen en Windows (señales SIGINT/SIGTERM → `ctrl_c()`, y `tty()` → `None`).
> No hay que tocar nada: compila para los tres SO desde el mismo código.

### Distribuir el `.exe`
No se versiona en git (`/target` está en `.gitignore`). Vías para llevarlo a Windows:
- **GitHub Releases** (recomendado para el producto).
- **Servidor HTTP temporal** desde el Mac (rápido para pruebas), y descargar con PowerShell:
  ```bash
  # en el Mac, sirviendo la carpeta del .exe:
  cd target/x86_64-pc-windows-msvc/release && python3 -m http.server 8899 --bind 0.0.0.0
  ```
- Carpeta compartida / pendrive.

> `scp` requiere un servidor SSH en el Windows (OpenSSH Server, que no viene activo por
> defecto). Si el puerto 22 del Windows está cerrado, usa HTTP o carpeta compartida.

---

## B. Instalar el cliente en Windows (PowerShell)

Con Claude Code ya instalado en la máquina Windows. Todos los pasos en **PowerShell**.

### 1. Colocar el `.exe`
```powershell
mkdir C:\peers -Force
# ejemplo con descarga por HTTP desde el Mac (ajusta la IP/puerto):
Invoke-WebRequest -Uri "http://10.0.1.60:8899/peers-client.exe" -OutFile "C:\peers\peers-client.exe"
```

### 2. Verificar que Windows alcanza el broker
```powershell
Test-NetConnection 10.0.1.60 -Port 7899
```
Debe dar **`TcpTestSucceeded : True`**. Si es `False`: firewall del equipo del broker o ruteo
entre subredes (Windows y broker en `10.0.5.x` vs `10.0.1.x` → distinta subred, requiere ruta).

### 3. Registrar el MCP (scope user / global)
```powershell
claude mcp add claude-peers -s user -e CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899 -e CLAUDE_PEERS_TOKEN=lexusfx-peers-2026 -- C:\peers\peers-client.exe
```

> **Sintaxis:** Claude Code ≥ 2.1.x usa `-s` (scope) y `-e` (env). Versiones viejas usaban
> `--scope`/`--env`. Ruta con `\` y sin espacios (o entre comillas si los tiene).

### 4. Verificar la conexión
```powershell
claude mcp list
```
Debe aparecer `claude-peers: ... - ✔ Connected`. Eso confirma que el `.exe` conecta al broker.

### 5. Arrancar Claude con el flag de canal (OBLIGATORIO para el push)
```powershell
cd C:\ruta\del\proyecto
claude --dangerously-load-development-channels server:claude-peers
```
Sin este flag, los mensajes llegan pero **no** se renderizan como `<channel>`. En Windows el
flag va igual en la línea de comandos (no hay env ni setting equivalente).

---

## Diferencias Windows vs Unix (resumen)

| Aspecto | Unix (macOS/Linux) | Windows |
|--------|--------------------|---------|
| Env antes de `claude` | `export VAR=...` | `$env:VAR="..."` (o mejor: `-e` en `claude mcp add`) |
| Ruta del cliente | `/usr/local/bin/peers-client` | `C:\peers\peers-client.exe` |
| Config MCP | `~/.claude.json` | `%USERPROFILE%\.claude.json` |
| Transferir el binario | `scp` | HTTP / carpeta compartida (SSH suele estar cerrado) |
| Flag de canal | igual | igual (obligatorio, por CLI) |
| tty / señales | nativo | `tty()`=None, `ctrl_c()` en vez de SIGTERM |

El resto (tools, tareas, jornada) es idéntico en las tres plataformas.
