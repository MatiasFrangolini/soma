# Soma 0.7.1

Esta versión mejora el uso de la CLI por agentes, el diagnóstico de trabajos y la conservación de evidencia. El candidato fue validado localmente antes de publicar un único tag y paquete.

- Resúmenes opcionales de trabajos con veredicto, checkout, selección, conteos y siguiente acción. El resultado completo y sus códigos de salida siguen disponibles; una consulta exitosa no acredita tests.
- Filtros de lista por checkout y estado, orden por fecha y límite de resultados. Ayuda compatible con `jobs help`, `version` y `--version` sin iniciar Soma ni crear datos.
- Lectura incremental y filtros en logs de trabajos, con cursores verificables, vencimiento explícito y disponibilidad histórica separada del progreso actual.
- Preparación instrumentada por reactor, configuración, entorno, adaptador, Git y lectura de entradas. Se conservan las comprobaciones de vigencia anteriores y posteriores a Maven.
- Preflight de configuraciones de tests y clonado transaccional hacia un ID nuevo o un entorno aislado, preservando el adaptador de recursos y las entradas ajenas.
- Archivo de metadata independiente de los logs: 100 resultados detallados y hasta 1000 registros compactos, 16 MiB y 30 días. Lectura por ID y exportación como JSON; no reutiliza automáticamente validaciones verdes.
- El archivo reserva espacio antes de retirar evidencia y comunica errores de almacenamiento conservando los resultados detallados. El resumen distingue fallos de omisiones y los clones compartidos conservan su selección correcta entre worktrees.
- Guía para agentes actualizada para usar esperas no bloqueantes y evitar consultas repetidas sin progreso.

La concurrencia configurada y los límites de procesos no se incrementan. La publicación, el tag y la actualización de Scoop se realizan al cerrar la entrega mediante el flujo de [releases](releases.md).
