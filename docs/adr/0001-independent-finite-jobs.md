---
status: accepted
---

# Trabajos finitos independientes de la aplicación

Soma mantendrá la ejecución del backend y añadirá trabajos finitos con identidad y resultados propios, operados inicialmente por CLI. Maven continuará siendo la herramienta del proyecto; un adaptador personal conservará el launcher local, y Soma iniciará un único trabajo Maven a la vez por defecto para limitar RAM. La distribución pública conservará su independencia de herramientas de agentes y de políticas de una PC.

Las solicitudes equivalentes compartirán únicamente trabajos pendientes o activos del mismo checkout y condiciones. Los resultados finalizados permanecerán como evidencia consultable; una nueva solicitud volverá a validar, evitando que un resultado histórico sustituya silenciosamente una prueba nueva.

Cada solicitante tendrá su propia participación: retirarla solo cancelará un trabajo pendiente o activo cuando ya no quede ninguna. Cerrar la ventana cancelará los trabajos de Soma y su aplicación, conservando el resultado terminal; un cierre imprevisto dejará evidencia de interrupción. El historial tendrá hasta 100 trabajos finalizados y hasta 30 MiB de logs de trabajos en total, adicionales al presupuesto de logs del backend.

Para permitir pruebas en el checkout de una aplicación Java activa, un JAR autocontenido se ejecutará desde una copia inmutable. Los comandos genéricos que dependan de archivos mutables conservarán la exclusión del checkout. Cada trabajo usará una configuración de pruebas propia, sin trasladar automáticamente parámetros JVM ni variables del backend.

La finalización de Maven y la evidencia de pruebas serán resultados distintos. La ausencia de pruebas ejecutadas, los reportes incompletos y las entradas que cambien impedirán presentar la validación como confirmada. Las omisiones se mostrarán explícitamente; una compilación podrá finalizar correctamente sin pruebas.
