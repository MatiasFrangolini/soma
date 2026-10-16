# Soma

Runner Windows nativo en Rust. GUI y CLI comparten una única autoridad de ejecución mediante IPC local. Usá `soma-cli help` y sus respuestas JSON para operar la app.

`model.rs` posee configuración, entorno y vigencia de builds; `engine.rs`, operaciones, logs y árboles de procesos; `worker.rs`, consola y Ctrl+C; `ipc.rs`, protocolo; `ui.rs`, controles Win32. Conservá las colas acotadas y el control de reentrada del visor.

La precedencia de variables es sistema → `.env` → overrides explícitos. Los datos personales viven en `%USERPROFILE%\.soma`, salvo configuración portable existente, `SOMA_HOME` o `--data-dir`. Conservá entradas ajenas al editar `projects.json`; ejecutá `validate` después.

Para validar cambios de ejecución, `tests/verify.ps1` ejercita la CLI con JVMs reales descartables. `tests/distribution.ps1` verifica almacenamiento entre versiones y el ZIP público. Respetá los controles de recursos de la máquina local; CI compila y prueba de forma secuencial.

Para publicar, leé `docs/releases.md`. El bundle usa una lista explícita de archivos; perfiles, `.env`, logs, adaptadores personales y datos de usuarios quedan fuera. La versión se declara en `Cargo.toml` y su entrada de `Cargo.lock`. El tag debe coincidir con ella.
