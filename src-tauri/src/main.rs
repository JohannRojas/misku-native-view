#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, image::Image};
use url::Url;

const DEFAULT_CONFIG_FILE: &str = "apps.toml";
const MAIN_WINDOW_LABEL: &str = "main";

#[derive(Debug)]
struct LaunchArgs {
    config_path: Option<PathBuf>,
    command: Command,
}

#[derive(Debug)]
enum Command {
    Open { app_id: Option<String> },
    List,
    Upsert(UpsertArgs),
}

#[derive(Debug)]
struct UpsertArgs {
    urls: Vec<String>,
    id: Option<String>,
    name: Option<String>,
    icon: Option<String>,
}

#[derive(Debug)]
enum UpsertAction {
    Created,
    Updated,
}

#[derive(Debug)]
struct UpsertResult {
    action: UpsertAction,
    id: String,
    url: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct AppsConfig {
    #[serde(default)]
    apps: Vec<AppProfile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AppProfile {
    id: String,
    name: String,
    url: String,
    #[serde(default = "default_width")]
    width: f64,
    #[serde(default = "default_height")]
    height: f64,
    #[serde(default = "default_min_width")]
    min_width: f64,
    #[serde(default = "default_min_height")]
    min_height: f64,
    #[serde(default = "default_true")]
    isolated_profile: bool,
    #[serde(default)]
    devtools: bool,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default = "default_true")]
    resizable: bool,
    #[serde(default = "default_true")]
    zoom_hotkeys_enabled: bool,
    #[serde(default)]
    icon: Option<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("misku-native-views: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(env::args().skip(1))?;

    match args.command {
        Command::List => {
            let config_path = resolve_config_path(args.config_path.as_deref())?;
            let config = load_config(&config_path)?;
            print_profiles(&config);
            Ok(())
        }
        Command::Upsert(upsert_args) => {
            let config_path = resolve_config_path_for_write(args.config_path.as_deref())?;
            let mut config = load_config_for_write(&config_path)?;
            let results = upsert_profiles(&mut config, &config_path, upsert_args)?;
            save_config(&config_path, &config)?;
            print_upsert_results(&config_path, &results);
            Ok(())
        }
        Command::Open { app_id } => {
            let config_path = resolve_config_path(args.config_path.as_deref())?;
            let config = load_config(&config_path)?;
            let inferred_id = infer_profile_id_from_exe();
            let profile = select_profile(&config, app_id.as_deref().or(inferred_id.as_deref()))?;

            tauri::Builder::default()
                .plugin(tauri_plugin_window_state::Builder::default().build())
                .setup(move |app| {
                    open_profile_window(app, &profile, &config_path)?;
                    Ok(())
                })
                .run(tauri::generate_context!())
                .map_err(|error| error.to_string())
        }
    }
}

fn parse_args<I>(args: I) -> Result<LaunchArgs, String>
where
    I: IntoIterator<Item = String>,
{
    let mut app_id = None;
    let mut config_path = None;
    let mut list = false;
    let mut iterator = args.into_iter();

    while let Some(arg) = iterator.next() {
        match arg.as_str() {
            "--app" | "-a" => {
                let value = iterator
                    .next()
                    .ok_or_else(|| "falta el valor despues de --app".to_string())?;
                app_id = Some(value);
            }
            "--config" | "-c" => {
                let value = iterator
                    .next()
                    .ok_or_else(|| "falta el valor despues de --config".to_string())?;
                config_path = Some(PathBuf::from(value));
            }
            "--list" | "-l" => list = true,
            "add" | "upsert" => {
                let upsert_args = parse_upsert_args(iterator, &mut config_path)?;
                return Ok(LaunchArgs {
                    config_path,
                    command: Command::Upsert(upsert_args),
                });
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            value if value.starts_with('-') => {
                return Err(format!("argumento desconocido: {value}"));
            }
            value => {
                if app_id.is_some() {
                    return Err(format!("perfil duplicado o argumento inesperado: {value}"));
                }
                app_id = Some(value.to_string());
            }
        }
    }

    let command = if list {
        Command::List
    } else {
        Command::Open { app_id }
    };

    Ok(LaunchArgs {
        config_path,
        command,
    })
}

fn parse_upsert_args<I>(args: I, config_path: &mut Option<PathBuf>) -> Result<UpsertArgs, String>
where
    I: IntoIterator<Item = String>,
{
    let mut urls = Vec::new();
    let mut id = None;
    let mut name = None;
    let mut icon = None;
    let mut iterator = args.into_iter();

    while let Some(arg) = iterator.next() {
        match arg.as_str() {
            "--id" | "-i" => {
                let value = iterator
                    .next()
                    .ok_or_else(|| "falta el valor despues de --id".to_string())?;
                id = Some(value);
            }
            "--name" | "-n" => {
                let value = iterator
                    .next()
                    .ok_or_else(|| "falta el valor despues de --name".to_string())?;
                name = Some(value);
            }
            "--icon" => {
                let value = iterator
                    .next()
                    .ok_or_else(|| "falta el valor despues de --icon".to_string())?;
                icon = Some(value);
            }
            "--config" | "-c" => {
                let value = iterator
                    .next()
                    .ok_or_else(|| "falta el valor despues de --config".to_string())?;
                *config_path = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                print_add_help();
                std::process::exit(0);
            }
            value if value.starts_with('-') => {
                return Err(format!("argumento desconocido para add: {value}"));
            }
            value => urls.push(value.to_string()),
        }
    }

    if urls.is_empty() {
        return Err("add necesita al menos una URL".to_string());
    }

    if urls.len() > 1 && (id.is_some() || name.is_some() || icon.is_some()) {
        return Err(
            "--id, --name y --icon solo se pueden usar cuando agregas una sola URL".to_string(),
        );
    }

    Ok(UpsertArgs {
        urls,
        id,
        name,
        icon,
    })
}

fn resolve_config_path(explicit_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit_path {
        return path
            .canonicalize()
            .map_err(|error| format!("no se pudo leer {}: {error}", path.display()));
    }

    let cwd_path = env::current_dir()
        .map_err(|error| format!("no se pudo resolver el directorio actual: {error}"))?
        .join(DEFAULT_CONFIG_FILE);
    if cwd_path.exists() {
        return cwd_path.canonicalize().map_err(|error| {
            format!(
                "no se pudo resolver la configuracion {}: {error}",
                cwd_path.display()
            )
        });
    }

    let exe_path = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(DEFAULT_CONFIG_FILE)));
    if let Some(path) = exe_path {
        if path.exists() {
            return path.canonicalize().map_err(|error| {
                format!(
                    "no se pudo resolver la configuracion {}: {error}",
                    path.display()
                )
            });
        }
    }

