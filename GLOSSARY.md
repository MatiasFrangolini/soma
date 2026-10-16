# Soma

Herramienta local para ejecutar aplicaciones y coordinar trabajos de compilación y pruebas entre usuarios y agentes.

## Lenguaje

**Proyecto**:
Aplicación identificada por un directorio local y sus configuraciones. Sus worktrees pertenecen al mismo proyecto.

**Worktree**:
Checkout de un proyecto con su propio directorio de archivos y estado de Git.

**Configuración de ejecución**:
Selección con nombre que reúne las condiciones necesarias para iniciar una aplicación.
_Evitar_: Perfil, cuando pueda confundirse con un perfil Spring.

**Configuración de pruebas**:
Selección con nombre que reúne las condiciones necesarias para compilar o probar un proyecto, independiente de la configuración con la que se ejecuta su aplicación.

**Perfil Spring**:
Grupo de configuración de una aplicación Spring que se activa durante su ejecución, como `local`.

**Ejecución**:
Instancia de una aplicación iniciada con una configuración de ejecución, hasta que termina o se detiene.

**Trabajo**:
Validación o compilación finita, con identidad, estado y resultado propios. Puede estar solicitada por varios agentes.
_Evitar_: Ejecución, cuando se refiere a la aplicación que permanece funcionando.

**Trabajo equivalente**:
Trabajo solicitado para el mismo checkout, las mismas entradas y las mismas condiciones de compilación o pruebas. Mientras está pendiente o activo puede satisfacer varias solicitudes.

**Participación**:
Interés de un usuario o agente en un trabajo compartido. Retirar una participación conserva el trabajo mientras otra participación lo necesite.

**Validación incompleta**:
Trabajo de pruebas cuya evidencia no permite confirmar la validación solicitada, aunque la herramienta haya finalizado correctamente.

**Resultado desactualizado**:
Resultado de un trabajo cuyas entradas cambiaron durante su preparación o ejecución y que requiere una validación nueva.

**Resultado de trabajo**:
Evidencia de lo que ocurrió durante un trabajo, vinculada a las entradas y condiciones que se probaron. Incluye su finalización, las pruebas observadas y las duraciones medidas.

**Build vigente**:
Resultado de una compilación exitosa que todavía corresponde a las entradas y condiciones actuales del proyecto.

**Logs de ejecución**:
Salida de una aplicación durante su ejecución.

**Logs de trabajo**:
Salida producida durante una compilación o validación finita.
