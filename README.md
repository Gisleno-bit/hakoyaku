# hakoyaku (箱訳)

Traduce en tiempo real el texto que aparece dentro de un recuadro de la pantalla
y lo muestra en un panel flotante al lado.

Pensado para juegos japoneses que no tienen traducción: marcas una vez dónde está
la caja de diálogo, y a partir de ahí cada frase nueva aparece traducida al lado
sin tocar nada.

Por defecto trabaja **en modo in-place**: tapa el texto original con un parche
del mismo color que la caja de diálogo y escribe encima la traducción. El
resultado es que el juego parece traducido, no que tenga un cartel pegado.

```
   ANTES                              DESPUÉS  (mode = "inplace")
┌──────────────────────────────┐   ┌──────────────────────────────┐
│  積雪の様にみっしりと         │   │  Sales a un espacio extraño, │
│  白い絨毯が敷かれた           │ → │  cubierto por una alfombra   │
│  奇妙な空間に出る。           │   │  blanca y espesa como nieve. │
└──────────────────────────────┘   └──────────────────────────────┘
```

Por defecto el parche cubre **la bandeja entera** (`inplace_cover = "box"`), no
solo el hueco que ocupaban las letras: queda más limpio y deja sitio de sobra,
que hace falta porque el castellano ocupa más que el japonés. Con
`inplace_cover = "text"` se ciñe a las letras.

Si prefieres conservar el original a la vista, `mode = "panel"` lo deja en un
recuadro al lado.

## Cómo funciona

1. Captura la región de pantalla que le has marcado, unas 5 veces por segundo.
2. Calcula una huella del contenido. Si no ha cambiado, no hace nada más — así
   el coste en reposo es casi cero.
3. Cuando cambia, **espera a que se estabilice** unas cuantas lecturas. Esto
   evita traducir el texto a medias mientras el juego lo escribe letra a letra.
4. Pasa la imagen por OCR con `Windows.Media.Ocr`, el motor que ya viene dentro
   de Windows 10 y 11. Además del texto, **devuelve las coordenadas de cada
   palabra**: de ahí sale la caja que hay que tapar.
5. Limpia el resultado: el OCR separa en "palabras" también el japonés, así que
   `積雪の様に` llega como `積 雪 の 様 に` y hay que juntarlo — sin cargarse los
   espacios legítimos entre palabras latinas.
6. Traduce (con caché, para no pagar dos veces por la misma frase).
7. Lo pinta en una ventana *layered* + *click-through*: siempre encima y el
   ratón la atraviesa, así que el juego ni se entera. En modo in-place la
   ventana se recoloca sobre el texto original, se rellena con el color
   dominante de la caja (muestreado del propio fotograma) y **la letra se
   encoge sola hasta que la frase cabe** — el castellano ocupa bastante más que
   el japonés.

## Requisitos

- Windows 10 o 11.
- Rust estable (`rustup`), solo para compilar.
- El **paquete de OCR del idioma origen** instalado en Windows. Se comprueba con
  `hakoyaku langs`. Para añadirlo:
  *Configuración → Hora e idioma → Idioma y región → Añadir idioma → (elige, p.ej.
  日本語) → Opciones → Reconocimiento óptico de caracteres.*
- Una clave de API si usas DeepL o Google. Con `libre` (LibreTranslate en Docker)
  o `openai` apuntando a Ollama no hace falta nada: va todo en local.

## Instalación

```powershell
git clone https://github.com/TU-USUARIO/hakoyaku
cd hakoyaku
cargo build --release
# el binario queda en target\release\hakoyaku.exe
```

## Puesta en marcha

**Lo más fácil: haz doble clic en `hakoyaku.exe`.** Se abre un asistente que
comprueba qué falta —OCR, región, clave— y te deja arreglarlo desde un menú, sin
tocar ningún fichero. La ventana no se cierra sola.

Si prefieres la línea de comandos:

```powershell
hakoyaku init      # crea hakoyaku.toml
hakoyaku langs     # ¿tienes el OCR de japonés?
hakoyaku pick      # marca las dos esquinas del cuadro de diálogo con F8
hakoyaku region    # dibuja el marco encima del juego: ¿encuadra bien?
$env:HAKOYAKU_API_KEY = "tu-clave-de-deepl"
hakoyaku dump      # comprueba qué está leyendo el OCR  ← el paso importante
hakoyaku run       # a jugar
hakoyaku run --lang en   # ...o en inglés, sin tocar el fichero
```

`pick` no dibuja un rectángulo arrastrando a propósito: eso exige una ventana a
pantalla completa por encima del juego y se pelea con los juegos en fullscreen.
En vez de eso pones el ratón en la esquina superior izquierda, pulsas **F8**, lo
pones en la inferior derecha y vuelves a pulsar **F8**. Funciona con el juego
delante.

