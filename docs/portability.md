# Configuración portable de Soma

Los ejecutables publicados contienen los mismos controles de Windows, IPC, procesos, logs y trabajos para cualquier usuario de Windows x64. No necesitan el nombre de cuenta, rutas ni herramientas de IA del desarrollador. Los cambios de código llegan a terceros cuando se compila y publica una versión nueva y esos usuarios la instalan; descargar el código fuente requiere compilarlo.

## JVM optimizada

El preset público `presets/jvm-optimized.json` contiene únicamente su ID, nombre y argumentos JVM. Está integrado en ambos ejecutables y también se incluye como JSON en el ZIP. Registrar un proyecto Maven ofrece **Local** y **JVM optimizada**. Local conserva la JVM sin ajustes adicionales; el preset se elige explícitamente en el selector de configuraciones o con `--config jvm-optimized`.

Para un proyecto ya registrado:

```powershell
soma-cli projects
soma-cli presets
soma-cli apply-preset --project ID --preset jvm-optimized
soma-cli validate
soma-cli run --project ID --config jvm-optimized
```

`apply-preset` crea una configuración nueva a partir de la primera configuración Java del proyecto; `--from-config ID` permite elegir otra. Conserva el build, JDK, `.env`, perfiles, readiness, artefacto y argumentos de aplicación de ese proyecto. Sustituye los ajustes de heap/recolector que controla el preset y conserva los demás argumentos JVM. No modifica la configuración de origen, las pruebas, otros proyectos ni adaptadores. Si ya existe `jvm-optimized`, devuelve `added:false` y conserva esa configuración, incluidas las modificaciones del usuario. La interfaz vuelve a leer las configuraciones al actualizarla o activarla.

Este preset está orientado a reducir el heap de aplicaciones de desarrollo con **Java HotSpot 21 y G1**, incluidos los JDK de distintos proveedores que utilizan HotSpot. Parte de 64 MiB y permite hasta 768 MiB de heap. El proceso Java también utiliza memoria fuera del heap; 768 MiB no es un límite de RAM total. Un proyecto que necesita más heap debe ajustar `-Xmx` en su propia configuración o usar Local. No se aplican estos parámetros a Maven ni a las JVM de tests, y no se promete una mejora de rendimiento universal.

Los ratios de heap y el GC periódico siguen las opciones documentadas de [Java 21](https://docs.oracle.com/en/java/javase/21/docs/specs/man/java.html) y [G1](https://docs.oracle.com/en/java/javase/21/gctuning/garbage-first-g1-garbage-collector1.html). El GC periódico se considera a intervalos mínimos de 30 segundos; no garantiza liberar RAM cada 30 segundos.

## Lo que cada persona configura

- **Proyecto:** registrar su propio checkout con Carpeta o `add --path`. Los artefactos, inputs y `.env` relativos se resuelven desde ese checkout, también en worktrees.
- **JDK:** al registrar Maven, Soma utiliza un `JAVA_HOME` válido de esa PC. Sin él, la ejecución usa `java.exe` del `PATH`. No busca una distribución por el nombre de carpeta de un proveedor. Es responsabilidad del proyecto elegir una versión compatible; `jdk` permite indicar una instalación concreta.
- **Entorno y proveedores:** cada persona aporta sus `.env`, credenciales, puertos y servicios. El preset no incluye valores de otro proyecto ni activa un perfil Spring por su cuenta.
- **Políticas locales:** `maven_adapter` y `job_adapter` son opcionales. El Wrapper del repositorio funciona sin scripts personales. Una política de recursos de una organización puede compartir su propio adaptador, con sus dependencias y configuración documentadas.

El ZIP contiene archivos de una lista explícita: ejecutables, documentación, licencia, actualizador, hashes y el preset público. `projects.json`, `.env`, logs, JDK, proyectos y adaptadores personales quedan en la PC de cada usuario. Para compartir los ajustes JVM, usá el preset y su comando; un `projects.json` completo también puede contener rutas, proveedores y secretos de la persona que lo creó.

## Compilar desde un clon nuevo

El proyecto compila para Windows x64 con Rust y el target `x86_64-pc-windows-gnu`. Se necesitan las herramientas MinGW de GCC y binutils; sus binarios deben estar en el `PATH`. Configurá `WINDRES` con la ruta del `windres.exe` de tu instalación. Los pasos completos y versiones fijadas de las herramientas de CI están en `.github/workflows/release.yml`.

```powershell
rustup target add x86_64-pc-windows-gnu
$env:WINDRES = (Get-Command windres.exe -ErrorAction Stop).Source
cargo build --locked --release --jobs 1 --target x86_64-pc-windows-gnu
cargo test --locked --release --jobs 1 --target x86_64-pc-windows-gnu
pwsh ./scripts/Package.ps1 -BinaryDirectory ./target/x86_64-pc-windows-gnu/release
```

Estos comandos no dependen del toolchain privado del desarrollador. Las pruebas nativas de `tests/` reciben rutas de JDK y directorios descartables como argumentos. Si tu organización usa un launcher de recursos, ejecutá las validaciones a través de su política local.

## Alcance verificado

Las pruebas de distribución arrancan instalaciones con otro `USERPROFILE`, otra ruta de JDK y un proyecto con espacios y `&`. Comprueban la configuración recién creada, la aplicación del preset a un proyecto existente, la conservación de ajustes personalizados, la ejecución de un JAR real y el contenido del ZIP. Esto verifica esos escenarios de Windows x64; no certifica todos los equipos, versiones de Windows ni JVM alternativas.

Todas las personas que instalen una misma release reciben esas capacidades de los ejecutables y el mismo preset público. Una actualización conserva sus proyectos y ajustes personales; en un proyecto ya registrado se agrega el preset con `apply-preset`. Actualizar un clon de código fuente o cambiar un JSON en otra PC no actualiza una instalación existente: hace falta compilar o instalar la nueva release.
