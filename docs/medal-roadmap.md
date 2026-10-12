# Próximas implementaciones (comparación con Medal)

Qué le falta a ClipForge frente a lo que Medal anuncia, y el plan de cada
pieza. Sale de la página de features de Medal
(<https://medal.tv/es/features>, consultada el 2026-10-11). Es una página de
marketing: dice **qué** ofrece, no **cómo** funciona por dentro, así que los
planes de abajo son nuestros, no una copia.

Esfuerzo: **S** = horas, **M** = alrededor de un día, **L** = varios días. Es el
mismo criterio de `docs/settings-roadmap.md`. Un plan no es una promesa: el
orden se decide al empezar cada uno.

## Estado de partida

| Ya está | Dónde |
|---|---|
| Replay buffer con H.264 por hardware, atajo global, bandeja | F-1.x, F-2.x |
| Audio del juego y micrófono (mezclado o en pista aparte) | F-2.5, F-3.3 |
| Galería, editor de recortes con una línea por pista de audio | F-3.1, F-3.2 |
| Carpetas (un clip en varias) | F-3.9, `docs/library.md` |
| **Biblioteca inteligente**: buscar, filtrar y ordenar; juego detectado | F-3.10 |
| **Atajos con distinta duración**: la duración del atajo principal y hasta 4 atajos más | F-3.5 |

**Bloqueo transversal: F-2.4 (paridad Windows).** Hoy no existe captura ni
codificador de Windows (`Recorder::start` devuelve un error fuera de macOS), así
que nada de lo de abajo se puede *ejecutar* en Windows. En cada plan, "Windows"
significa "se compila y se lintea con el cruce del CI, y se prueba cuando F-2.4
exista".

## Resumen

| # | Función (Medal) | ClipForge hoy | Siguiente paso | Esf. | Depende de |
|---|---|---|---|---|---|
| 0 | Biblioteca inteligente | **Hecha** | Seguimientos en el backlog | — | — |
| 1 | Clips de cualquier duración | **Hecha** (hasta 4 atajos extra, duración propia) | Seguimientos abajo | — | — |
| 2 | Comparte fácilmente | No | Copiar al portapapeles, aviso de tamaño | S–M | — |
| 3 | Captura de pantalla | No | Atajo que guarda un PNG | M | — (Windows: F-2.4) |
| 4 | Biblioteca por juego / perfiles | Carpeta automática por juego **hecha** (4a) | Perfiles por juego (4b) | L | 0, 1 |
| 5 | Marcadores | No | Atajo «Marcar momento» | M | — |
| 6 | Edita con facilidad (cámara lenta, zoom, GIF…) | Solo recorte | Exportar… con recodificación | S → L | **Decisión** |
| 7 | Detección de eventos en juego | No | Eventos por APIs locales oficiales | L | 0, post-roll |
| 8 | Audio multipista (juego, voz, Discord) | Juego y voz | Pista por aplicación | L | F-2.4 (Windows) |
| 9 | Grabación de larga duración | No | Escritor MP4 fragmentado | L | 5, límite de espacio |
| 10 | Códecs modernos, hasta 4K a 144 fps | Solo H.264 1080p60 | Presets, HEVC, AV1 | L | Presets de calidad |
| 11 | Compatible con todos los juegos | Pantalla completa | Elegir monitor o ventana | M–L | F-2.4 (Windows) |
| — | Nube, links, app móvil, comunidad… | No | Fuera de alcance sin servidor | — | — |

## Orden sugerido

```
0 Biblioteca inteligente (hecha)
├─ 1 Atajos con duración (hecha) ──────┐
├─ 4a Carpeta automática (hecha)       ├─ 4b Perfiles por juego
├─ 2 Compartir sin servidor            │
├─ 3 Capturas de pantalla              │
├─ 5 Marcadores ───────────────────────┼─ 9 Grabación larga
├─ 6 Exportar… ─ (decisión) ───────────┘
├─ 7 Eventos de juego  (necesita post-roll de la ola 1)
├─ 8 Audio por aplicación
├─ 10 Códecs / 4K      (necesita presets de calidad de la ola 1)
└─ 11 Monitor / ventana
```

Primero lo que casi no cuesta y se nota al jugar (1, 2, 3, 4a). Los marcadores y
Exportar son los que más abren después. Los eventos de juego son lo más
parecido a «lo que hace famoso a Medal», pero necesitan tres cosas antes: el
post-roll, el juego detectado (ya está) y un límite de espacio, porque un clip
automático por jugada llena el disco.

---

## 0. Biblioteca inteligente — hecha (F-3.10)

Buscar (título, juego, nombre de carpeta), filtrar (juego, carpeta, fecha) y
ordenar, sobre Recientes, la raíz de la Biblioteca y cada carpeta; el juego sale
de la aplicación que estaba en primer plano mientras se grababa el clip
(`core/src/activity/`, `app/src-tauri/src/clipmeta.rs`). Diseño en
`docs/library.md`. Las etiquetas de Medal son nuestras carpetas (un clip puede
estar en varias).

Seguimientos (también en `milestones/backlog-improvements.md`): duración en el
índice y orden por duración, filtros que se recuerdan al reiniciar, ícono de la
aplicación, lista de aplicaciones ignoradas editable, guardar el juego dentro
del mp4, pantalla propia para renombrar juegos, y el pendiente de verificar con
un juego real y en Windows.

---

## 1. Atajos con distinta duración — hecha (F-3.5)

Medal deja guardar «los últimos segundos o minutos». Ahora el atajo principal
tiene su duración (`shortcuts.clipSeconds`, 30 s por defecto, de 5 s hasta los 2
min del buffer) y se pueden sumar hasta 4 atajos más, cada uno con la suya
(`shortcuts.extraSaveClip`: F9 para los últimos 15 s, F10 para el último
minuto…). La bandeja y el botón de la galería guardan la duración principal y
la muestran. Las reglas de teclas son las de siempre (`works_alone`), y todos
los atajos tienen que ser distintos entre sí y del de silenciar.

Cómo quedó, para quien toque esto después:
* `shortcuts::switch` compara las registraciones como un **conjunto** de
  combinaciones, no por posición, así que la lista puede crecer y achicarse; si
  solo cambia una duración no se llama al sistema (la acción se busca al apretar
  la tecla). Si el SO rechaza una, se devuelve todo como estaba.
* `Command::SaveClip(Option<u64>)`: `None` es «la duración de Configuración»
  (bandeja y botón); un atajo manda la suya.
* La fila de un atajo nuevo escucha las teclas apenas se agrega; una fila sin
  teclas no se guarda (el motor exige que cada atajo sea válido).

Con el buffer recién iniciado el clip sale más corto que lo pedido; la
notificación «Clip guardado» ya dice los segundos reales. Seguimiento posible:
una entrada de la bandeja por cada atajo extra.

---

## 2. Compartir sin servidor

**Qué es / por qué.** Medal genera un link. Sin servidor propio, lo más cercano
es dejar el archivo listo para pegarlo en Discord, un chat o un mail.

**Estado hoy.** La tarjeta tiene editar, renombrar, carpetas, mostrar el archivo y
borrar; nada para sacar el clip de la app.

**Plan.**
1. Comando `copy_clip_to_clipboard(name)` (valida con `library::clip_path`):
   macOS, `NSPasteboard.generalPasteboard` con la URL del archivo
   (`objc2-app-kit`, feature `NSPasteboard`); Windows, `CF_HDROP`
   (`Win32_System_DataExchange`, `Win32_System_Memory`, `Win32_UI_Shell`).
2. UI: acción «Copiar» en la tarjeta y el reproductor; aviso «Copiado, pegalo
   en Discord».
3. `lib/share.ts` (puro): si el clip pesa más que el límite de un destino, avisa
   y propone «Exportar para Discord» (plan 6). Los límites **cambian**: verificar
   los vigentes de Discord al implementar y dejarlos como tabla editable.
4. Paso 2, aparte: arrastrar el clip desde la ventana hacia otra app. Arrastrar
   hacia afuera desde un webview necesita una sesión de arrastre nativa
   (`NSDraggingSession` / `DoDragDrop`); evaluar un plugin de Tauri antes de
   escribirlo a mano.

**Pruebas.** TS: tabla de límites. Rust: validación de nombre. macOS: test
ignorado que copia y relee el pasteboard con AppKit (`pbpaste` no lee URLs de
archivo). Windows: solo compilación.

**Riesgos.** Pegar un archivo en Discord web depende del navegador; en la app de
escritorio funciona. En Windows, la propiedad del handle del portapapeles es
fácil de hacer mal (se detecta con una prueba real).

**Esfuerzo:** S–M. **Depende de:** nada; se complementa con el plan 6.

---

## 3. Capturas de pantalla

**Qué es / por qué.** «Toma una captura con un botón y la añade a tu
biblioteca.»

**Estado hoy.** Todo el pipeline es de video: `library::is_clip_file` solo acepta
`.mp4` y `ClipInfo` no tiene tipo.

**Plan.**
1. `core/src/capture/macos/screenshot.rs`: `SCScreenshotManager::capture_image`
   (macOS 14+, feature `macos_14_0` del crate `screencapturekit` 8; verificar si
   ya viene activa) y `save_png`. Excluir las ventanas de ClipForge.
2. Nombre `shot-YYYYMMDD-HHMMSS.png` en la carpeta de clips, con el mismo
   desempate `-2`, `-3` de `ClipService::unique_clip_path`.
3. Atajo propio: `shortcuts.screenshot: Option<String>` (sin atajo por defecto: no
   se registra nada que el usuario no pidió), `ShortcutAction::Screenshot`,
   `Command::Screenshot`, notificación.
4. Biblioteca: `ClipInfo.kind` (`clip` | `screenshot`). `list_clips` incluye
   `.png`; separar `clip_path` (video: editor, recorte, forma de onda) de un
   `media_path` más amplio (renombrar, borrar, carpetas, mostrar). La tarjeta y el
   reproductor muestran `<img>`; la barra de filtros gana «Tipo».
5. «Copiar» del plan 2 también copia la imagen.

**Pruebas.** Rust: nombres y extensiones, que el editor rechace un `.png`. macOS:
test ignorado con pantalla real (firma PNG y dimensiones). TS: tarjeta de imagen,
filtro por tipo.

**Riesgos.** Retina: pedir el tamaño en píxeles, no en puntos. Con varios
monitores toma el primero (igual que la captura de video, ver plan 11). HDR.

**Esfuerzo:** M en macOS. **Depende de:** nada (Windows: F-2.4, un cuadro de
Graphics Capture).

---

## 4. Carpeta automática por juego y perfiles por juego

**Qué es / por qué.** Con el juego detectado (F-3.10) y las carpetas (F-3.9) ya
están las dos mitades de la «biblioteca por juego» de Medal; falta unirlas.
Después, que cada juego tenga su configuración.

**Plan A (S): carpeta automática — hecho (F-3.11).** `library.autoFolders`
(apagado por defecto: crea carpetas). Al guardar un clip con juego,
`FolderStore::file_under_game` lo pone en la carpeta de ese nombre (sin distinguir
mayúsculas) o la crea con ícono de joystick y un color que sale del nombre; una
carpeta del usuario se usa tal cual. Si se borra, el próximo clip del juego la
vuelve a crear mientras el ajuste esté prendido (es la propuesta que había aquí).
Detalle en `docs/library.md`. **Falta:** la acción «Archivar los clips que ya
existen por juego» (una sola vez) y avisar en el diálogo de borrado que la
carpeta volverá si el ajuste sigue prendido.

**Plan B (L): perfiles.**
1. `Settings.profiles: [{ game, clipSeconds, micMode, … }]`.
2. El `ActivityLog` anuncia el cambio de juego con histéresis (≥ 10 s) y
   `Recorder::apply_profile` aplica lo que es en vivo (duración, modo del
   micrófono, silencio). Lo que reinicia la captura (calidad) se aplica con aviso
   porque vacía el buffer.

**Pruebas.** `plan_game_folder` (puro), histéresis del cambio de juego (con
`ActivityLog` y relojes inyectados), idempotencia al guardar varios clips.

**Riesgos / decisión abierta.** Los nombres detectados pueden ser feos
(`VALORANT-Win64-Shipping`): primero corregirlos con «cambiar en todos los clips»,
y quizá una tabla de alias editable.

**Esfuerzo:** A = S (hecho); B = L. **Depende de:** 0 (hecho); B también de 1 (hecho) y de los
presets de calidad (ola 1).

---

## 5. Marcadores

**Qué es / por qué.** «Añadir marcadores con una tecla rápida.» Para un buffer de
replay sirve así: apretás «Marcar» en la jugada, más tarde guardás el clip, y el
editor te deja saltar a la marca. Es la idea «Marcar momento» del pozo de ideas.

**Plan.**
1. `MarkerLog` (como `ActivityLog`): instantes marcados, acotado al largo del
   buffer. Usa el reloj compartido `SharedEpoch` para poder traducir un marcador a
   segundos dentro del clip.
2. `shortcuts.mark: Option<String>`, `ShortcutAction::Mark`, aviso breve.
3. Al guardar: `ClipMeta.markers` (segundos desde el inicio del clip) en
   `clipmeta.json`. `editor::describe` los devuelve en `EditorClip.markers`.
4. Recorte: el gancho `clip_copied` traslada los marcadores al nuevo origen y
   descarta los que quedan fuera.
5. Editor: marcas en la línea de tiempo, tecla `M` para saltar a la siguiente
   (`lib/timeline.ts`: `nextMarker`, pura).

**Pruebas.** Traducción instante → segundos del clip (puro, con bordes: marca justo
antes del inicio, después del final), traslado en el recorte, `nextMarker`.

**Riesgos.** El mapeo de reloj es lo delicado: los paquetes del buffer usan el
reloj de captura, no `Instant::now()`; hay que anotar el marcador en esa base.

**Esfuerzo:** M. **Depende de:** nada. Lo reutiliza la grabación larga (plan 9).

---

## 6. Exportar… (recodificar de forma acotada)

**Qué es / por qué.** Medal edita con cámara lenta, zoom, GIF, música y pies de
foto; Discord limita el tamaño de lo que se sube. Casi todo eso necesita volver
a codificar, y hoy `CLAUDE.md` lo prohíbe («never re-encode»).

> **Decisión abierta.** Propuesta: una excepción **acotada**: recodificar solo
> en `core/src/clip/export.rs`, detrás de una acción explícita («Exportar…»), con
> los codificadores por hardware que ya existen y dejando intacto el original.
> Guardar un clip y recortarlo siguen siendo remux sin recodificar. Una sola
> excepción habilita casi todo lo de abajo.

**Estado hoy.** `trim.rs` copia el stream; no hay decodificador de video.
`videotoolbox` 0.18 trae un módulo `decompression`, así que decodificar H.264 en
macOS es posible; el AAC ya se decodifica con `symphonia` y se codifica con
AudioToolbox.

**Plan.**
1. **Paso 0, sin recodificar: cámara lenta/rápida por retemporización.** Cambiar
   las duraciones de las muestras de video (`stts`) en `mp4mux::write_mp4_cut`
   (parámetro `speed`) es un remux; el audio del tramo se descarta. A 60 fps,
   0,5× da 30 fps efectivos y se ve bien; 0,25× se ve entrecortado. Verificar con
   ffprobe y AVFoundation (duración, cantidad de cuadros). Solo clip entero en la
   primera versión.
2. `export.rs`: decodificar (VideoToolbox) → transformar (escala/recorte) →
   codificar (`Encoder` existente) → muxear (`mp4mux`). Audio: decodificar con
   `symphonia` y recodificar a la tasa pedida, o copiar si no cambia.
3. Presets: **tope de tamaño** (10 / 25 / 50 MB…): `plan_bitrate(duration, target,
   audio)` puro, una pasada, verificar el tamaño y repetir con 0,9× hasta 3
   veces; bajar resolución si la tasa por píxel queda muy baja. **Vertical 9:16**
   (recorte central + escala). **GIF** (crate `gif` + cuantización, ≤ 480 px, 10–15
   fps). Zoom (rectángulo de recorte animado) y pies de foto (CoreText) quedan
   para después.
4. UI: «Exportar…» junto a «Guardar recorte» en el editor: presets, tamaño
   estimado, barra de progreso con eventos `export-progress` y cancelar.

**Pruebas.** `plan_bitrate` (puro, bordes de duración y tope). Oráculos reales
como en el editor: ffprobe + AVFoundation (tamaño ≤ tope, duración, dimensiones) y
comparación de luminancia del primer y último cuadro con el origen
(`core/tests/support/avf_video.swift`).

**Riesgos.** Calidad contra tiempo (el hardware codifica varias veces más rápido
que el tiempo real), memoria de cuadros, etiquetado de espacio de color
(BT.709), HDR. Windows necesita decodificador y codificador de Media Foundation.

**Esfuerzo:** paso 0 = S; exportar con tope de tamaño = M; GIF = M; zoom y pies
de foto = L. **Depende de:** la decisión; Windows de F-2.4.

---

## 7. Detección de eventos en el juego

**Qué es / por qué.** Medal guarda el clip solo cuando hay una victoria, un gol,
un headshot. Es lo que más cambia la experiencia: no hay que apretar nada.

**Regla.** **Solo APIs locales oficiales de cada juego.** Nada de leer la memoria
del juego ni de inyectar overlays: el anti-cheat puede prohibir la cuenta. Verificar
la documentación vigente de cada API al implementar.

**Fuentes posibles.**
* **CS2 y Dota 2:** *Game State Integration*. El juego envía por HTTP el estado a
  una URL local que define un archivo `gamestate_integration_*.cfg` en su carpeta
  de configuración. ClipForge levanta un servidor en `127.0.0.1` y ofrece
  «Instalar la integración», que escribe ese `.cfg` (la carpeta se encuentra
  con `libraryfolders.vdf` de Steam). Los eventos salen de comparar estados
  consecutivos (más bajas, ronda ganada).
* **League of Legends:** *Live Client Data API*, un servidor HTTPS local con
  certificado propio; se consulta `eventdata` (baja, multibaja, ace).
* **Valorant y los que no exponen nada oficial:** no.

**Plan.**
1. `core/src/events/mod.rs`: `GameEvent { game, kind, at }`, trait
   `GameEventSource` y un `Debouncer` puro (junta ráfagas, enfriamiento).
2. Adaptadores `gsi.rs` y `league.rs`. No hay cliente HTTP en las dependencias:
   decidir entre un servidor mínimo sobre `tokio` y un crate chico; para League hay
   que aceptar un certificado propio **solo en loopback**.
3. App: un servicio que, al llegar un evento, espera el **post-roll** y llama a
   `save_clip(largo)`, marcando `ClipMeta.source = evento:<tipo>`; notificación.
4. Configuración: `events.enabled` (**apagado por defecto**: guarda clips solo),
   por juego y por tipo de evento, con segundos antes y después.
5. UI: panel «Eventos» con el estado por juego («Integración instalada ✓»), insignia
   ⚡ en los clips automáticos y un filtro «Origen: manual / automático».

**Pruebas.** Secuencias de estados grabadas (JSON) por juego → eventos esperados;
`Debouncer`; servidor en un puerto efímero con POST falsos. No hay prueba real sin
el juego.

**Riesgos.** Cambios de esquema de las APIs, puerto ocupado, `.cfg` en la carpeta
equivocada, ráfagas que llenan el disco (por eso el límite de espacio va antes).

**Esfuerzo:** L el marco; M por cada juego después. **Depende de:** post-roll
(ola 1), juego detectado (hecho), y conviene el límite de espacio (ola 2).

---

## 8. Audio por aplicación (pista de Discord)

**Qué es / por qué.** «Separa el volumen del juego, el micrófono y Discord en pistas
independientes.» Ya separamos juego y voz; falta aislar una aplicación.

**Estado hoy.** El buffer tiene los carriles `Main` y `Mic`; el muxer
(`write_mp4_with_tracks`) ya recibe N pistas y el editor dibuja una por pista, pero
las rotula por convención (primera = Juego, segunda = Micrófono).

**Plan.**
1. Generalizar el carril del buffer (`core/src/buffer`) a `Main`, `Mic` y
   `App(n)`; cada uno con sus `AudioParams`.
2. macOS: dos `SCStream` a la vez —uno con todas las apps **menos** la elegida y
   otro **solo** con esa app (audio, con un video mínimo)—, sobre el mismo
   `SharedEpoch`. Verificar que ScreenCaptureKit permita dos streams simultáneos.
3. Windows: *process loopback* de WASAPI (`ActivateAudioInterfaceAsync` con
   `AUDIOCLIENT_ACTIVATION_PARAMS`, incluir o excluir el árbol de procesos);
   requiere una versión reciente de Windows 10.
4. Configuración: «Pistas por aplicación» (elegir de las apps abiertas), con el
   nombre de cada pista **escrito en el mp4** para que el editor la rotule bien
   (cierra el seguimiento «name the tracks» del editor).
5. Editor: rótulo de cada línea a partir del nombre de la pista.

**Pruebas.** Carriles con fuentes falsas; test ignorado de macOS con tres tonos
(como `macos_mic_mix.rs`) decodificados con AVFoundation; muxer con 3 pistas.

**Riesgos.** Sincronía entre streams (el patrón de `SharedEpoch` ya la resuelve para
el micrófono), apps con procesos auxiliares (Discord), eco, CPU.

**Esfuerzo:** L. **Depende de:** F-2.4 para Windows.

---

## 9. Grabación de larga duración

**Qué es / por qué.** Grabar la sesión entera, no solo los últimos segundos.

**Estado hoy.** El muxer no es fragmentado: escribe el archivo al final desde la
RAM. Para una sesión de horas eso pierde todo si la app se cierra, y no cabe en
memoria.

**Plan.**
1. `core/src/clip/fmp4.rs`: escritor de MP4 fragmentado (`ftyp`, `moov` con
   `mvex`, y un `moof` + `mdat` por fragmento de ~2 s alineado a un keyframe).
   Un archivo cortado a la mitad se reproduce hasta el último fragmento.
2. `SessionRecorder`: recibe **los mismos paquetes codificados** que el ring
   buffer (el motor reparte a dos destinos), así que no hay una segunda
   codificación.
3. Al terminar, convertir a un mp4 normal para que `trim.rs` pueda abrirlo (hoy
   solo lee archivos no fragmentados) o enseñarle a leer fragmentos.
4. Espacio: ~8–15 MB por minuto a 1080p60 (cerca de 1 GB por hora); mostrar el
   tamaño corriendo y cortar si queda poco disco.
5. UI: botón «Grabar sesión» y en la bandeja, con cronómetro; insignia «Sesión» en
   la biblioteca. Los marcadores (plan 5) cuentan dentro de la sesión.
6. Editor: forma de onda y tira de cuadros para archivos de horas (hoy se decodifica
   todo): hacerlo por tramos.

**Pruebas.** Estructura de cajas con ffprobe; simulación de corte (truncar el
archivo y reproducir); 10 minutos sintéticos con `TestPatternSource` y memoria
plana.

**Riesgos.** Disco lleno, corte de luz, deriva de A/V en horas, rendimiento del
editor.

**Esfuerzo:** L. **Depende de:** plan 5 y el límite de almacenamiento (ola 2).

---

## 10. Códecs modernos y calidad alta (HEVC, AV1, 4K a 144 fps)

**Qué es / por qué.** Archivos más chicos a igual calidad y resoluciones más altas.

**Estado hoy.** Solo H.264, 1080p60 (`EncoderConfig::default()`). El crate `mp4`
0.14 tiene `Hev1Box`, pero su `HvcCBox` es un esqueleto (solo el byte de versión):
no escribe un `hvcC` válido. No hay caja para AV1.

**Plan.**
1. Primero los **presets de calidad** de la ola 1 (resolución, fps, tasa; reinician
   la captura y vacían el buffer: avisar). Medir RAM: 4K60 es ~4× el buffer actual.
2. **HEVC** con VideoToolbox: sacar VPS/SPS/PPS, escribir **nuestra** caja `hvcC`
   (HEVCDecoderConfigurationRecord) y una entrada `hvc1` en `stsd`, igual que hoy
   se parchea el `moov` al final del archivo.
3. `trim.rs`/`probe` aceptan `hvc1` (hoy rechazan lo que no sea H.264).
4. Reproducción: WebKit reproduce HEVC; en Windows, WebView2 necesita la extensión
   de video HEVC, así que las miniaturas de la galería pueden fallar: prever aviso o
   un cuadro de portada generado aparte.
5. **AV1:** Apple no tiene codificador AV1 por hardware, así que solo Windows con
   GPU reciente (Media Foundation), después de F-2.4.

**Riesgos.** Compatibilidad al compartir (HEVC en navegadores y Discord), RAM,
límites del codificador.

**Esfuerzo:** L. **Depende de:** presets de calidad; AV1 y Windows de F-2.4.

---

## 11. Elegir monitor o ventana

**Qué es / por qué.** «Compatible con todos los juegos»: hoy se captura el primer
monitor entero (`ScreenCaptureKitSource`).

**Plan.**
1. Comando `list_capture_targets`: monitores, ventanas y apps de
   `SCShareableContent`, con miniatura (reutiliza el plan 3).
2. `ScreenCaptureConfig.target: Display | Window | Application` y una función pura
   `plan` como `mic::plan` para decidir qué reiniciar.
3. Se guarda por id estable (UUID del monitor, bundle id de la app) y, si ya no
   existe, vuelve al monitor principal con un aviso.
4. Cambiarlo reinicia la captura de video: avisar que se vacía el buffer.
5. «Solo el juego»: proponer el juego detectado (F-3.10). El filtro de audio debe
   coincidir con el objetivo (se cruza con el plan 8).
6. Windows: selector de Graphics Capture; F-2.4.

**Pruebas.** `plan` (puro); test ignorado real por tipo de objetivo.

**Riesgos.** La ventana capturada desaparece (el juego se reinicia): volver a
resolverla por app. Pantalla completa exclusiva.

**Esfuerzo:** M–L. **Depende de:** F-2.4 para Windows.

---

## Fuera de alcance mientras no haya servidor

Links para compartir, cargas instantáneas, nube (Medal Cloud), app móvil con
sincronización, visualizaciones y comunidad, privacidad pública/privada de los
clips, bot de Discord y SDK para estudios. Todos necesitan algo hospedado: cuentas,
almacenamiento, transcodificación y moderación. Lo más barato si algún día se
quiere uno es subir a un destino que el usuario ya tiene (un webhook de Discord con
el tope de tamaño del plan 6, o una carpeta sincronizada); un servicio propio es
otro proyecto.

## Decisiones abiertas

1. **Excepción de recodificación** (plan 6): ¿se permite solo en «Exportar…»?
2. **Cámara lenta por retemporización** (plan 6, paso 0): ¿alcanza dejarla sin
   audio, o se exige conservar el sonido (y entonces se recodifica)?
3. **Carpeta automática por juego** (plan 4): ¿se recrea si el usuario la borra
   mientras el ajuste está prendido?
4. **Eventos** (plan 7): qué juegos entran primero y si los clips automáticos van
   en una carpeta propia.
5. **Límites de Discord** (plan 2): verificar los vigentes al implementar.
