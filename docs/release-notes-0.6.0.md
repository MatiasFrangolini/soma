# Soma 0.6.0

- **CLI sin ventanas.** Reutiliza el dueño del directorio de datos y cierra los dueños temporales cuando terminan la aplicación y los trabajos pendientes. `show` abre la interfaz del mismo proceso.
- **Logs por ejecución.** Consultá una operación, seguí su salida mediante cursores y filtrá texto literal. Las respuestas indican si el historial quedó truncado.
- **Salud actual.** `status --operation ID --health` comprueba el endpoint local configurado y verifica que pertenezca a esa ejecución. La lectura no reinicia aplicaciones.
- **Resultados después del cierre.** Los IDs, resultados finales y logs siguen disponibles después de que termina el dueño temporal.
- **JVM optimizada.** Preset público para Java HotSpot 21/G1, con heap inicial de 64 MiB y máximo de 768 MiB. Se ofrece junto a Local y puede agregarse a proyectos existentes sin sobrescribir sus ajustes.
- **Configuración portable.** Java usa el `JAVA_HOME` o `PATH` de cada persona. El ZIP incluye el preset y las guías; los perfiles y adaptadores personales quedan en cada PC.
- **README renovado.** Instrucciones breves de instalación, primeros pasos, CLI y actualización, con referencia completa en `CLI.md`.

Actualizá con `soma-update` después de cerrar tus ejecuciones y Soma. Para un proyecto ya registrado, ejecutá `soma-cli apply-preset --project ID --preset jvm-optimized`.
