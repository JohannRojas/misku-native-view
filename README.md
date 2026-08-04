# Misku Native Views

Misku Native Views convierte URLs en aplicaciones de escritorio independientes para Windows mediante Tauri 2 y Microsoft Edge WebView2. El comando público del paquete es y seguirá siendo:

```powershell
misku-nv
```

## Modelo de aplicaciones

Cada creación genera una instancia nueva, incluso si la URL ya existe:

```powershell
misku-nv https://github.com/openai
misku-nv https://github.com/openai
```

Las dos aplicaciones anteriores tienen UUID, perfil WebView2, manifiesto, estado de ventana y acceso directo diferentes. El `id` es un alias legible; el UUID es la identidad estable de la aplicación y no cambia al actualizarla.

Una instalación nueva administrada usa, por defecto, esta estructura:

```text
%LOCALAPPDATA%\Misku Native Views\
├── apps.toml
├── icons\
│   └── <uuid>-<asset-id>.ico
├── apps\
│   └── <uuid>\
│       ├── app.toml
│       ├── install.json
│       └── icons\
│           └── <uuid>.ico
└── runtimes\
    └── <version-hash>\
        └── misku-native-views.exe
```

Los accesos directos se instalan por separado en el menú Inicio del usuario. Una actualización del paquete puede añadir un runtime versionado sin sobrescribir el ejecutable que usa otra instancia.

## Requisitos

Para usar el paquete publicado:

- Windows 10 u 11 x64.
- Microsoft Edge WebView2 Evergreen Runtime.

Para desarrollar o compilar desde el repositorio:

- Rust 1.88 o posterior con el toolchain MSVC.
- Microsoft Visual Studio Build Tools con “Desktop development with C++”.
- Node.js y pnpm para empaquetar el CLI.
- Python y Pillow solamente para convertir imágenes a `.ico`.

## Instalar el CLI

```powershell
pnpm install -g misku-native-view-cli
misku-nv --version
```

Para probar un tarball local:

```powershell
pnpm pack
pnpm install -g .\misku-native-view-cli-<version>.tgz
```

El paquete incluye el wrapper `misku-nv`, el runtime nativo y el administrador seguro de accesos directos. Un usuario final no necesita Cargo ni Visual Studio.

## Uso

Crear y abrir una aplicación:

```powershell
misku-nv https://github.com --name GitHub
```

Crear sin abrir:

```powershell
misku-nv https://github.com/openai --name "GitHub OpenAI" --no-open
```

Por defecto, el CLI descubre el favicon de la página, lo valida y lo normaliza a un `.ico` multirresolución. El recurso administrado usa un nombre versionado en `icons/<uuid>-<asset-id>.ico` y cada app recibe su propia copia estable en `apps/<uuid>/icons/<uuid>.ico`. Si la página está offline o no ofrece un PNG/ICO válido, la creación continúa con el icono genérico. Para evitar cualquier consulta:

```powershell
misku-nv https://example.com --no-favicon
```

Crear varias aplicaciones en una operación:

```powershell
misku-nv create https://github.com/openai https://github.com/microsoft
```

`create`, `add` y la forma abreviada con una URL siempre crean una instancia. Nunca actualizan implícitamente una aplicación existente. Para modificar una instancia se requiere `update`:

```powershell
misku-nv --list
misku-nv --list --json
misku-nv update <uuid> --name "Nuevo nombre"
misku-nv update <uuid> --url https://example.com/nueva-ruta
misku-nv update <uuid> --refresh-icon
misku-nv update <uuid> --url https://example.com/otra --refresh-icon
misku-nv remove <uuid>
misku-nv remove <uuid> --purge-data
```

`remove` conserva cookies y sesiones salvo que se indique `--purge-data`. Es preferible usar el UUID en automatizaciones; el alias `id` también puede usarse mientras sea inequívoco.

Para reparar manifiestos y accesos directos administrados:

```powershell
misku-nv repair
```

Para usar un registro específico:

```powershell
misku-nv --config C:\ruta\apps.toml --list
```

El CLI instalado usa su registro administrado en `%LOCALAPPDATA%`; no adopta silenciosamente un `apps.toml` del directorio de trabajo.

Al actualizar desde la versión 0.1, si todavía no existe un registro local pero sí `%APPDATA%\Misku Native Views\apps.toml`, el CLI continúa usando ese registro legado. Sus iconos y sesiones WebView aisladas permanecen en la ubicación anterior; las apps nuevas usan almacenamiento local por UUID. Los perfiles legados con almacenamiento compartido se migran a aislamiento y pueden requerir iniciar sesión de nuevo. `--purge-data` revisa ambas ubicaciones.

## Seguridad y navegación

- Se acepta HTTPS por defecto.
- HTTP requiere `--allow-http` y solamente se admite para `localhost` o direcciones loopback.
- Las URLs con credenciales embebidas se rechazan.
- La WebView permanece en el origen configurado y en los orígenes añadidos con `--allow-origin`.
- Los enlaces HTTPS hacia otros orígenes se abren en el navegador del sistema.
- Esquemas peligrosos se bloquean. Todas las ventanas nuevas HTTPS se abren en el navegador del sistema, incluso si su origen está permitido; los flujos OAuth que dependan de compartir cookies mediante popup pueden requerir una integración específica.
- Cada UUID usa un directorio de datos WebView y un archivo de estado de ventana propios.
- El registro se escribe de forma atómica y se bloquea durante las mutaciones.
- Iconos, alias y rutas administradas se validan antes de crear o eliminar archivos.
- El favicon usa peticiones sin cookies ni credenciales, redirects manuales, DNS fijado, límites de tiempo/tamaño y únicamente recursos del mismo origen.
- HTTPS es obligatorio para favicons públicos; HTTP solo funciona para loopback con `--allow-http`. Un fallo de descarga nunca impide crear la app.

