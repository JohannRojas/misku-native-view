#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod model;
mod runtime;

use cli::{Command, HelpTopic, LaunchArgs};
use model::{
    AppProfile, CreateRequest, UpdateRequest, acquire_config_lock, create_profiles, export_profile,
    load_config, load_config_for_write, purge_profile_data, remove_profile, save_config_atomic,
    select_profile, update_profile,
};
use serde::Serialize;
use std::{
    env,
    path::{Path, PathBuf},
};

const DEFAULT_CONFIG_FILE: &str = "apps.toml";
const CONFIG_ENV: &str = "MISKU_NV_CONFIG";

#[derive(Serialize)]
struct OperationResult {
    action: &'static str,
    profile: AppProfile,
    config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("misku-native-views: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = cli::parse_args(env::args().skip(1))?;
    match args.command.clone() {
        Command::Help(topic) => {
            print_help(topic);
            Ok(())
        }
        Command::Version => {
            println!("misku-nv {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::List => list_apps(&args),
        Command::Create(command) => {
            let config_path = resolve_config_path_for_write(args.config_path.as_deref())?;
            let _lock = acquire_config_lock(&config_path)?;
            let mut config = load_config_for_write(&config_path)?;
            let profiles = create_profiles(
                &mut config,
                &config_path,
                CreateRequest {
                    urls: command.urls,
                    id: command.id,
                    name: command.name,
                    icon: command.icon,
                    allowed_origins: command.allowed_origins,
                    allow_insecure_http: command.allow_insecure_http,
                },
            )?;
            save_config_atomic(&config_path, &config)?;
            let results = profiles
                .into_iter()
                .map(|profile| OperationResult {
                    action: "created",
                    profile,
                    config: display_path(&config_path),
                    output: None,
                })
                .collect::<Vec<_>>();
            print_results(args.json, &results)
        }
        Command::Update(command) => {
            let config_path = resolve_config_path(args.config_path.as_deref())?;
            let _lock = acquire_config_lock(&config_path)?;
            let mut config = load_config(&config_path)?;
            let profile = update_profile(
                &mut config,
                &config_path,
                UpdateRequest {
                    selector: command.selector,
                    url: command.url,
                    id: command.id,
                    name: command.name,
                    icon: command.icon,
                    clear_icon: command.clear_icon,
                    allowed_origins: command.allowed_origins,
                    allow_insecure_http: command.allow_insecure_http,
                },
            )?;
            save_config_atomic(&config_path, &config)?;
            print_results(
                args.json,
                &[OperationResult {
                    action: "updated",
                    profile,
                    config: display_path(&config_path),
                    output: None,
                }],
            )
        }
        Command::Remove(command) => {
            let config_path = resolve_config_path(args.config_path.as_deref())?;
            let _lock = acquire_config_lock(&config_path)?;
            let mut config = load_config(&config_path)?;
            let profile = remove_profile(&mut config, &command.selector)?;
            save_config_atomic(&config_path, &config)?;
            if command.purge_data {
                purge_profile_data(&profile).map_err(|error| {
                    format!(
                        "la app se elimino del registro, pero no se pudieron purgar sus datos: {error}"
                    )
                })?;
            }
            print_results(
                args.json,
                &[OperationResult {
                    action: "removed",
                    profile,
                    config: display_path(&config_path),
                    output: None,
                }],
            )
        }
        Command::Export(command) => {
            let config_path = resolve_config_path(args.config_path.as_deref())?;
            let config = load_config(&config_path)?;
            let profile = select_profile(&config, Some(&command.selector))?;
            let output = absolute_path(&command.output)?;
            let exported = export_profile(&config_path, &profile, &output)?;
            print_results(
                args.json,
                &[OperationResult {
                    action: "exported",
                    profile: exported,
                    config: display_path(&config_path),
                    output: Some(display_path(&output)),
                }],
            )
        }
        Command::Open { selector } => {
            let config_path = resolve_config_path(args.config_path.as_deref())?;
            let config = load_config(&config_path)?;
            let inferred_id = infer_profile_id_from_exe();
            let profile = select_profile(&config, selector.as_deref().or(inferred_id.as_deref()))?;
            runtime::launch(profile, &config_path)
        }
    }
}

fn list_apps(args: &LaunchArgs) -> Result<(), String> {
    let config_path = resolve_config_path(args.config_path.as_deref())?;
    let config = load_config(&config_path)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&config.apps)
                .map_err(|error| format!("no se pudo generar JSON: {error}"))?
        );
    } else if config.apps.is_empty() {
        println!("No hay apps configuradas.");
    } else {
        for profile in &config.apps {
            println!(
                "{}  {}  {}  {}",
                profile.id,
                profile.instance_id(),
                profile.name,
                profile.url
            );
        }
    }
    Ok(())
}

