# Releases

El repositorio incluye fuente Rust, icono, pruebas, empaquetado y bucket de Scoop. Los datos personales viven fuera de las versiones instaladas. El ZIP incluye exclusivamente los ejecutables, README, guía para agentes, licencia, guía de portabilidad, preset JVM público e información de versión/hash.

Para publicar una versión:

1. Modificá el código. Subí la versión de `Cargo.toml` y la entrada de Soma en `Cargo.lock` siguiendo `major.minor.patch`.
2. Validá los cambios y guardalos en un commit en `main`.
3. Ejecutá `pwsh ./scripts/Release.ps1`.

El script sube `main` y el tag correspondiente. GitHub Actions compila Windows x64, ejecuta los tests Rust y las pruebas de procesos, distribución, worktrees, readiness y trabajos Maven reales, genera un ZIP con SHA-256 y publica una release. Después actualiza `bucket/soma.json` en `main`. No se publica ningún paquete si falla la compilación o las pruebas. Conviene esperar el CI de `main` antes de crear el tag.

Para una entrega ya validada en su rama, el commit final de `main` puede incluir `[release-only]`: ese push no consume un build duplicado de `main`, pero el workflow del tag sigue ejecutando todas las comprobaciones antes de publicar. No uses la marca sin verificar previamente la rama y crear inmediatamente el tag de la misma versión.

Si una ejecución falla, resolvé su causa y publicá una versión nueva. No sobrescribas binarios de una versión que otras personas ya instalaron. La comprobación local de un bundle se hace con `scripts/Package.ps1 -BinaryDirectory RUTA` y `tests/distribution.ps1`; no sustituye la ejecución de CI en el runner Windows.

Los usuarios cierran Soma y ejecutan `soma-update`. Scoop comprueba el hash, conserva las versiones según sus reglas y actualiza los shims y el acceso del menú Inicio. Los proyectos, logs y parámetros JVM están en `%USERPROFILE%\.soma` y no forman parte del ZIP. `scoop uninstall soma` retira la app y conserva esos datos.

El preset Maven general usa el wrapper del proyecto y el objetivo `package`. `JVM optimizada` es una configuración opcional y portable con límites documentados en `docs/portability.md`. Un `maven_adapter` explícito puede reemplazar el build para políticas locales del runtime; `job_adapter` admite la política de trabajos independientes. Distribuí únicamente el preset público; los adaptadores personales, `projects.json` y datos de un backend quedan fuera del paquete.
