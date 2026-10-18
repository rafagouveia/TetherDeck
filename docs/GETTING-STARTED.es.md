# Primeros pasos en Windows

[English](GETTING-STARTED.en.md) · [Português](GETTING-STARTED.md)

## Requisitos

Windows x86_64 con gráficos DirectX 11, soporte de dispositivos Apple para Windows,
un iPhone desbloqueado y emparejado, y FFmpeg externo con decodificación HEVC para
la pantalla en vivo. La automatización por elementos requiere un runner
WebDriverAgent XCTest compatible, firmado y aprovisionado para tu dispositivo.

Los programas de TetherDeck están escritos en Rust y no requieren Python. Un túnel
RSD externo puede tener sus propios requisitos.

## Conexión

1. Mantén juntos los ejecutables del paquete y abre **TetherDeck.exe**.
2. Conecta y desbloquea el iPhone; acepta la solicitud de confianza si aparece.
3. Elige USB, busca el dispositivo y abre su sesión.
4. Si falla, revisa diagnóstico: presencia USB, servicio Apple, usbmux y acceso
   emparejado son capas diferentes. La recuperación necesita autorización y permisos.

Para Wi-Fi, prepara primero el emparejamiento USB cuando sea necesario. Mantén
ambos equipos en una red local accesible y elige el transporte apropiado.
Wi-Fi Apple/usbmux, Wi-Fi directo emparejado y un túnel RSD son caminos distintos.
La opción RSD necesita un túnel local ya activo; TetherDeck no lo crea automáticamente.
No compartas registros de emparejamiento remoto.

Una vez verificado el camino de red, sus funciones compatibles operan sin cable.
Vídeo/contacto y elementos tienen estados de disponibilidad separados.

## Pantalla en vivo

Abre la pantalla independiente después de conectar. El marco tiene fondo externo
transparente, cabecera arrastrable, cuatro esquinas de tamaño proporcional y controles
flotantes. Presiona para tocar, mueve mientras mantienes el botón pulsado y suelta
para levantar el dedo. Pausa, vídeo antiguo y cancelación liberan el contacto.
Los comandos fallidos no se repiten automáticamente.

Las estadísticas muestran FPS de pintura GPUI, intervalo derivado, edad local del
cuadro y respuesta de la API. No miden la latencia completa entre ratón y dispositivo.

Configura `TETHERDECK_FFMPEG`, coloca `ffmpeg.exe` junto al programa, utiliza
`%ProgramFiles%\ffmpeg\ffmpeg.exe` o `PATH`, en ese orden. FFmpeg no está incluido.

## Elegir proveedor

En **Elementos**, elige **Automático**, **WebDriverAgent** o **Accesibilidad iOS**.
AX inspecciona mediante el túnel emparejado, incluso por Wi-Fi, sin runner ni certificado
WDA. Su recorrido puede desplazar la aplicación y no permite activación genérica.
WDA ofrece jerarquía y activación mediante referencias recientes; la respuesta indica
el proveedor utilizado.

Para WDA por Wi-Fi revisa que el runner limite HTTP y MJPEG a la dirección interna del
túnel. Después habilita **Runner preparado para Wi-Fi aislado** para ese Bundle ID en
las opciones de sesión. Está desactivado inicialmente; cambiar el Bundle ID o importar
otro IPA invalida la revisión. El nombre o la versión por sí solos no bastan. Consulta
[proveedores y requisitos](ELEMENT-PROVIDERS.md). AX funciona sin esta opción.

## Runner y herramientas

Importa un `.ipa` WDA ya firmado, revisa sus metadatos e inicia explícitamente la
instalación por USB emparejado. El Bundle ID se completa cuando iOS confirma la
instalación; abre una nueva sesión compatible para activarlo. Consulta
[importación del runner](RUNNER-IMPORT.md).

Un `.cer` por sí solo no firma una aplicación. Cada equipo proporciona su propia
identidad y perfil; el certificado del autor no se distribuye. TetherDeck no recibe
claves privadas, contraseñas ni credenciales Apple, y no compila Xcode en Windows.

Puedes elegir idioma del sistema/español/portugués/inglés, apariencia, movimiento
reducido y descubrimiento automático. Los textos del dispositivo permanecen originales.
MCP y CLI usan la API autenticada. La configuración MCP exportada contiene un token;
no la adjuntes a una issue.

Para solicitar ayuda, publica pasos reproducibles y diagnósticos sin datos personales.
No adjuntes claves, perfiles, registros de emparejamiento ni IPAs personales.
