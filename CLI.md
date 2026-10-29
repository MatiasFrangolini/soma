# Soma

Guía de referencia. Para instalar y empezar, leé [README.md](README.md).

Runner liviano para Windows x64: elegí un proyecto, compilá, ejecutá y leé sus logs sin abrir un IDE. Interfaz nativa Win32 escrita en Rust, sin WebView ni servicio permanente. Se usa desde la ventana; la CLI JSON es opcional.

## Instalar

Con [Scoop](https://scoop.sh) instalado, desde una terminal normal:

```powershell
scoop bucket add soma https://github.com/MatiasFrangolini/soma
scoop install soma/soma
```

Abrí **Soma** desde Inicio o ejecutá `soma`. Los agentes usan `soma-cli help`.

Para actualizar, cerrá Soma y ejecutá:

```powershell
soma-update
```

Para desinstalar: `scoop uninstall soma`. Tus datos se conservan en `%USERPROFILE%\.soma`.

Soma no requiere Codex, IA, cuentas de servicios, Rust, IntelliJ, .NET ni Java para abrir su interfaz. Java/JDK y Maven son necesarios únicamente para los proyectos que los usan. Windows Terminal es opcional para abrir una terminal externa.

`soma-update` refresca el catálogo de Scoop y actualiza únicamente Soma. También podés ejecutar `scoop update` seguido de `scoop update soma`. El script termina al completar la operación; no hay actualizador residente.

## Usar

**Carpeta** registra un proyecto. El preset Maven detecta el wrapper y genera una configuración inicial; también se pueden configurar comandos de otros lenguajes. **Abrir config** abre `projects.json` en el editor del sistema.

**Worktree** muestra los checkouts existentes de Git y selecciona el directorio que se ejecutará. **Rama** permite cambiar a una rama local disponible dentro del worktree elegido. Soma conserva los cambios locales: el cambio de rama requiere un checkout limpio y nunca hace stash, reset ni un checkout forzado. Si otra ventana de Soma usa ese directorio o Git tiene la rama en otro worktree, el cambio se rechaza. Los selectores se bloquean durante la ejecución.

- **Compilar y Run** compila y ejecuta si termina correctamente.
- **Run build vigente** reutiliza un build registrado, comprobando rama, entradas, configuración y artefacto.
- **Build** compila sin ejecutar.
- **Stop** solicita Ctrl+C; tras 15 segundos puede finalizar únicamente su árbol de procesos.
- **Terminal** abre una terminal externa en la carpeta del proyecto.

Cerrar Soma detiene la aplicación y los trabajos que inició. Hay una aplicación por dueño y hasta `job_concurrency` trabajos Maven activos, en checkouts distintos, tenga o no interfaz abierta. Los logs visibles del backend se limitan a 2 MiB y el disco a tres segmentos de 10 MiB.

La CLI distingue una ventana cerrada de un canal local inaccesible. Si Windows deniega la conexión, devuelve `ok:false`, `code:"access_denied"` y `os_error:5`; no abre otra instancia, no informa `closed`/`stopped` y no reemplaza la consulta por datos históricos. Otros errores de conexión devuelven `ipc_unavailable`. Solo la ausencia del canal permite abrir Soma o consultar el historial offline. Ejecutá la CLI desde un contexto autorizado para acceder al dueño existente; no es necesario reiniciarlo ni ampliar los permisos del canal.

En agentes Windows aislados, la integridad baja del proceso puede impedir escribir en el canal de un Soma iniciado con integridad media, incluso usando la misma cuenta. Consultar `status` o `logs` también escribe la solicitud IPC. Para ese caso, `next_action:"request_authorized_execution"` indica que el agente debe usar la ejecución autorizada del anfitrión para el comando exacto y comprobar su JSON. Soma no se eleva automáticamente ni abre un canal alternativo. En Codex se usa `exec_command` con `sandbox_permissions:"require_escalated"`, sujeto a la política de aprobación vigente. Las instrucciones de integración deben aplicar esto desde la primera llamada IPC; cambiar solo el mensaje de error no da acceso al agente.

Cada configuración admite directorio, ejecutable/argumentos, build opcional, `.env`, variables explícitas y selección de JDK. El preset Java añade JAR, argumentos JVM, perfiles Spring y YAML externo. La precedencia es entorno del proceso dueño de Soma → `.env` del checkout → overrides explícitos. El archivo `.env` se lee al iniciar cada ejecución. Cambiar variables en una terminal no modifica el entorno de un dueño ya iniciado; usá el perfil JSON para los overrides.

Registrar un proyecto Maven ofrece **Local** y **JVM optimizada**. La segunda opción comparte los ajustes de heap reducido para Java HotSpot 21/G1 y conserva las rutas y el entorno de tu proyecto. En proyectos existentes, agregala con `soma-cli apply-preset --project ID --preset jvm-optimized`. El preset JSON público, sus límites y la configuración de cada PC se explican en [Configuración portable](docs/portability.md); en el ZIP, leé `PORTABILITY.md`.

Al elegir un worktree, los paths relativos y los paths absolutos que pertenecen al proyecto se resuelven en el nuevo checkout, incluyendo el directorio del build. Los parámetros externos, como el JDK instalado, conservan su ubicación. Los builds se registran por proyecto, configuración y directorio; un JAR de otro worktree no satisface `--reuse`.

Los datos están en `%USERPROFILE%\.soma`, una ubicación estable al ejecutar desde Inicio o desde agentes empaquetados en MSIX. En el primer inicio se recuperan automáticamente la configuración y los logs de la versión 0.3.0 en `%LOCALAPPDATA%\Soma`, conservando los originales. `--data-dir` y `SOMA_HOME` permiten otra ubicación; se conserva compatibilidad con `RUNLOCAL_HOME` y carpetas portables `Soma-data`/`RunLocal-data` existentes junto al EXE. El bundle público no incluye configuraciones ni logs de ningún usuario.

## CLI para agentes

```powershell
soma-cli projects
soma-cli configs --project mi-proyecto
soma-cli worktrees --path C:\repos\mi-proyecto
soma-cli switch --path C:\repos\mi-worktree --branch mi-rama
soma-cli run --path C:\repos\mi-worktree --config local
soma-cli status
soma-cli logs --tail 200
soma-cli wait --operation ID_DEVUELTO --ready --timeout 180
soma-cli stop --operation ID_DEVUELTO
```

La CLI reutiliza el dueño del mismo directorio de datos. Si no existe, inicia uno temporal sin crear ventanas. Al terminar la aplicación y vaciarse la cola de trabajos, ese dueño se cierra después de dos segundos ociosos; consultar estado o logs no impide el cierre. `soma-cli show` o abrir Soma manualmente muestra la interfaz del mismo proceso y la conserva hasta cerrarla explícitamente. Una ventana abierta por el usuario no se cierra automáticamente.

`--path` identifica el checkout exacto de un proyecto registrado, incluidos sus worktrees. `--project ID` conserva la selección del dueño existente. `status` incluye directorio, rama, commit y estado de readiness; `owner.pid` y `owner.temporary` identifican el dueño. `stop --operation ID` rechaza un ID distinto del actual; Stop manual desde la ventana sigue disponible.

`--wait` espera la finalización del proceso. Para servidores, `run --wait-ready` o `wait --operation ID --ready` espera la API sin esperar que termine Java. `config-path` devuelve la ruta editable y `validate` comprueba la configuración sin imprimir valores de variables.

### Logs por operación y salud actual

```powershell
soma-cli status --operation ID --health
soma-cli logs --operation ID --tail 100
soma-cli logs --operation ID --after CURSOR --contains ERROR
```

`readiness` conserva la comprobación del arranque y `readiness_checked_at_ms` su fecha. `--health` comprueba el endpoint configurado en ese momento; `health.checked_now` distingue esa consulta de una respuesta histórica. Un ID reemplazado se rechaza. La comprobación no inicia, detiene ni reinicia procesos.

Los cursores son opacos y están ligados al historial y a su alcance global o por operación. Sobreviven al cierre del dueño. Si el historial rotó o el cursor pertenece a otro alcance, se devuelve `cursor_expired`; el cliente debe pedir una nueva lectura explícita. `--contains` busca texto literal. No combines `--tail` y `--after`.

Las respuestas indican `source`, `historical`, `observed_at_ms`, atribución, truncamiento y `has_more`. Cada página se limita a 2 MiB. Con el dueño ausente, `status` informa `owner.state:absent` y `execution_state:unknown`; conserva la última identidad conocida. Los logs de disco son históricos y no demuestran que un backend externo esté detenido. Un canal inaccesible devuelve su error: nunca se convierte en ausencia ni habilita una lectura histórica alternativa.

La configuración opcional de readiness usa HTTP/HTTPS local con la API nativa de Windows:

```json
"readiness": {
  "url": "http://127.0.0.1:8080/actuator/health/readiness",
  "timeout_seconds": 120,
  "json_field": "status",
  "expected": "UP"
}
```

Con `json_field` omitido basta una respuesta 2xx. El puerto debe pertenecer a un proceso de esa ejecución; otro servidor saludable no puede dar un positivo. Solo se consulta durante el arranque y se limita el cuerpo a 16 KiB. Un timeout devuelve error y deja el proceso activo para consultar sus logs o detenerlo con su ID. También se puede pasar `--health-url URL` para un endpoint con JSON `status=UP`. La URL admite localhost o IP de loopback; no sigue redirecciones ni desactiva la validación de certificados HTTPS.

Usá el directorio de datos habitual para compartir el dueño y la cola. Reservá `--data-dir` con perfiles propios para entornos que requieren aislamiento: cada directorio tiene un dueño independiente. Las operaciones de Soma protegen el checkout frente a compilaciones simultáneas y cambios de rama. Herramientas externas a Soma no participan de sus bloqueos.

`wait --operation ID` conserva el resultado final después del cierre automático, con `source:"disk"`; se retienen hasta 100 resultados de aplicación o build. Son resultados históricos: no prueban que una API siga disponible. Sin resultado final, la ausencia del dueño sigue siendo un error; un error de permisos nunca habilita la consulta histórica. Los resultados y logs de trabajos Maven mantienen su historial independiente.

## Trabajos Maven independientes

```powershell
soma-cli jobs configs --path C:\repos\mi-worktree
soma-cli jobs submit --path C:\repos\mi-worktree --config tests --kind test -- -pl commons -am -Dtest=ExampleTest -Dsurefire.failIfNoSpecifiedTests=false test
soma-cli jobs list --path C:\repos\mi-worktree --summary --limit 10
soma-cli jobs wait --job ID --timeout 600 --summary
soma-cli jobs status --job ID
soma-cli jobs logs --job ID --tail 200
soma-cli jobs release --job ID --participant MI_PARTICIPACION
```

`submit` requiere argumentos Maven después de `--` y no ejecuta un `test` implícito. `--help` antes de `--` muestra ayuda sin iniciar el dueño. Las opciones desconocidas de Soma se rechazan antes de iniciar un trabajo; después de `--`, los argumentos se entregan a Maven.

`version` / `--version` y `jobs help` no abren Soma ni crean datos. `version --json` devuelve la versión estructurada. `--data-dir DIRECTORIO` también puede preceder al comando.

### Resultados compactos y consultas

`jobs status --summary`, `jobs wait --summary` y `jobs submit --summary` devuelven los campos en la raíz: `job_id`, `state`, `verdict`, `exit_code`, `evidence`, `directory`, `branch`, `head`, `selection`, `timings` y `next_action`. El JSON completo de status/wait sigue disponible omitiendo `--summary`; sus campos están dentro de `job`. Un submit sin `--wait` confirma la aceptación; no convierte su resumen en validación final.

`status` con `ok:true` y salida 0 confirma la consulta, incluso si `verdict` es `failed`. Para completar una validación usá `wait`: conserva los códigos de error del resultado completo. `verdict` puede ser `passed`, `passed_with_skips`, `built`, `failed`, `incomplete`, `invalidated`, `cancelled`, `interrupted`, `pending` o `unknown`. `built` no acredita tests. Un código Maven distinto de cero, la falta de evidencia o un cambio de entradas nunca se convierten en éxito por usar el resumen. Un estado activo leído sin dueño disponible se informa como desconocido; no prueba progreso actual.

`jobs list --path DIRECTORIO --state failed,invalidated --limit 20 --summary` filtra el checkout exacto y los estados, ordena del más reciente al más antiguo y devuelve `matched` / `has_more`. Sin `--limit` devuelve los trabajos retenidos; el límite máximo es 1228 (100 detallados, 128 pendientes y 1000 archivados). La forma compacta es la misma con dueño disponible o leyendo desde disco; `participants` sólo está disponible al consultar al dueño. El resumen omite argumentos completos y stacks; conserva la selección y el primer fallo acotado.

Para agentes: iniciá `jobs wait --timeout 600 --summary` en una sesión no bloqueante del anfitrión, conservá su identificador de terminal y revisá esa sesión con esperas acotadas. Evitá encadenar `wait --timeout 1`, `status` y el mismo `tail` sin un cambio que diagnosticar. Un timeout mantiene activos el trabajo y la participación; retirate con tu `participant_id` si abandonás trabajo pendiente. `wait` consulta internamente el estado cada 250 ms; no es una suscripción a eventos.

### Logs incrementales de trabajos

`jobs logs --job ID --tail 100` devuelve un `cursor` ligado al historial del trabajo canónico. Para continuar usá `jobs logs --job ID --after CURSOR`, opcionalmente con `--contains TEXTO`. El cursor avanza por los bytes consumidos, incluidas las líneas descartadas por el filtro. No se combina `--after` con `--tail`. `has_more` pide continuar con el cursor devuelto; una lectura vacía no termina el job.

Los cursores no se pueden compartir entre trabajos; una rotación o historial distinto puede devolver `cursor_expired`, que requiere una lectura inicial nueva. `job_logs_unavailable` indica logs retirados por retención, aunque siga disponible la metadata del resultado. Las lecturas están acotadas y funcionan después del cierre del dueño. `source:disk` describe la lectura de archivos, no la salud de un proceso.

### Comprobar y clonar configuraciones de pruebas

```powershell
soma-cli jobs doctor --path C:\repos\mi-worktree --config tests
soma-cli jobs doctor --path C:\repos\mi-worktree --config tests --require docker
soma-cli jobs config-clone --path C:\repos\mi-worktree --from tests --to isolated --env-file C:\private\tests.env --to-data-dir C:\private\soma-tests
soma-cli jobs doctor --path C:\repos\mi-worktree --config isolated --data-dir C:\private\soma-tests
```

`doctor` inspecciona el proyecto, la configuración de tests, el JDK, el Wrapper y los archivos del adaptador sin ejecutar Maven, tests ni el adaptador. `--require docker` consulta el daemon con un plazo de cinco segundos. No valida compatibilidad de Testcontainers, bases, permisos ni credenciales. No imprime valores del entorno ni el contenido de archivos inválidos; no hereda la configuración del runtime y no inicia un dueño.

`config-clone` conserva entradas, exclusiones, JDK y entorno de la configuración de tests elegida. `--env-file` reemplaza explícitamente su archivo de variables; el destino debe tener un ID nuevo. Sin `--to-data-dir`, agrega la configuración al proyecto actual. Con esa opción prepara un proyecto de tests para el checkout exacto en otro directorio de datos: conserva el adaptador y la concurrencia de origen al crearlo, y rechaza destinos con otro `job_adapter` o un proyecto del mismo ID para otro checkout. No altera el origen ni copia la configuración de ejecución. Las ediciones de Soma se serializan y reemplazan el JSON atómicamente, preservando las entradas ajenas. No se sobrescriben configuraciones existentes.

Un clon compartido conserva las rutas internas referidas al proyecto registrado y las resuelve para cada checkout al usarse; un clon aislado conserva las rutas del checkout elegido. Las rutas externas explícitas permanecen externas.

`submit` abre/reutiliza Soma y devuelve `job_id` y `participant_id`. Pedidos iguales sobre el mismo checkout, entradas, entorno, selección y adaptador comparten el trabajo mientras está pendiente o activo. Después de terminar, otro pedido ejecuta Maven de nuevo. Guardá ambos IDs. Retirar tu participación solo cancela cuando no quedan otras; un timeout de `wait` conserva tu participación. Cerrar la ventana cancela todos sus trabajos. Consultar resultados o logs con la ventana cerrada no la abre.

La raíz de `projects.json` admite `job_concurrency` entre 1 y 3; por defecto es 1. El valor se carga al iniciar el dueño. Trabajos de distintos checkouts pueden avanzar a la vez; un checkout sigue protegido por su lease y se elige otro trabajo listo cuando el suyo está ocupado. La concurrencia no aísla bases de datos, puertos ni proveedores. El `job_adapter` sigue siendo responsable de respetar la política de recursos de la máquina. `jobs status` devuelve `wait_reason`: `soma_queue`, `checkout_in_use` o `resource_admission`; las duraciones siguen separadas en `timings`. Un dueño existente conserva su configuración hasta su cierre normal; no cierres un dueño ajeno para aplicarla. Las versiones anteriores no aceptan el campo nuevo si se configura explícitamente.

Los proyectos Maven nuevos reciben `test_configs`; en proyectos existentes agregá una configuración separada:

```json
"test_configs": [{
  "id": "tests", "name": "Tests Java 21", "jdk": "C:\\Java\\jdk-21",
  "env_file": null, "env": {}, "inputs": ["."], "input_excludes": []
}]
```

Los tests heredan el entorno de Soma y sus propios overrides; el `.env`, perfil Spring y argumentos JVM de la aplicación no se trasladan automáticamente. Indicá un `env_file` si las pruebas lo necesitan. `--kind build` permite una compilación sin exigir tests; `--kind test` exige evidencia aunque Maven devuelva cero. Sin argumentos se usa `test` o `package`, respectivamente. Se ejecuta el Wrapper del checkout, o `mvn.cmd` si no existe. Esta entrega requiere el `pom.xml` de ese checkout: no admite `-f/--file`, módulos externos ni rutas de módulos calculadas.

Los resultados JSON conservan checkout, rama/HEAD, huella de entradas y configuración, código de salida, conteos, omisiones y hasta 20 detalles de fallos. Los valores de propiedades Maven se ocultan salvo selecciones `test`/`it.test`. Los logs y reportes originales pueden contener lo que impriman las herramientas: tratá esos archivos como datos locales privados. Surefire/Failsafe reciben un sufijo único para recoger solo XML de ese trabajo en sus carpetas estándar. XML deshabilitado, rutas de reportes personalizadas, reportes incompletos o cero tests ejecutados producen validación incompleta. Cambios en entradas/configuración durante la ejecución invalidan el resultado. Esto detecta diferencias al inicio/final; no congela el código ni detecta un cambio revertido entre ambos puntos.

`state` describe el proceso y `evidence.status` la validación. `jobs wait` devuelve error para fallo (5), cancelación (130), entradas cambiantes (10), evidencia incompleta (11) o interrupción (12); un timeout devuelve 6. No interpretés `jobs status` con `ok:true` como tests aprobados: indica que la consulta fue válida. `requested_maven_args` contiene argumentos solicitados; `effective_maven_args` registra los que recibió el proxy después de la normalización del adaptador. Ambos ocultan valores de propiedades.

`timings` separa preparación, cola de Soma/checkout, espera del adaptador, ejecución y recolección de evidencia. `resource_wait_ms` incluye arranque del adaptador, admisión y evento del hijo; `maven_ms` incluye el Wrapper y el pequeño cierre del proxy. `test_reported_ms` proviene del XML y no se suma como otra fase del tiempo total. Son mediciones de pared, no un desglose interno de Maven ni una promesa de acelerar la compilación.

El resultado completo incluye `timings.preparation`, `pre_execution` y `final_verification`: reactor, configuración, entorno, adaptador y `fingerprint` (`scan_ms`, `git_ms`, `hash_ms`, `files`, `bytes`). `report_collection_ms` separa el XML de la comprobación final. Son submediciones de fases ya incluidas en los tiempos anteriores; no se suman otra vez. Las verificaciones de contenido se mantienen antes y después de Maven; no se sustituyen por fechas de modificación.

Se retienen 100 resultados detallados y hasta 30 MiB de logs de trabajos en total, aparte del backend. Al retirar un resultado detallado se conserva metadata sin stacks en `job-history`, limitada a 1000 registros, 16 MiB y 30 días; la poda ocurre al abrir el dueño o archivar resultados. `jobs status/wait --job ID` también consultan ese archivo, con `archived:true`, `details_retained:false` y disponibilidad de logs explícita. Usá `jobs list --history --summary --limit 1000` para incluirlo y redirigí su JSON a un archivo si necesitás exportarlo. La metadata no acredita vigencia de fuentes ni salud actual y nunca se usa como caché de resultados verdes.

Soma reserva espacio en el archivo antes de retirar un resultado detallado. Si no puede leer o desalojar metadata, conserva la evidencia y devuelve `history_unavailable` al intentar nuevos envíos; las consultas al dueño pueden incluir `history_error`. La exportación tampoco omite silenciosamente registros ilegibles. Después de resolver el problema de almacenamiento, el próximo envío reintenta el mantenimiento. Los trabajos que ya estaban activos conservan sus participaciones.

Cada trabajo activo usa un buffer de 64 KiB y hasta tres segmentos de 1 MiB. La cola admite 128 pedidos pendientes y cada trabajo 128 participaciones. Las 128 referencias intermedias más recientes se conservan; usá el ID canónico devuelto por Submit. Al volver a abrir después de una interrupción se registra `interrupted`, sin reejecutar automáticamente.

Para permitir tests mientras una aplicación Java sigue activa, añadí `"snapshot": true` a su `run` de tipo `java_jar` únicamente si el JAR es autocontenido. Soma copia y verifica el JAR, conserva el CWD original y ejecuta esa copia hasta el próximo reinicio. No aísla archivos externos, bases de datos o puertos: elegí un entorno de pruebas adecuado. Los runtimes genéricos y JAR sin esta opción mantienen la exclusión del checkout. Los cambios de rama siguen bloqueados mientras corre una aplicación.

Una política local puede configurar `job_adapter` como comando con `executable` y `args`. Soma añade `-RequestFile RUTA`; el JSON transitorio identifica el Wrapper, argumentos, directorio y proxy de Soma. El adaptador debe pasar el proxy `soma_cli __maven_child --request RUTA -- ARGUMENTOS_MAVEN` por su launcher, con los argumentos como array. El proxy avisa después de la admisión y ejecuta el Wrapper con los argumentos efectivos. Sin adaptador se usa ese proxy directamente. No hay servicio, JVM Maven residente ni dependencia de herramientas de IA.

## Desarrollo y releases

Se compila con Rust estable, target `x86_64-pc-windows-gnu`, linker MinGW y `windres` en PATH (o `WINDRES` con su ruta). `cargo build --release --jobs 1` genera `Soma.exe` y `soma-cli.exe`, que deben quedar juntos. `Cargo.lock` fija dependencias.

`model.rs` posee configuración y vigencia; `git.rs`, checkouts y protección del directorio; `readiness.rs`, la comprobación HTTP local; `engine.rs`, estados, logs y procesos; `worker.rs`, consola y Ctrl+C; `ipc.rs`, comunicación local; `ui.rs`, controles Win32. El refresco del visor impide reentrada de notificaciones del sistema.

El monograma S de `assets/icons/05-soma-vapor.svg` conserva las proporciones de la referencia de diseño elegida. Su ICO se incrusta en el EXE. No se cargan imágenes de presentación durante la ejecución.

[Flujo de publicación](docs/releases.md). Un tag `vX.Y.Z` dispara build, pruebas, ZIP, release y actualización del bucket de Scoop. Licencia MIT.