Marca la región **un poco más grande** que el texto, pero sin meter dentro
elementos que cambien solos (relojes, barras de vida, animaciones de fondo): cada
cambio de píxel dispara una lectura nueva.

### Saber que está funcionando

Dos señales visuales, ambas activadas por defecto:

- **El marco naranja** alrededor de la región vigilada, dibujado por encima del
  juego. Si encuadra la caja de diálogo, vas bien. Se ajusta con
  `overlay.region_color` y `overlay.region_thickness`, y se apaga con
  `show_region = false` cuando ya no lo necesites.
- **El texto de reposo** en el panel de traducción (`overlay.idle_text`).
  Mientras no haya nada que traducir, el panel no se queda en negro: pone
  «esperando texto…». Así distingues «está esperando» de «se ha colgado».

`hakoyaku region` dibuja solo el marco, sin arrancar nada más, para comprobar el
encuadre antes de gastar cuota de API.

### `dump` es tu herramienta de ajuste

```powershell
hakoyaku dump --raw
```

Guarda dos BMP —lo que ve el OCR y la captura sin tocar— y te enseña las líneas
en crudo, el texto ya limpio, si pasaría los filtros y cómo lo traduce. Si algo
no funciona, el problema casi siempre se ve aquí.

## Ajuste fino

Todo vive en `hakoyaku.toml`. Lo que más se toca:

| Problema | Qué cambiar |
|---|---|
| Se descuadra al mover el juego | Ánclate a la ventana con la opción 1. |
| En modo ratón no detecta nada | Opción 6: pon el ratón sobre la caja y pulsa F8, te dice qué encuentra. Si dice que no está sobre una caja, **baja** `cursor.edge_tolerance` a 10. |
| No traduce nunca, aunque haya diálogo | La región es demasiado grande o el fondo está animado. Marca solo la caja de texto; si aun así, baja `capture.change_tolerance` a 5. |
| Relee sin parar en escenas con partículas | Sube `capture.change_tolerance` a 20. |
| El OCR lee mal o no lee nada | Sube `capture.upscale` a 3. Con texto pequeño o pixel-art es lo que más ayuda. |
| Texto claro sobre caja oscura y lee regular | `preprocess = "invert"`, o `"binarize"` si el fondo tiene textura. |
| Traduce frases a medias | Sube `capture.stable_frames` a 3 o 4. |
| Tarda en reaccionar al diálogo | Baja `capture.poll_ms` a 60 y `capture.cooldown_ms` a 0. La consola te dice cuántos ms se van en OCR y cuántos en traducir. |
| El OCR tarda demasiado | Baja `capture.upscale` a 2, o marca una región más pequeña. El coste crece con el cuadrado. |
| Se dispara con números del HUD | Sube `ocr.min_chars`, o deja `require_cjk = true`. |
| Asoman restos de las letras originales | Sube `overlay.inplace_padding` a 8 o 10. |
| El parche se ve de otro color | Fija `overlay.inplace_background = "#RRGGBB"` con el color real de la caja. |
| La traducción sale con letra diminuta | Sube `overlay.min_font_size`, o agranda la región. |
| Prefiero ver el japonés | `overlay.mode = "panel"` y `show_original = true`. |
| El panel tapa algo del juego | Cambia `overlay.position` o pon `"custom"` con `x`/`y`. |
| El marco encuadra mal | `hakoyaku pick` otra vez, y compruébalo con `hakoyaku region`. |
| Ya no quiero ver el marco | `overlay.show_region = false`. |
| Quiero ver también el japonés | `overlay.show_original = true` y una fuente con kanji (`Yu Gothic UI`, `Meiryo UI`). |

### Elegir backend de traducción

| Backend | Calidad con japonés | Coste | Notas |
|---|---|---|---|
| `deepl` | Muy buena | 500k caracteres/mes gratis | La opción por defecto. Las claves gratuitas acaban en `:fx` y el programa detecta solo el endpoint correcto. |
| `openai` | La mejor con diálogo | Gratis con Ollama en local | Capta el tono y el registro. Va más lento: prueba `stable_frames = 3`. |
| `google` | Buena | De pago | Cloud Translation v2 con clave de API. |
| `libre` | Regular | Gratis | LibreTranslate en Docker. Todo offline, nada sale del PC. |
| `none` | — | — | No traduce, solo enseña el OCR. Para depurar. |

Con Ollama basta con `backend = "openai"` y `model = "qwen2.5:7b"`; el endpoint
por defecto ya apunta a `localhost:11434`.