    Err(format!(
        "no encontre {DEFAULT_CONFIG_FILE}; crea uno o usa --config <ruta>"
    ))
}

fn resolve_config_path_for_write(explicit_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit_path {
        if path.exists() {
            return path
                .canonicalize()
                .map_err(|error| format!("no se pudo leer {}: {error}", path.display()));
        }

        return absolute_path(path);
    }

    let cwd_path = env::current_dir()
        .map_err(|error| format!("no se pudo resolver el directorio actual: {error}"))?
        .join(DEFAULT_CONFIG_FILE);
    if cwd_path.exists() {
        return cwd_path.canonicalize().map_err(|error| {
            format!(
                "no se pudo resolver la configuracion {}: {error}",
                cwd_path.display()
            )
        });
    }

    let exe_path = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(DEFAULT_CONFIG_FILE)));
    if let Some(path) = exe_path {
        if path.exists() {
            return path.canonicalize().map_err(|error| {
                format!(
                    "no se pudo resolver la configuracion {}: {error}",
                    path.display()
                )
            });
        }
    }

    Ok(cwd_path)
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|error| format!("no se pudo resolver el directorio actual: {error}"))
}

fn load_config(path: &Path) -> Result<AppsConfig, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("no se pudo leer {}: {error}", path.display()))?;
    let config: AppsConfig = toml::from_str(&content)
        .map_err(|error| format!("configuracion invalida en {}: {error}", path.display()))?;

    if config.apps.is_empty() {
        return Err(format!("{} no contiene ningun [[apps]]", path.display()));
    }

    validate_config(&config)?;
    Ok(config)
}

fn load_config_for_write(path: &Path) -> Result<AppsConfig, String> {
    if !path.exists() {
        return Ok(AppsConfig { apps: Vec::new() });
    }

    let content = fs::read_to_string(path)
        .map_err(|error| format!("no se pudo leer {}: {error}", path.display()))?;
    let config: AppsConfig = toml::from_str(&content)
        .map_err(|error| format!("configuracion invalida en {}: {error}", path.display()))?;

    validate_config(&config)?;
    Ok(config)
}

fn save_config(path: &Path, config: &AppsConfig) -> Result<(), String> {
    validate_config(config)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("no se pudo crear {}: {error}", parent.display()))?;
    }

    let content = toml::to_string_pretty(config)
        .map_err(|error| format!("no se pudo serializar apps.toml: {error}"))?;
    fs::write(path, content)
        .map_err(|error| format!("no se pudo escribir {}: {error}", path.display()))
}

fn validate_config(config: &AppsConfig) -> Result<(), String> {
    let mut ids = HashSet::new();
    for profile in &config.apps {
        validate_profile(profile)?;
        if !ids.insert(profile.id.clone()) {
            return Err(format!("id duplicado en apps.toml: {}", profile.id));
        }
    }

    Ok(())
}