fn print_results(json: bool, results: &[OperationResult]) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(results)
                .map_err(|error| format!("no se pudo generar JSON: {error}"))?
        );
        return Ok(());
    }

    for result in results {
        let action = match result.action {
            "created" => "creado",
            "updated" => "actualizado",
            "removed" => "eliminado",
            "exported" => "exportado",
            value => value,
        };
        println!(
            "{action}: {} -> {} (uuid: {})",
            result.profile.id,
            result.profile.url,
            result.profile.instance_id()
        );
        if let Some(output) = &result.output {
            println!("manifiesto: {output}");
        }
    }
    if let Some(first) = results.first() {
        println!("configuracion: {}", first.config);
    }
    Ok(())
}

fn resolve_config_path(explicit_path: Option<&Path>) -> Result<PathBuf, String> {
    let path = configured_path(explicit_path)?;
    path.canonicalize()
        .map_err(|error| format!("no se pudo leer {}: {error}", path.display()))
}

fn resolve_config_path_for_write(explicit_path: Option<&Path>) -> Result<PathBuf, String> {
    let path = configured_path(explicit_path)?;
    if path.exists() {
        path.canonicalize()
            .map_err(|error| format!("no se pudo leer {}: {error}", path.display()))
    } else {
        absolute_path(&path)
    }
}

fn configured_path(explicit_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit_path {
        return Ok(path.to_path_buf());
    }
    if let Some(path) = env::var_os(CONFIG_ENV) {
        return Ok(PathBuf::from(path));
    }
    env::current_exe()
        .map_err(|error| format!("no se pudo resolver el ejecutable actual: {error}"))?
        .parent()
        .map(|parent| parent.join(DEFAULT_CONFIG_FILE))
        .ok_or_else(|| {
            format!("no se pudo resolver {DEFAULT_CONFIG_FILE}; usa --config <ruta> o {CONFIG_ENV}")
        })
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| format!("no se pudo resolver el directorio actual: {error}"))
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

fn display_path(path: &Path) -> String {
    let value = path.display().to_string();
    value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
}

fn print_help(topic: HelpTopic) {
    match topic {
        HelpTopic::Main => println!(
            "Misku Native Views\n\nUso:\n  misku-nv <url> [opciones]\n  misku-nv <id|uuid>\n  misku-nv create <url> [url...]\n  misku-nv update <id|uuid> [opciones]\n  misku-nv remove <id|uuid> [--purge-data]\n  misku-nv --list [--json]\n\nComandos:\n  create, add     Siempre crea una app independiente\n  update          Modifica explicitamente una app\n  remove          Elimina una app del registro\n  export          Genera un manifiesto independiente\n  run, open       Abre una app\n\nOpciones globales:\n  -c, --config <ruta>  Registro apps.toml explicito\n  --json               Salida estructurada\n  -h, --help           Muestra esta ayuda\n  -V, --version        Muestra la version"
        ),
        HelpTopic::Create => println!(
            "Uso:\n  misku-nv create <url> [url...]\n\nOpciones:\n  -i, --id <id>             Alias visible (solo una URL)\n  -n, --name <nombre>       Nombre visible (solo una URL)\n  --icon <ruta>             Importa un .ico o .png\n  --allow-origin <origen>   Permite navegacion interna adicional\n  --allow-http              Solo habilita HTTP para loopback\n\nCada URL crea un UUID, perfil y manifiesto independientes."
        ),
        HelpTopic::Update => println!(
            "Uso:\n  misku-nv update <id|uuid> [opciones]\n\nOpciones:\n  --url <url>\n  -i, --id <nuevo-id>\n  -n, --name <nombre>\n  --icon <ruta> | --clear-icon\n  --allow-origin <origen> | --clear-allowed-origins\n  --allow-http | --deny-http"
        ),
        HelpTopic::Remove => println!(
            "Uso:\n  misku-nv remove <id|uuid> [--purge-data]\n\nSin --purge-data se conservan cookies y sesiones."
        ),
        HelpTopic::Export => println!("Uso:\n  misku-nv export <id|uuid> --output <ruta-app.toml>"),
    }
}