La clave se puede poner en el TOML, pero es mejor usar la variable de entorno
`HAKOYAKU_API_KEY`, que tiene prioridad. Así no se cuela en un commit.

## Modo «sigue al ratón»

La alternativa a marcar nada: **se traduce la caja que estés señalando con el
cursor**. Se activa con la opción 7 del menú.

Funciona porque en una novela visual el ratón está casi siempre sobre el cuadro
de diálogo — es donde se hace clic para avanzar. En cada vuelta se captura una
zona alrededor del cursor, se detectan los bordes de la caja que hay debajo, y
se traduce esa. Si mueves el ratón a otro cuadro (el nombre del personaje, un
menú), cambia solo.

Dos detalles que lo hacen usable:

- **Adherencia.** La detección de bordes baila unos píxeles entre fotogramas, así
  que si la caja nueva se parece lo bastante a la anterior se reutiliza. Sin eso
  el recuadro tiembla. Se ajusta con `cursor.stickiness`.
- **Descarte.** Si la caja detectada ocupa casi toda la zona de búsqueda,
  significa que el cursor estaba sobre el fondo y no sobre texto: se ignora.
- **Puedes apuntar a las letras.** Es lo natural: señalas el texto que quieres
  traducir. El color de fondo de la caja se estima con la mediana del vecindario
  —robusta, porque las letras son minoría— y desde ahí se busca el píxel de
  fondo más cercano para empezar. Al expandir se pueden cruzar hasta 90 píxeles
  de «algo que no es fondo», así que un kanji grande no corta la detección.
- **Solo el juego.** Se comprueba qué ventana está dibujada bajo el cursor, no
  solo que el punto caiga dentro del rectángulo del juego. Si tienes el
  explorador de archivos encima, el punto sigue estando dentro pero lo que se
  captura es el explorador.
- **Detección adaptativa.** Al buscar los bordes, la referencia de color se va
  mezclando con lo que encuentra: así sigue un degradado sin salirse, y se para
  en el salto brusco del borde.
- **Tolerancia en escalera.** No hay un número que acertar: se prueban varias
  tolerancias de menor a mayor y se coge la primera que da una caja creíble. Con
  una sola tolerancia el ajuste es imposible — demasiado ajustada corta el
  degradado, demasiado suelta cruza el borde y detecta media pantalla, y el
  margen entre ambas depende del juego.

**Reserva a la región marcada.** El cuadro de diálogo principal de muchos juegos
es una banda semitransparente sin borde, con el fondo transparentándose: ahí la
detección por bordes no tiene a qué agarrarse. Pero ese cuadro siempre está en
el mismo sitio, así que si no encuentra caja bajo el ratón y hay una región
marcada, usa esa. Lo mejor de los dos modos: **marca la región del diálogo
principal *y* activa el modo ratón**, y tendrás el diálogo siempre y los botones
cuando los señales.

**Sin filtro de longitud al señalar.** Los botones ponen `はい` (2 caracteres) o
`僕` (1). Con `ocr.min_chars = 4` no se traducirían nunca — pero si has apuntado
ahí a propósito, quieres eso traducido. El mínimo solo se aplica a la región
fija.

Conviene combinarlo con el anclaje a la aplicación (opción 1), así no intenta
detectar cajas cuando sacas el ratón del juego.

## Marcar el cuadro de diálogo

Dos formas, ambas en la opción 1 del menú:

**Pinchando dentro (recomendado).** Pones el ratón en una zona sin letras del
cuadro de texto, pulsas `F8`, y hakoyaku detecta los bordes solo: toma el color
de fondo de la caja y avanza en las cuatro direcciones mientras la fila o
columna siga siendo mayoritariamente de ese color.

**Dos esquinas.** El método de siempre: `F8` en la esquina superior izquierda,
`F8` en la inferior derecha. Útil cuando la caja no tiene un fondo uniforme o
quieres recortar solo una parte.

## Anclarse a una aplicación

Por defecto la región va en coordenadas absolutas de pantalla, así que si mueves
la ventana del juego el recuadro se queda mirando donde estaba. Para evitarlo,
la opción 1 del menú te deja **elegir la ventana del juego de una lista**; a
partir de ahí la región se guarda relativa a su área de cliente y le sigue,
muevas o redimensiones lo que muevas.

De propina, con `only_when_focused = true` (por defecto) no se traduce nada
mientras el juego no tenga el foco. Se acabó ver al programa traduciendo el
navegador que tenías detrás.

El título se guarda recortado a su parte estable: de
`サキュバスプリズン-乳夢帰還-V1.00` se queda con `サキュバスプリズン`, porque
los números de versión y los contadores de FPS cambian solos. Puedes editarlo a
mano en `[target] window_title` si hiciera falta.

