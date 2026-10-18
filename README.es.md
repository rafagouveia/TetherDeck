# TetherDeck

[English](README.md) · [Português](README.pt-BR.md)

![TetherDeck](assets/brand/tetherdeck-banner.png)

**Un espacio de trabajo nativo para conectar, visualizar y automatizar un iPhone desde Windows.**

Proyecto gratuito y de código abierto en Rust, actualmente en vista previa
experimental y centrado en Windows. Integra pantalla en vivo independiente,
contacto continuo con el ratón, controles del dispositivo, inspección de elementos
y diagnóstico de conexión.

## Primeros pasos

1. Instala el soporte de dispositivos Apple en Windows y establece el emparejamiento USB.
2. Proporciona FFmpeg externo con decodificación HEVC para la pantalla en vivo.
3. Abre **TetherDeck.exe**, elige el transporte, busca el iPhone y abre una sesión.
4. Utiliza la pantalla remota, elementos, diagnóstico e integraciones.

Mantén los ejecutables del paquete en la misma carpeta. La interfaz permite elegir
idioma, apariencia, movimiento reducido y descubrimiento automático.

Consulta la [guía de inicio](docs/GETTING-STARTED.es.md), los
[requisitos del runner](docs/RUNNER-IMPORT.md) y la [arquitectura](docs/ARCHITECTURE.md).
Las instrucciones de compilación están en el [README principal](README.md#build-from-source).

## Certificado y Wi-Fi

La aplicación de Windows no necesita un certificado Apple. La automatización por
elementos utiliza un runner WebDriverAgent firmado por tu equipo para tus dispositivos.
Cada usuario proporciona su propio IPA; el certificado personal del autor no se distribuye.
Un `.cer` por sí solo no firma una aplicación. La importación no recibe claves privadas,
contraseñas ni credenciales Apple. iOS valida la firma durante la instalación USB.

Wi-Fi puede requerir preparación inicial mediante USB. Wi-Fi Apple/usbmux, Wi-Fi
directo emparejado y un túnel RSD son conexiones distintas. El túnel externo debe
estar activo antes de seleccionar esa opción.

Los FPS de pintura, la edad local del cuadro y la respuesta de la API se muestran
por separado; no miden la latencia completa. 30/60 FPS son objetivos de rendimiento,
no una garantía para todos los dispositivos, redes o equipos. El foco actual es Windows GPUI.

## Comunidad

Comparte problemas y sugerencias en [Issues](https://github.com/rafagouveia/TetherDeck/issues).
Se aceptan informes en español, portugués e inglés. Elimina tokens, identificadores
del dispositivo, direcciones y contenido personal antes de publicar capturas o diagnósticos.

Consulta [cómo contribuir](CONTRIBUTING.md), [conducta](CODE_OF_CONDUCT.md),
[seguridad](SECURITY.md) y [licencias de terceros](docs/THIRD-PARTY.md).
Código e identidad visual originales bajo [MIT](LICENSE).
