# Soma

Ejecutá aplicaciones locales, compilá proyectos y consultá sus logs desde una ventana de Windows o una CLI con respuestas JSON.

Soma es una aplicación nativa para **Windows x64**. Incluye una cola de trabajos Maven para compartir compilaciones y tests entre terminales y agentes.

## Instalar

Con [Scoop](https://scoop.sh):

```powershell
scoop bucket add soma https://github.com/MatiasFrangolini/soma
scoop install soma/soma
```

También podés descargar el ZIP de la [última versión](https://github.com/MatiasFrangolini/soma/releases/latest) y extraerlo. Mantené `Soma.exe` y `soma-cli.exe` juntos.

Soma funciona por su cuenta. Para proyectos Java necesitás un JDK compatible con tu proyecto y Maven o el Wrapper del repositorio. Git permite usar las opciones de ramas y worktrees. Windows Terminal es opcional.

## Usar la ventana

1. Abrí **Soma** desde Inicio o ejecutá `soma`.
2. Elegí **Carpeta** y seleccioná tu proyecto. Si tiene `pom.xml`, Soma prepara las configuraciones Java.
3. Revisá **Abrir config**: JDK, JAR, `.env` y comando de build deben corresponder a tu proyecto.
4. Elegí **Compilar y Run**. Consultá la salida en el visor y usá **Stop** al terminar.

**Run build vigente** evita compilar si Soma puede verificar que el build corresponde al checkout y sus entradas actuales. Los selectores **Worktree** y **Rama** permiten elegir dónde ejecutar; un cambio de rama requiere un checkout limpio y sin una aplicación activa.

## JVM optimizada

Los proyectos Maven nuevos ofrecen dos opciones:

| Configuración | Uso |
| --- | --- |
| **Local** | Conserva los valores predeterminados de la JVM. |
| **JVM optimizada** | Usa G1, heap inicial de 64 MiB y máximo de 768 MiB. Pensada para desarrollo con Java HotSpot 21. |

El límite corresponde al heap: Java también consume memoria fuera de él. Si tu aplicación necesita más, ajustá `-Xmx` o elegí Local.

Para agregar la opción a un proyecto que ya tenías registrado:

```powershell
soma-cli projects
soma-cli apply-preset --project ID --preset jvm-optimized
```

El comando conserva la configuración original y cualquier preset que ya hayas personalizado. Los ajustes están en [presets/jvm-optimized.json](presets/jvm-optimized.json); cada persona usa su propio JDK, checkout y entorno.

## Usar la CLI

```powershell
soma-cli add --path C:\repos\mi-app
soma-cli run --path C:\repos\mi-app --config jvm-optimized
soma-cli status
soma-cli logs --tail 100
```

`run` devuelve un `operation_id`. Usalo para consultar o detener esa ejecución:

```powershell
soma-cli status --operation ID --health
soma-cli logs --operation ID --tail 100
soma-cli stop --operation ID
```

`--health` comprueba ahora el endpoint local de readiness configurado y verifica que pertenezca a esa ejecución. Si querés esperar al arranque, usá `wait --operation ID --ready`; `--wait` espera que el proceso termine.

La CLI trabaja **sin abrir ventanas**. Reutiliza el dueño del mismo directorio de datos y, si debe iniciar uno temporal, lo cierra después de dos segundos sin aplicación ni trabajos pendientes. `soma-cli show` muestra la interfaz de ese mismo proceso y la mantiene abierta hasta cerrarla. Los resultados y logs siguen disponibles después del cierre.

Para leer sólo lo nuevo, pasá el `cursor` de una respuesta a `logs --after CURSOR`. Podés combinarlo con `--operation ID` y `--contains TEXTO`. Un cursor vencido devuelve un error explícito. Los logs guardados son historial; no prueban que una aplicación siga funcionando.

## Compilar y probar con Maven

```powershell
soma-cli jobs configs --path C:\repos\mi-app
soma-cli jobs doctor --path C:\repos\mi-app --config tests
soma-cli jobs submit --path C:\repos\mi-app --config tests --kind test -- test
soma-cli jobs wait --job ID --summary
```

Soma usa el Wrapper del proyecto y registra código de salida y evidencia de Surefire/Failsafe. Las solicitudes equivalentes pendientes o activas comparten el trabajo; cada solicitante recibe su `participant_id`. Para retirarte, usá `jobs release --job ID --participant ID`.

Un timeout deja el trabajo activo. Un `jobs status` con `ok:true` confirma la consulta; `jobs wait` y su evidencia indican si la validación pasó. La configuración JVM de la aplicación se mantiene separada de Maven y de los tests.

`--summary` muestra un veredicto compacto con checkout, selección y conteos. `jobs list --path DIRECTORIO --summary` filtra los trabajos; `jobs logs --job ID --after CURSOR` continúa una lectura sin repetir el historial. Al retirarse los 100 resultados detallados, la metadata se archiva con límites de cantidad, edad y tamaño. Los comandos y la creación segura de configuraciones de tests se explican en [la referencia](CLI.md#trabajos-maven-independientes).

## Configuración y actualizaciones

Tus proyectos y logs viven en `%USERPROFILE%\.soma`. El entorno sigue este orden: proceso dueño → `.env` del checkout → overrides explícitos. `soma-cli config-path` muestra el JSON y `soma-cli validate` comprueba la configuración.

Usá el directorio habitual para compartir instancia y cola. Reservá `--data-dir` para entornos que necesitan aislamiento. Cada usuario configura sus rutas, credenciales y proveedores; el ZIP incluye el preset público y conserva los datos personales fuera de la instalación.

Para actualizar, cerrá tus ejecuciones y Soma; después ejecutá:

```powershell
soma-update
```

Para desinstalar: `scoop uninstall soma`. Tus datos personales se conservan.

## Más información

- [Guía completa de CLI y configuración](CLI.md).
- [Portabilidad, requisitos Java y compilación](https://github.com/MatiasFrangolini/soma/blob/main/docs/portability.md), incluida como `PORTABILITY.md` en el ZIP.
- [Publicar una versión](https://github.com/MatiasFrangolini/soma/blob/main/docs/releases.md).
- `soma-cli help` muestra los comandos disponibles. [Licencia MIT](LICENSE).