Ejemplo de origen adicional:

```powershell
misku-nv https://example.com --allow-origin https://login.example.com
```

## Desarrollo

El helper de desarrollo pasa explícitamente el `apps.toml` del repositorio:

```powershell
.\scripts\misku.ps1 --list
.\scripts\misku.ps1 create https://example.com
.\scripts\misku.ps1 <id-o-uuid>
```

También se puede usar el entorno de compilación directamente:

```powershell
.\scripts\dev.ps1 check --workspace --locked
.\scripts\dev.ps1 test --workspace --locked
.\scripts\dev.ps1 run -p misku-native-views "--" --config .\apps.toml --list
```

## Registro `apps.toml`

El registro admite configuraciones heredadas sin identidad explícita; al cargarlas asigna una identidad estable y, en la siguiente escritura, las migra al esquema actual.

```toml
schema_version = 2

[[apps]]
instance_id = "11111111-1111-4111-8111-111111111111"
profile_key = "11111111-1111-4111-8111-111111111111"
id = "tftacademy"
name = "TFT Academy"
url = "https://tftacademy.com/tierlist/comps/"
icon = "icons/tftacademy.ico"
width = 1280
height = 860
min_width = 900
min_height = 640
isolated_profile = true
devtools = false
resizable = true
zoom_hotkeys_enabled = true
```

No reutilices manualmente `instance_id` ni `profile_key`: deben ser únicos. Para crear perfiles usa `misku-nv`; la edición manual se reserva para opciones visuales avanzadas.

Campos principales:

- `instance_id`: UUID inmutable de la instancia.
- `profile_key`: clave inmutable del almacenamiento WebView.
- `id`: alias legible y único dentro del registro.
- `name`: título de ventana.
- `url`: URL inicial.
- `icon`: `.ico` o `.png` relativo al registro.
- `allowed_origins`: orígenes adicionales permitidos dentro de la WebView.
- `allow_insecure_http`: opt-in de HTTP loopback.
- `width`, `height`, `min_width`, `min_height`: dimensiones de ventana.
- `isolated_profile`: debe permanecer en `true`; el esquema actual rechaza almacenamiento WebView compartido.
- `devtools`, `user_agent`, `resizable`, `zoom_hotkeys_enabled`: opciones del runtime.

## Portables

Genera un portable por UUID:

```powershell
.\scripts\build-portable.ps1
```

La salida no depende del alias ni del host de la URL:

```text
portable\
├── apps.json
└── <uuid>\
    ├── misku-native-views.exe
    ├── apps.toml
    └── icons\
        └── <uuid>.ico
```

`apps.json` proviene de `--list --json` y cada `apps.toml` se genera con el comando nativo `export`; el script no interpreta TOML con expresiones regulares. Por ello, dos rutas del mismo host o dos copias de la misma URL siguen siendo portables separados.

Ejecuta una instancia directamente:

```powershell
.\portable\<uuid>\misku-native-views.exe
```

Los `.lnk` no se precalculan dentro de `portable`. Para crear accesos directos en su destino final:

```powershell
.\scripts\install-start-menu.ps1
.\scripts\install-start-menu.ps1 -Uninstall
```

El desinstalador solo elimina los `.lnk` registrados como propios y elimina su carpeta administrada únicamente si queda vacía; no realiza borrados recursivos.

## Iconos

Sin `--icon`, cada creación intenta usar `link rel="icon"`, `shortcut icon`, `apple-touch-icon`, el Web Manifest y finalmente `/favicon.ico`. El resultado queda asociado al UUID y persiste durante `repair`, actualizaciones y reinicios. Los nombres centrales versionados permiten reemplazar un icono de forma atómica, mientras que la copia instalada conserva la ruta `apps/<uuid>/icons/<uuid>.ico` que usa el acceso directo. `--icon` siempre tiene prioridad; `--refresh-icon` reemplaza explícitamente el favicon sin cambiar el UUID, perfil ni sesiones.

La descarga acepta PNG o ICO y Rust vuelve a decodificar la imagen antes de generar el ICO que consumen Tauri y Windows. Para convertir manualmente otros formatos:

```powershell
.\scripts\convert-icon.ps1 .\downloads\logo.webp .\icons\myapp.ico
.\scripts\convert-icon.ps1 .\downloads\logo.webp .\icons\myapp.ico -Background "#FFFFFF"
```

Después impórtala mediante el CLI:

```powershell
misku-nv https://example.com --icon .\icons\myapp.ico
```

## CI y publicación

Los workflows de GitHub compilan y prueban el runtime, empaquetan el tarball de pnpm y ejecutan una prueba de humo del comando `misku-nv`. La publicación requiere el secreto `NPM_TOKEN`.

No deben versionarse sesiones privadas ni resultados generados como `target/`, `portable/`, `runtime/`, `screenshots/`, `artifacts/`, `.env*` o logs.
