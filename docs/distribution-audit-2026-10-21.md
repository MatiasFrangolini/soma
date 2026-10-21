# Revisión de distribución - 21 de octubre de 2026

Registro de la revisión inicial del checkout basado en `1b7e29b`, cuando la última release era `v0.5.2` (21/10/2026, 14:41 UTC). La tabla conserva ese estado previo a la integración. Después se incorporó la observabilidad a la candidata 0.6.0 junto con el ciclo de vida CLI y los presets. La publicación requiere validar el conjunto y el CI del tag.

| Cambio o configuración | ¿Lo recibe otro usuario? | Situación comprobada |
| --- | --- | --- |
| Interfaz Win32, control de árboles de procesos, logs acotados, worktrees, readiness y trabajos Maven | Sí, con la release que los contiene | Código común, sin rutas de la PC del desarrollador. |
| Recuperación de IPC desde agentes Windows aislados | Sí, desde 0.5.2 | Forma parte del código publicado. Requiere el mecanismo autorizado del anfitrión cuando Windows deniega el canal. |
| CLI sin ventanas, reutilización y cierre de dueños temporales | Al publicar e instalar una nueva release | Implementado y probado localmente; todavía fuera de la descarga pública. |
| JVM optimizada | Al publicar e instalar una nueva release | Ahora es un preset público, integrado en los ejecutables y en el ZIP. Antes vivía sólo en una configuración personal. |
| Selección de JDK | Usa los datos de cada PC | Se eliminó la búsqueda por el nombre `ms-21`; usa `JAVA_HOME` válido o Java del `PATH`. Se mantienen las instalaciones explícitas de proyectos existentes. |
| Proyectos, `.env`, perfiles Spring, JDK y credenciales | Cada usuario aporta los suyos | No pertenecen al paquete público. El preset clona la configuración del proyecto local. |
| Adaptadores Maven y política local de recursos | Sólo si se configuran en esa PC | Son opcionales. El Wrapper funciona sin los scripts personales del desarrollador. |
| Logs por operación, cursores y consulta de salud actual | Todavía no | Se confirmó otro worktree con cambios sin commit ni integración. No se incluyeron silenciosamente en este paquete. |

## Cambios realizados

- `presets/jvm-optimized.json` define los parámetros públicos, con el nombre **JVM optimizada**. El preset conserva el heap inicial de 64 MiB, máximo de 768 MiB y los ratios/intervalo utilizados en la configuración personal, haciendo explícito G1. No incorpora rutas, Spring, proveedores ni ajustes Maven personales.
- Los proyectos Maven nuevos ofrecen Local y JVM optimizada. `apply-preset` permite agregar la opción a los ya existentes sin sobrescribir configuraciones, pruebas ni adaptadores.
- El paquete incorpora el preset y `PORTABILITY.md` mediante una lista explícita. Los perfiles personales y archivos de entorno siguen fuera.
- La guía portable documenta dependencias, JDK por PC, límites del preset y compilación desde un clon nuevo. Esta revisión de estado no se incluye en el ZIP.

## Evidencia y límites

La prueba anterior reprodujo la ausencia del preset en una instalación limpia. La candidata compila y pasa los tests Rust y las pruebas de distribución/ciclo de vida: perfil nuevo, migración, otro path de JDK, Java únicamente en PATH, JAR Java 21 real, preservación de ajustes personalizados, reutilización concurrente de dueño y cierre automático. Las pruebas usan directorios descartables y los ejecutables empaquetados.

El perfil de usuario se simula mediante variables de entorno y otra ruta de JDK mediante una junction al JDK instalado. Es evidencia de independencia de rutas y configuración, no una ejecución en una segunda PC física, otro SID de Windows ni otro proveedor de JVM. No se certifican todas las versiones de Windows ni se atribuyen mejoras de memoria/rendimiento medidas a este preset. El heap observado por el JAR confirma el límite configurado; no mide RAM total.

## Resolución en la candidata 0.6.0

La observabilidad se integró con el ciclo de vida CLI y los presets, conservando el worktree de origen. La versión se fijó en 0.6.0. El flujo de publicación valida el conjunto, ejecuta CI del commit/tag y genera la release; los usuarios luego actualizan Soma y agregan el preset a sus proyectos existentes. Sus JSON personales se conservan.
