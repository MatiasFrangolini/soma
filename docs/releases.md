# Releases

El repositorio incluye fuente Rust, icono, pruebas, empaquetado y bucket de Scoop. Los datos personales viven fuera de las versiones instaladas. El ZIP incluye exclusivamente los ejecutables, README, guía para agentes, licencia e información de versión/hash.

Para publicar una versión:

1. Modificá el código. Subí la versión de `Cargo.toml` y la entrada de Soma en `Cargo.lock` siguiendo `major.minor.patch`.
2. Validá los cambios y guardalos en un commit en `main`.
3. Ejecutá `pwsh ./scripts/Release.ps1`.

El script sube `main` y el tag correspondiente. GitHub Actions compila Windows x64, ejecuta las pruebas de procesos y distribución, genera un ZIP con SHA-256 y publica una release. Después actualiza `bucket/soma.json` en `main`. No se publica ningún paquete si falla la compilación o las pruebas.

Si una ejecución falla, resolvé su causa y publicá una versión nueva. No sobrescribas binarios de una versión que otras personas ya instalaron. La comprobación local de un bundle se hace con `scripts/Package.ps1 -BinaryDirectory RUTA` y `tests/distribution.ps1`; no sustituye la ejecución de CI en el runner Windows.

Los usuarios cierran Soma y ejecutan `scoop update soma`. Scoop comprueba el hash, conserva las versiones según sus reglas y actualiza los shims y el acceso del menú Inicio. Los proyectos, logs y parámetros JVM están en `%LOCALAPPDATA%\Soma` y no forman parte del ZIP. `scoop uninstall soma` retira la app y conserva esos datos.

El preset Maven general usa el wrapper del proyecto y el objetivo `package`. Un `maven_adapter` explícito puede reemplazarlo para políticas locales. No distribuyas adaptadores de una PC ni ajustes de heap específicos de un backend como valores universales.
