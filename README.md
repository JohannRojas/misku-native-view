# Misku Native Views

Misku Native Views is a small Windows-first launcher that turns configured web URLs into lightweight desktop apps using Rust, Tauri 2, and Microsoft Edge WebView2.

It is inspired by tools like Pake, but keeps the workflow simple: edit `apps.toml`, build portable executables, and optionally install Start Menu shortcuts.

## Features

- Multiple web apps from one `apps.toml` file.
- One portable `.exe` copy per app id.
- Isolated WebView2 profile directories per app.
- Custom window and shortcut icons.
- CLI command to add or update apps by URL.
- Optional Start Menu shortcut installation.
- Cargo target output outside the repo to avoid OneDrive lock/sync issues.

## Requirements

- Windows 11.
- Rust stable with the MSVC toolchain.
- Microsoft C++ Build Tools with Desktop development with C++.
- Microsoft Edge WebView2 Evergreen Runtime.
- Python + Pillow only if you want to convert `.webp`, `.png`, or `.jpg` files into `.ico` icons.

## Configure Apps

Apps live in `apps.toml`:

```toml
[[apps]]
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

Supported fields:

- `id`: short identifier used by the CLI and executable name.
- `name`: window title.
- `url`: `https://` or `http://` URL.
- `icon`: optional `.ico` or `.png`, relative to `apps.toml`.
- `width`, `height`: initial window size.
- `min_width`, `min_height`: minimum window size.
- `isolated_profile`: separates cookies, cache, and sessions per app.
- `devtools`: enables DevTools for that app.
- `user_agent`: optional custom user agent.
- `resizable`: allows resizing the window.
- `zoom_hotkeys_enabled`: enables native zoom shortcuts.

## CLI

Use the helper script during development:

```powershell
.\scripts\misku.ps1 --list
.\scripts\misku.ps1 add https://chatgpt.com
.\scripts\misku.ps1 add --id tftacademy --name "TFT Academy" https://tftacademy.com/tierlist/comps/
.\scripts\misku.ps1 --app tftacademy
```

`add` creates a new profile when the app does not exist. If the generated `id`, explicit `--id`, or exact URL already exists, it updates the existing profile instead of creating a duplicate.

For raw Cargo commands, use `scripts/dev.ps1` so Visual Studio Build Tools are loaded and Cargo output goes to `%LOCALAPPDATA%`:

```powershell
.\scripts\dev.ps1 check
.\scripts\dev.ps1 run -p misku-native-views "--" --list
```

## Build Portable Apps

Generate `portable/` with one executable per configured app:

```powershell
.\scripts\build-portable.ps1
```

Run an app directly:

```powershell
.\portable\tftacademy.exe
```

If a profile defines `icon`, the build script also creates a matching `.lnk` shortcut with that icon. The `.exe` keeps the base embedded app icon; use the generated shortcut for the per-app Explorer/Start Menu icon.

## Start Menu Shortcuts

Install generated shortcuts for the current Windows user:

```powershell
.\scripts\install-start-menu.ps1
```

Remove them:

```powershell
.\scripts\install-start-menu.ps1 -Uninstall
```

## Icons

Convert image files into multi-size Windows `.ico` files:

```powershell
.\scripts\convert-icon.ps1 .\downloads\logo.webp .\icons\myapp.ico
```

Use a solid padding background when needed:

```powershell
.\scripts\convert-icon.ps1 .\downloads\logo.webp .\icons\myapp.ico -Background "#FFFFFF"
```

Then reference the icon from `apps.toml`:

```toml
icon = "icons/myapp.ico"
```

## Publishing Notes

The repository intentionally ignores generated and local-only files such as:

- `target/`
- `src-tauri/target/`
- `portable/`
- `screenshots/`
- `.env` and `.env.*`
- logs

Do not commit private sessions, generated portable executables, local screenshots, or environment files.