# Soma

Runner liviano para Windows x64: elegí un proyecto, compilá, ejecutá y leé sus logs sin abrir un IDE. Interfaz nativa Win32 escrita en Rust, sin WebView ni servicio permanente. Se usa desde la ventana; la CLI JSON es opcional.

## Instalar

Con [Scoop](https://scoop.sh) instalado, desde una terminal normal:

```powershell
scoop bucket add soma https://github.com/MatiasFrangolini/soma
scoop install soma/soma
```

Abrí **Soma** desde Inicio o ejecutá `soma`. Los agentes usan `soma-cli help`.

Para actualizar, cerrá Soma y ejecutá:

```powershell
soma-update
```

Para desinstalar: `scoop uninstall soma`. Tus datos se conservan en `%USERPROFILE%\.soma`.

Soma no requiere Codex, IA, cuentas de servicios, Rust, IntelliJ, .NET ni Java para abrir su interfaz. Java/JDK y Maven son necesarios únicamente para los proyectos que los usan. Windows Terminal es opcional para abrir una terminal externa.

`soma-update` refresca el catálogo de Scoop y actualiza únicamente Soma. También podés ejecutar `scoop update` seguido de `scoop update soma`. El script termina al completar la operación; no hay actualizador residente.

## Usar

**Carpeta** registra un proyecto. El preset Maven detecta el wrapper y genera una configuración inicial; también se pueden configurar comandos de otros lenguajes. **Abrir config** abre `projects.json` en el editor del sistema.

**Worktree** muestra los checkouts existentes de Git y selecciona el directorio que se ejecutará. **Rama** permite cambiar a una rama local disponible dentro del worktree elegido. Soma conserva los cambios locales: el cambio de rama requiere un checkout limpio y nunca hace stash, reset ni un checkout forzado. Si otra ventana de Soma usa ese directorio o Git tiene la rama en otro worktree, el cambio se rechaza. Los selectores se bloquean durante la ejecución.

- **Compilar y Run** compila y ejecuta si termina correctamente.
- **Run build vigente** reutiliza un build registrado, comprobando rama, entradas, configuración y artefacto.
- **Build** compila sin ejecutar.
- **Stop** solicita Ctrl+C; tras 15 segundos puede finalizar únicamente su árbol de procesos.
- **Terminal** abre una terminal externa en la carpeta del proyecto.

Cerrar Soma detiene la aplicación que inició. Solo hay una ejecución a la vez. Los logs visibles se limitan a 2 MiB y el disco a tres segmentos de 10 MiB.

Cada configuración admite directorio, ejecutable/argumentos, build opcional, `.env`, variables explícitas y selección de JDK. El preset Java añade JAR, argumentos JVM, perfiles Spring y YAML externo. La precedencia es entorno de la ventana de Soma → `.env` del checkout → overrides explícitos. El archivo `.env` se lee al iniciar cada ejecución. Cambiar variables en una terminal no modifica el entorno de una ventana ya abierta; usá el perfil JSON para los overrides.

Al elegir un worktree, los paths relativos y los paths absolutos que pertenecen al proyecto se resuelven en el nuevo checkout, incluyendo el directorio del build. Los parámetros externos, como el JDK instalado, conservan su ubicación. Los builds se registran por proyecto, configuración y directorio; un JAR de otro worktree no satisface `--reuse`.

Los datos están en `%USERPROFILE%\.soma`, una ubicación estable al ejecutar desde Inicio o desde agentes empaquetados en MSIX. En el primer inicio se recuperan automáticamente la configuración y los logs de la versión 0.3.0 en `%LOCALAPPDATA%\Soma`, conservando los originales. `--data-dir` y `SOMA_HOME` permiten otra ubicación; se conserva compatibilidad con `RUNLOCAL_HOME` y carpetas portables `Soma-data`/`RunLocal-data` existentes junto al EXE. El bundle público no incluye configuraciones ni logs de ningún usuario.

## CLI para agentes

```powershell
soma-cli projects
soma-cli configs --project mi-proyecto
soma-cli worktrees --path C:\repos\mi-proyecto
soma-cli switch --path C:\repos\mi-worktree --branch mi-rama
soma-cli run --path C:\repos\mi-worktree --config local
soma-cli status
soma-cli logs --tail 200
soma-cli wait --operation ID_DEVUELTO --ready --timeout 180
soma-cli stop --operation ID_DEVUELTO
```

La CLI abre o reutiliza la ventana. `--path` identifica el checkout exacto de un proyecto registrado, incluidos sus worktrees. `--project ID` conserva el uso anterior y toma la selección de la ventana. `status` incluye directorio, rama, commit y estado de readiness. `stop --operation ID` rechaza un ID distinto del actual; Stop manual desde la ventana sigue disponible.

`--wait` espera la finalización del proceso. Para servidores, `run --wait-ready` o `wait --operation ID --ready` espera la API sin esperar que termine Java. `config-path` devuelve la ruta editable y `validate` comprueba la configuración sin imprimir valores de variables.

La configuración opcional de readiness usa HTTP/HTTPS local con la API nativa de Windows:

```json
"readiness": {
  "url": "http://127.0.0.1:8080/actuator/health/readiness",
  "timeout_seconds": 120,
  "json_field": "status",
  "expected": "UP"
}
```

Con `json_field` omitido basta una respuesta 2xx. El puerto debe pertenecer a un proceso de esa ejecución; otro servidor saludable no puede dar un positivo. Solo se consulta durante el arranque y se limita el cuerpo a 16 KiB. Un timeout devuelve error y deja el proceso activo para consultar sus logs o detenerlo con su ID. También se puede pasar `--health-url URL` para un endpoint con JSON `status=UP`. La URL admite localhost o IP de loopback; no sigue redirecciones ni desactiva la validación de certificados HTTPS.

Para instancias independientes usá `--data-dir` con perfiles propios. Una ejecución protege su directorio frente a builds y cambios de rama desde otras ventanas de Soma. Las compilaciones y pruebas específicas pueden seguir usando directamente las herramientas del proyecto y sus controles locales de recursos.

## Desarrollo y releases

Se compila con Rust estable, target `x86_64-pc-windows-gnu`, linker MinGW y `windres` en PATH (o `WINDRES` con su ruta). `cargo build --release --jobs 1` genera `Soma.exe` y `soma-cli.exe`, que deben quedar juntos. `Cargo.lock` fija dependencias.

`model.rs` posee configuración y vigencia; `git.rs`, checkouts y protección del directorio; `readiness.rs`, la comprobación HTTP local; `engine.rs`, estados, logs y procesos; `worker.rs`, consola y Ctrl+C; `ipc.rs`, comunicación local; `ui.rs`, controles Win32. El refresco del visor impide reentrada de notificaciones del sistema.

El monograma S de `assets/icons/05-soma-vapor.svg` conserva las proporciones de la referencia de diseño elegida. Su ICO se incrusta en el EXE. No se cargan imágenes de presentación durante la ejecución.

[Flujo de publicación](docs/releases.md). Un tag `vX.Y.Z` dispara build, pruebas, ZIP, release y actualización del bucket de Scoop. Licencia MIT.