fn validate_profile(profile: &AppProfile) -> Result<(), String> {
    if profile.id.trim().is_empty() {
        return Err("un perfil tiene id vacio".to_string());
    }

    let url = Url::parse(&profile.url)
        .map_err(|error| format!("url invalida en {}: {error}", profile.id))?;

    match url.scheme() {
        "https" | "http" => Ok(()),
        scheme => Err(format!(
            "url invalida en {}: esquema no soportado '{scheme}'",
            profile.id
        )),
    }
}

fn upsert_profiles(
    config: &mut AppsConfig,
    config_path: &Path,
    args: UpsertArgs,
) -> Result<Vec<UpsertResult>, String> {
    let mut results = Vec::new();

    for raw_url in args.urls {
        let url = parse_app_url(&raw_url)?;
        let id = match &args.id {
            Some(value) => normalize_id(value)?,
            None => infer_id_from_url(&url)?,
        };
        let generated_name = name_from_id(&id);
        let requested_name = args.name.clone();
        let requested_icon = args.icon.clone();
        let existing_index = find_existing_profile(config, args.id.as_deref(), &id, &url);

        if let Some(index) = existing_index {
            let profile = &mut config.apps[index];
            if args.id.is_some() {
                profile.id = id.clone();
            }
            if let Some(name) = requested_name {
                profile.name = name;
            } else if profile.name.trim().is_empty() {
                profile.name = generated_name;
            }
            if let Some(icon) = requested_icon {
                profile.icon = Some(icon);
            }
            profile.url = url.to_string();
            results.push(UpsertResult {
                action: UpsertAction::Updated,
                id: profile.id.clone(),
                url: profile.url.clone(),
            });
        } else {
            let icon = requested_icon.or_else(|| infer_icon_for_profile(config_path, &id));
            let profile = AppProfile {
                id: id.clone(),
                name: requested_name.unwrap_or(generated_name),
                url: url.to_string(),
                width: default_width(),
                height: default_height(),
                min_width: default_min_width(),
                min_height: default_min_height(),
                isolated_profile: default_true(),
                devtools: false,
                user_agent: None,
                resizable: default_true(),
                zoom_hotkeys_enabled: default_true(),
                icon,
            };

            results.push(UpsertResult {
                action: UpsertAction::Created,
                id,
                url: profile.url.clone(),
            });
            config.apps.push(profile);
        }
    }

    validate_config(config)?;
    Ok(results)
}

fn parse_app_url(raw: &str) -> Result<Url, String> {
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&candidate).map_err(|error| format!("url invalida '{raw}': {error}"))?;

    match url.scheme() {
        "https" | "http" => Ok(url),
        scheme => Err(format!(
            "url invalida '{raw}': esquema no soportado '{scheme}'"
        )),
    }
}

fn find_existing_profile(
    config: &AppsConfig,
    requested_id: Option<&str>,
    inferred_id: &str,
    url: &Url,
) -> Option<usize> {
    if let Some(requested_id) = requested_id {
        if let Ok(normalized_id) = normalize_id(requested_id) {
            if let Some(index) = config
                .apps
                .iter()
                .position(|profile| profile.id == normalized_id)
            {
                return Some(index);
            }
        }
    }

    let url_key = normalized_url_key(url.as_str());
    if let Some(index) = config
        .apps
        .iter()
        .position(|profile| normalized_url_key(&profile.url) == url_key)
    {
        return Some(index);
    }

    config
        .apps
        .iter()
        .position(|profile| profile.id == inferred_id)
}

fn normalized_url_key(raw: &str) -> Option<String> {
    let mut url = parse_app_url(raw).ok()?;
    url.set_fragment(None);
    Some(url.as_str().trim_end_matches('/').to_ascii_lowercase())
}

fn infer_id_from_url(url: &Url) -> Result<String, String> {
    let host = url
        .host_str()
        .ok_or_else(|| format!("url sin host: {url}"))?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').filter(|part| !part.is_empty()).collect();
    let base = if labels.len() <= 2 {
        labels.first().copied().unwrap_or(&host).to_string()
    } else {
        labels[..labels.len() - 1].join("-")
    };

    normalize_id(&base)
}

fn normalize_id(raw: &str) -> Result<String, String> {
    let mut id = String::new();
    let mut previous_was_separator = false;

    for character in raw.trim().to_ascii_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            id.push(character);
            previous_was_separator = false;
        } else if !previous_was_separator {
            id.push('-');
            previous_was_separator = true;
        }
    }

    let normalized = id.trim_matches('-').to_string();
    if normalized.is_empty() {
        return Err(format!("no se pudo generar un id valido desde '{raw}'"));
    }

    Ok(normalized)
}

