# Soma

Runner liviano para Windows x64: elegí un proyecto, compilá, ejecutá y leé sus logs sin abrir un IDE. Interfaz nativa Win32 escrita en Rust y CLI JSON para agentes, sin WebView ni servicio permanente.

## Instalar

Con [Scoop](https://scoop.sh) instalado, desde una terminal normal:

```powershell
scoop bucket add soma https://github.com/MatiasFrangolini/soma
scoop install soma/soma
```

Abrí **Soma** desde Inicio o ejecutá `soma`. Los agentes usan `soma-cli help`.

Para actualizar, cerrá Soma y ejecutá:

```powershell
scoop update soma
```

Para desinstalar: `scoop uninstall soma`. Tus datos se conservan en `%USERPROFILE%\.soma`.

Soma no requiere Rust, IntelliJ, .NET ni Java para abrir su interfaz. Java/JDK y Maven son necesarios únicamente para los proyectos que los usan. Windows Terminal es opcional para abrir una terminal externa.

## Usar

**Carpeta** registra un proyecto. El preset Maven detecta el wrapper y genera una configuración inicial; también se pueden configurar comandos de otros lenguajes. **Abrir config** abre `projects.json` en el editor del sistema.

- **Compilar y Run** compila y ejecuta si termina correctamente.
- **Run build vigente** reutiliza un build registrado, comprobando rama, entradas, configuración y artefacto.
- **Build** compila sin ejecutar.
- **Stop** solicita Ctrl+C; tras 15 segundos puede finalizar únicamente su árbol de procesos.
- **Terminal** abre una terminal externa en la carpeta del proyecto.

Cerrar Soma detiene la aplicación que inició. Solo hay una ejecución a la vez. Los logs visibles se limitan a 2 MiB y el disco a tres segmentos de 10 MiB.

Cada configuración admite directorio, ejecutable/argumentos, build opcional, `.env`, variables explícitas y selección de JDK. El preset Java añade JAR, argumentos JVM, perfiles Spring y YAML externo. La precedencia es entorno del sistema → `.env` del repo → overrides explícitos. El archivo `.env` se lee al iniciar cada ejecución.

Los datos están en `%USERPROFILE%\.soma`, una ubicación estable al ejecutar desde Inicio o desde agentes empaquetados en MSIX. En el primer inicio se recuperan automáticamente la configuración y los logs de la versión 0.3.0 en `%LOCALAPPDATA%\Soma`, conservando los originales. `--data-dir` y `SOMA_HOME` permiten otra ubicación; se conserva compatibilidad con `RUNLOCAL_HOME` y carpetas portables `Soma-data`/`RunLocal-data` existentes junto al EXE. El bundle público no incluye configuraciones ni logs de ningún usuario.

## CLI para agentes

```powershell
soma-cli projects
soma-cli configs --project mi-proyecto
soma-cli run --project mi-proyecto --config local
soma-cli status
soma-cli logs --tail 200
soma-cli stop
```

La CLI abre o reutiliza la ventana. Las operaciones largas devuelven un ID; `--wait` espera el resultado real. `config-path` devuelve la ruta editable y `validate` comprueba la configuración sin imprimir valores de variables. Un estado `running` indica proceso activo; comprobá aparte la disponibilidad de la aplicación.

## Desarrollo y releases

Se compila con Rust estable, target `x86_64-pc-windows-gnu`, linker MinGW y `windres` en PATH (o `WINDRES` con su ruta). `cargo build --release --jobs 1` genera `Soma.exe` y `soma-cli.exe`, que deben quedar juntos. `Cargo.lock` fija dependencias.

`model.rs` posee configuración y vigencia; `engine.rs`, estados, logs y procesos; `worker.rs`, consola y Ctrl+C; `ipc.rs`, comunicación local; `ui.rs`, controles Win32. El refresco del visor impide reentrada de notificaciones del sistema.

El monograma S de `assets/icons/05-soma-vapor.svg` conserva las proporciones de la referencia de diseño elegida. Su ICO se incrusta en el EXE. No se cargan imágenes de presentación durante la ejecución.

[Flujo de publicación](docs/releases.md). Un tag `vX.Y.Z` dispara build, pruebas, ZIP, release y actualización del bucket de Scoop. Licencia MIT.