## Atajos mientras juegas

Se configuran en la sección `[hotkeys]`, porque cualquier tecla que viniera
fijada chocaría con algún juego: unos usan las F para guardar partida, otros el
espacio para avanzar el diálogo.

| Por defecto | Qué hace |
|---|---|
| `Ctrl+Espacio` | Quitar y devolver la traducción. Al devolverla se relee sola, así que reaparece al momento. |
| `Ctrl+Alt+P` | Pausar. Lo último traducido se queda en pantalla. |
| `Ctrl+Alt+R` | Releer ahora, sin esperar a que cambie el diálogo. |
| `Ctrl+Shift+Q` | Salir. |

Formato: `ctrl`, `shift`, `alt` y `win` como modificadores, más una tecla —una
letra, un número, `f1`..`f12`, `space`, `tab`, `esc`, `enter`, `supr`, `inicio`,
`fin`, `repag`, `avpag`, flechas o `tilde`. Los nombres en castellano también
valen. Un campo vacío desactiva ese atajo.

Conviene que lleven modificador: una tecla suelta se la come el juego, o se
dispara sola al escribir.

## Problemas conocidos

**Parpadea, retraduce sin parar o se queda pillado.** El recuadro se dibuja
encima de la región vigilada, así que la captura podría recoger nuestro propio
texto y entrar en bucle. Se evita con `SetWindowDisplayAffinity`, que hace la
ventana invisible para cualquier API de captura. Si tu versión de Windows no lo
soporta, hakoyaku lo avisa al arrancar: en ese caso sube `capture.cooldown_ms`
a 2000 o más.


**Se ve todo negro / no lee nada.** El juego está en pantalla completa exclusiva.
`BitBlt` no puede capturar ahí. Ponlo en **ventana** o **borderless**; casi todos
los juegos de RPG Maker y similares lo permiten.

**La región capturada está desplazada.** Suele pasar con escalado de pantalla
distinto de 100%. El programa se declara DPI-aware, pero si marcaste la región
con una versión anterior o a mano, vuelve a hacer `hakoyaku pick`.

**El overlay no aparece.** Algunos juegos con anti-cheat o con overlay propio
pelean por estar encima. Prueba `hakoyaku run --console` para confirmar que el
resto de la cadena funciona.

**Error 429 o 456 de la API.** Has llegado al límite de peticiones o de cuota.
Sube `capture.poll_ms` y `stable_frames`, y comprueba que la región no incluye
nada que parpadee.

## Estructura del proyecto

La lógica está partida en dos capas a propósito:

```
src/
  text.rs        limpieza del texto del OCR         ─┐
  frame.rs       imagen, huella, escalado, BMP       │  portable,
  cache.rs       caché de traducciones               │  se testea en
  config.rs      TOML y validación                   │  cualquier sistema
  pipeline.rs    detector de estabilidad y bucle     │
  translate/     deepl, google, libre, openai       ─┘
  capture.rs     trait + GDI (BitBlt)               ─┐
  ocr.rs         trait + Windows.Media.Ocr           │  solo Windows,
  overlay.rs     colocación (portable) + ventana     │  detrás de traits
  outline.rs     marco de la región vigilada          │
  picker.rs      marcar región con F8                │
  target.rs      anclaje a la ventana del juego      │
  cursor.rs      seguimiento del ratón              ─┘
```

Cada backend de traducción separa *construir la petición* y *parsear la
respuesta* (funciones puras, testeadas) de *mandarla por HTTP*. Y el pipeline
está escrito contra traits, no contra implementaciones, así que el test de
integración monta la cadena entera con mocks y comprueba el comportamiento sin
abrir una ventana, sin tocar la pantalla y sin salir a la red.

## Tests

```powershell
cargo test
```

202 tests: 188 unitarios y 14 de integración. El CI compila y los ejecuta en
Windows, y además corre en Linux la parte portable para que no se cuele lógica
de negocio dentro de un `#[cfg(windows)]`.

## Estado

Versión 0.1. Lo que falta, por orden de utilidad:

- Atajo global para pausar/reanudar sin ir a la consola.
- Reajustar la región en caliente, arrastrando el marco.
- Historial de las últimas frases traducidas.
- Varias regiones a la vez (diálogo + nombre del personaje).
- Detección automática de la caja de diálogo en vez de marcarla a mano.
- **Interfaz gráfica de verdad** (egui): ventana principal con ajustes, marco de
  región ajustable arrastrando con el ratón, perfiles por juego y atajos
  globales. Hoy hay un asistente de consola que cubre lo esencial.

## Licencia

MIT. Ver [LICENSE](LICENSE).