fn name_from_id(id: &str) -> String {
    id.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn infer_icon_for_profile(config_path: &Path, id: &str) -> Option<String> {
    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    for extension in ["ico", "png"] {
        let relative_path = format!("icons/{id}.{extension}");
        if base_dir.join(&relative_path).exists() {
            return Some(relative_path);
        }
    }

    None
}

fn select_profile(config: &AppsConfig, requested_id: Option<&str>) -> Result<AppProfile, String> {
    match requested_id {
        Some(id) => config
            .apps
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
            .ok_or_else(|| format!("no existe el perfil '{id}'. Usa --list para ver opciones")),
        None => config
            .apps
            .first()
            .cloned()
            .ok_or_else(|| "no hay perfiles configurados".to_string()),
    }
}

fn infer_profile_id_from_exe() -> Option<String> {
    env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .filter(|stem| stem != "misku-native-views")
}

fn resolve_profile_icon_path(config_path: &Path, icon_path: Option<&str>) -> Option<PathBuf> {
    let icon_path = icon_path?;
    let icon_path = Path::new(icon_path);
    if icon_path.is_absolute() {
        return Some(icon_path.to_path_buf());
    }

    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    Some(base_dir.join(icon_path))
}

fn open_profile_window(
    app: &mut tauri::App,
    profile: &AppProfile,
    config_path: &Path,
) -> tauri::Result<()> {
    let url = Url::parse(&profile.url).map_err(tauri::Error::InvalidUrl)?;
    let profile_dir = if profile.isolated_profile {
        let directory = app
            .path()
            .app_data_dir()?
            .join("profiles")
            .join(&profile.id);
        fs::create_dir_all(&directory)?;
        Some(directory)
    } else {
        None
    };

    let mut builder = WebviewWindowBuilder::new(app, MAIN_WINDOW_LABEL, WebviewUrl::External(url))
        .title(&profile.name)
        .inner_size(profile.width, profile.height)
        .min_inner_size(profile.min_width, profile.min_height)
        .resizable(profile.resizable)
        .zoom_hotkeys_enabled(profile.zoom_hotkeys_enabled)
        .devtools(profile.devtools)
        .center()
        .prevent_overflow();

    if let Some(profile_dir) = profile_dir {
        builder = builder.data_directory(profile_dir);
    }

    if let Some(icon_path) = resolve_profile_icon_path(config_path, profile.icon.as_deref()) {
        builder = builder.icon(Image::from_path(icon_path)?)?;
    }

    if let Some(user_agent) = &profile.user_agent {
        builder = builder.user_agent(user_agent);
    }

    builder.build()?;
    Ok(())
}

fn print_profiles(config: &AppsConfig) {
    for profile in &config.apps {
        println!("{}  {}  {}", profile.id, profile.name, profile.url);
    }
}

fn display_path(path: &Path) -> String {
    let value = path.display().to_string();
    value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
}

fn print_upsert_results(config_path: &Path, results: &[UpsertResult]) {
    for result in results {
        let action = match result.action {
            UpsertAction::Created => "creado",
            UpsertAction::Updated => "actualizado",
        };
        println!("{action}: {} -> {}", result.id, result.url);
    }
    println!("configuracion: {}", display_path(config_path));
    println!("siguiente paso: .\\scripts\\build-portable.ps1");
}

fn print_help() {
    println!(
        "Misku Native Views\n\nUso:\n  misku-native-views [--config apps.toml] [--app id]\n  misku-native-views [--config apps.toml] <id>\n  misku-native-views --list\n  misku-native-views add [opciones] <url> [url...]\n\nOpciones:\n  -a, --app <id>       Perfil a abrir\n  -c, --config <ruta>  Archivo apps.toml alternativo\n  -l, --list           Lista perfiles\n  -h, --help           Muestra esta ayuda\n\nAdd:\n  add <url>            Crea o actualiza perfiles por URL\n  add --help           Muestra opciones de add"
    );
}

fn print_add_help() {
    println!(
        "Misku Native Views - add\n\nUso:\n  misku-native-views add <url> [url...]\n  misku-native-views add --id <id> --name <nombre> --icon <ruta> <url>\n\nOpciones:\n  -i, --id <id>        Id del perfil. Si ya existe, se actualiza.\n  -n, --name <nombre>  Nombre visible de la app.\n  --icon <ruta>        Icono .ico o .png relativo a apps.toml.\n  -c, --config <ruta>  Archivo apps.toml alternativo.\n  -h, --help           Muestra esta ayuda.\n\nEjemplos:\n  misku-native-views add https://chatgpt.com\n  misku-native-views add --id tftacademy --name \"TFT Academy\" https://tftacademy.com/tierlist/comps/"
    );
}

fn default_width() -> f64 {
    1200.0
}

fn default_height() -> f64 {
    800.0
}

fn default_min_width() -> f64 {
    640.0
}

fn default_min_height() -> f64 {
    480.0
}

fn default_true() -> bool {
    true
}
