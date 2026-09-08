use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct LaunchArgs {
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) json: bool,
    pub(crate) command: Command,
}

#[derive(Clone, Debug)]
pub(crate) enum Command {
    Manager,
    Open { selector: Option<String> },
    Inspect { selector: Option<String> },
    List,
    Create(CreateArgs),
    Update(UpdateArgs),
    Remove(RemoveArgs),
    Export(ExportArgs),
    Help(HelpTopic),
    Version,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum HelpTopic {
    Main,
    Create,
    Update,
    Remove,
    Export,
}

#[derive(Clone, Debug)]
pub(crate) struct CreateArgs {
    pub(crate) urls: Vec<String>,
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) icon: Option<String>,
    pub(crate) allowed_origins: Vec<String>,
    pub(crate) allow_insecure_http: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct UpdateArgs {
    pub(crate) selector: String,
    pub(crate) url: Option<String>,
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) icon: Option<String>,
    pub(crate) clear_icon: bool,
    pub(crate) allowed_origins: Option<Vec<String>>,
    pub(crate) allow_insecure_http: Option<bool>,
    pub(crate) suspend_when_minimized: Option<bool>,
}

#[derive(Clone, Debug)]
pub(crate) struct RemoveArgs {
    pub(crate) selector: String,
    pub(crate) purge_data: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ExportArgs {
    pub(crate) selector: String,
    pub(crate) output: PathBuf,
}

pub(crate) fn parse_args<I>(args: I) -> Result<LaunchArgs, String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    let mut config_path = None;
    let mut json = false;
    let mut selector = None;
    let mut list = false;
    let mut manager = false;
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--config" | "-c" => {
                config_path = Some(PathBuf::from(required_value(&args, &mut index, arg)?));
            }
            "--json" => json = true,
            "--list" | "-l" | "list" => list = true,
            "--manager" | "manage" => manager = true,
            "--app" | "-a" => {
                set_selector(
                    &mut selector,
                    required_value(&args, &mut index, arg)?.to_string(),
                )?;
            }
            "run" | "open" | "inspect" => {
                let mut command = parse_run(&args[index + 1..], &mut config_path, &mut json)?;
                if arg == "inspect"
                    && let Command::Open { selector } = command
                {
                    command = Command::Inspect { selector };
                }
                return Ok(LaunchArgs {
                    config_path,
                    json,
                    command,
                });
            }
            "create" | "add" | "upsert" => {
                let command = parse_create(&args[index + 1..], &mut config_path, &mut json)?;
                return Ok(LaunchArgs {
                    config_path,
                    json,
                    command,
                });
            }
            "update" => {
                let command = parse_update(&args[index + 1..], &mut config_path, &mut json)?;
                return Ok(LaunchArgs {
                    config_path,
                    json,
                    command,
                });
            }
            "remove" | "delete" => {
                let command = parse_remove(&args[index + 1..], &mut config_path, &mut json)?;
                return Ok(LaunchArgs {
                    config_path,
                    json,
                    command,
                });
            }
            "export" => {
                let command = parse_export(&args[index + 1..], &mut config_path, &mut json)?;
                return Ok(LaunchArgs {
                    config_path,
                    json,
                    command,
                });
            }
            "--help" | "-h" | "help" => {
                return Ok(LaunchArgs {
                    config_path,
                    json,
                    command: Command::Help(HelpTopic::Main),
                });
            }
            "--version" | "-V" => {
                return Ok(LaunchArgs {
                    config_path,
                    json,
                    command: Command::Version,
                });
            }
            value if value.starts_with("--config=") => {
                config_path = Some(PathBuf::from(&value["--config=".len()..]));
            }
            value if value.starts_with('-') => {
                return Err(format!("argumento desconocido: {value}"));
            }
            value => set_selector(&mut selector, value.to_string())?,
        }
        index += 1;
    }

    if manager && (list || selector.is_some() || json) {
        return Err("manage no se puede combinar con una app, --list o --json".into());
    }
    Ok(LaunchArgs {
        config_path,
        json,
        command: if list {
            Command::List
        } else if manager || selector.is_none() {
            Command::Manager
        } else {
            Command::Open { selector }
        },
    })
}

fn parse_run(
    args: &[String],
    config_path: &mut Option<PathBuf>,
    json: &mut bool,
) -> Result<Command, String> {
    let mut selector = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--config" | "-c" => {
                *config_path = Some(PathBuf::from(required_value(args, &mut index, arg)?));
            }
            "--json" => *json = true,
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Main)),
            value if value.starts_with("--config=") => {
                *config_path = Some(PathBuf::from(&value["--config=".len()..]));
            }
            value if value.starts_with('-') => {
                return Err(format!("argumento desconocido para run: {value}"));
            }
            value => set_selector(&mut selector, value.to_string())?,
        }
        index += 1;
    }
    Ok(Command::Open { selector })
}

fn parse_create(
    args: &[String],
    config_path: &mut Option<PathBuf>,
    json: &mut bool,
) -> Result<Command, String> {
    let mut urls = Vec::new();
    let mut id = None;
    let mut name = None;
    let mut icon = None;
    let mut allowed_origins = Vec::new();
    let mut allow_insecure_http = false;
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--id" | "-i" => {
                id = Some(required_value(args, &mut index, arg)?.to_string());
            }
            "--name" | "-n" => {
                name = Some(required_value(args, &mut index, arg)?.to_string());
            }
            "--icon" => {
                icon = Some(required_value(args, &mut index, arg)?.to_string());
            }
            "--allow-origin" => {
                allowed_origins.push(required_value(args, &mut index, arg)?.to_string());
            }
            "--allow-http" => allow_insecure_http = true,
            "--config" | "-c" => {
                *config_path = Some(PathBuf::from(required_value(args, &mut index, arg)?));
            }
            "--json" => *json = true,
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Create)),
            value if value.starts_with("--id=") => {
                id = Some(value["--id=".len()..].to_string());
            }
            value if value.starts_with("--name=") => {
                name = Some(value["--name=".len()..].to_string());
            }
            value if value.starts_with("--icon=") => {
                icon = Some(value["--icon=".len()..].to_string());
            }
            value if value.starts_with("--allow-origin=") => {
                allowed_origins.push(value["--allow-origin=".len()..].to_string());
            }
            value if value.starts_with("--config=") => {
                *config_path = Some(PathBuf::from(&value["--config=".len()..]));
            }
            value if value.starts_with('-') => {
                return Err(format!("argumento desconocido para create: {value}"));
            }
            value => urls.push(value.to_string()),
        }
        index += 1;
    }

    if urls.is_empty() {
        return Err("create necesita al menos una URL".to_string());
    }
    Ok(Command::Create(CreateArgs {
        urls,
        id,
        name,
        icon,
        allowed_origins,
        allow_insecure_http,
    }))
}

fn parse_update(
    args: &[String],
    config_path: &mut Option<PathBuf>,
    json: &mut bool,
) -> Result<Command, String> {
    let mut selector = None;
    let mut url = None;
    let mut id = None;
    let mut name = None;
    let mut icon = None;
    let mut clear_icon = false;
    let mut allowed_origins: Option<Vec<String>> = None;
    let mut allow_insecure_http = None;
    let mut suspend_when_minimized = None;
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--url" => url = Some(required_value(args, &mut index, arg)?.to_string()),
            "--id" | "-i" => id = Some(required_value(args, &mut index, arg)?.to_string()),
            "--name" | "-n" => {
                name = Some(required_value(args, &mut index, arg)?.to_string());
            }
            "--icon" => {
                icon = Some(required_value(args, &mut index, arg)?.to_string());
                clear_icon = false;
            }
            "--clear-icon" => {
                icon = None;
                clear_icon = true;
            }
            "--allow-origin" => {
                allowed_origins
                    .get_or_insert_with(Vec::new)
                    .push(required_value(args, &mut index, arg)?.to_string());
            }
            "--clear-allowed-origins" => allowed_origins = Some(Vec::new()),
            "--allow-http" => allow_insecure_http = Some(true),
            "--deny-http" => allow_insecure_http = Some(false),
            "--suspend-on-minimize" => suspend_when_minimized = Some(true),
            "--keep-active" => suspend_when_minimized = Some(false),
            "--config" | "-c" => {
                *config_path = Some(PathBuf::from(required_value(args, &mut index, arg)?));
            }
            "--json" => *json = true,
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Update)),
            value if value.starts_with("--url=") => {
                url = Some(value["--url=".len()..].to_string());
            }
            value if value.starts_with("--id=") => {
                id = Some(value["--id=".len()..].to_string());
            }
            value if value.starts_with("--name=") => {
                name = Some(value["--name=".len()..].to_string());
            }
            value if value.starts_with("--icon=") => {
                icon = Some(value["--icon=".len()..].to_string());
                clear_icon = false;
            }
            value if value.starts_with("--allow-origin=") => {
                allowed_origins
                    .get_or_insert_with(Vec::new)
                    .push(value["--allow-origin=".len()..].to_string());
            }
            value if value.starts_with("--config=") => {
                *config_path = Some(PathBuf::from(&value["--config=".len()..]));
            }
            value if value.starts_with('-') => {
                return Err(format!("argumento desconocido para update: {value}"));
            }
            value => set_selector(&mut selector, value.to_string())?,
        }
        index += 1;
    }

    let selector = selector.ok_or_else(|| "update necesita un id o UUID".to_string())?;
    if url.is_none()
        && id.is_none()
        && name.is_none()
        && icon.is_none()
        && !clear_icon
        && allowed_origins.is_none()
        && allow_insecure_http.is_none()
        && suspend_when_minimized.is_none()
    {
        return Err("update necesita al menos un cambio".to_string());
    }
    Ok(Command::Update(UpdateArgs {
        selector,
        url,
        id,
        name,
        icon,
        clear_icon,
        allowed_origins,
        allow_insecure_http,
        suspend_when_minimized,
    }))
}

fn parse_remove(
    args: &[String],
    config_path: &mut Option<PathBuf>,
    json: &mut bool,
) -> Result<Command, String> {
    let mut selector = None;
    let mut purge_data = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--purge-data" => purge_data = true,
            "--config" | "-c" => {
                *config_path = Some(PathBuf::from(required_value(args, &mut index, arg)?));
            }
            "--json" => *json = true,
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Remove)),
            value if value.starts_with("--config=") => {
                *config_path = Some(PathBuf::from(&value["--config=".len()..]));
            }
            value if value.starts_with('-') => {
                return Err(format!("argumento desconocido para remove: {value}"));
            }
            value => set_selector(&mut selector, value.to_string())?,
        }
        index += 1;
    }
    Ok(Command::Remove(RemoveArgs {
        selector: selector.ok_or_else(|| "remove necesita un id o UUID".to_string())?,
        purge_data,
    }))
}

fn parse_export(
    args: &[String],
    config_path: &mut Option<PathBuf>,
    json: &mut bool,
) -> Result<Command, String> {
    let mut selector = None;
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--output" | "-o" => {
                output = Some(PathBuf::from(required_value(args, &mut index, arg)?));
            }
            "--config" | "-c" => {
                *config_path = Some(PathBuf::from(required_value(args, &mut index, arg)?));
            }
            "--json" => *json = true,
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Export)),
            value if value.starts_with("--output=") => {
                output = Some(PathBuf::from(&value["--output=".len()..]));
            }
            value if value.starts_with("--config=") => {
                *config_path = Some(PathBuf::from(&value["--config=".len()..]));
            }
            value if value.starts_with('-') => {
                return Err(format!("argumento desconocido para export: {value}"));
            }
            value => set_selector(&mut selector, value.to_string())?,
        }
        index += 1;
    }
    Ok(Command::Export(ExportArgs {
        selector: selector.ok_or_else(|| "export necesita un id o UUID".to_string())?,
        output: output.ok_or_else(|| "export necesita --output <ruta>".to_string())?,
    }))
}

fn required_value<'a>(
    args: &'a [String],
    index: &mut usize,
    option: &str,
) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("falta el valor despues de {option}"))
}

fn set_selector(selector: &mut Option<String>, value: String) -> Result<(), String> {
    if selector.replace(value.clone()).is_some() {
        return Err(format!(
            "selector duplicado o argumento inesperado: {value}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_misku_compatible_create_command() {
        let parsed = parse_args([
            "--json".to_string(),
            "--config".to_string(),
            "registry.toml".to_string(),
            "create".to_string(),
            "--name".to_string(),
            "GitHub".to_string(),
            "https://github.com/openai".to_string(),
        ])
        .unwrap();

        assert!(parsed.json);
        assert_eq!(parsed.config_path, Some(PathBuf::from("registry.toml")));
        match parsed.command {
            Command::Create(args) => {
                assert_eq!(args.name.as_deref(), Some("GitHub"));
                assert_eq!(args.urls, vec!["https://github.com/openai"]);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn add_alias_still_creates() {
        let parsed = parse_args([
            "add".to_string(),
            "https://example.com/a".to_string(),
            "https://example.com/b".to_string(),
        ])
        .unwrap();
        match parsed.command {
            Command::Create(args) => assert_eq!(args.urls.len(), 2),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_update_without_changing_instance_selector() {
        let parsed = parse_args([
            "update".to_string(),
            "my-app".to_string(),
            "--url".to_string(),
            "https://example.com/New".to_string(),
            "--name=New name".to_string(),
        ])
        .unwrap();
        match parsed.command {
            Command::Update(args) => {
                assert_eq!(args.selector, "my-app");
                assert_eq!(args.url.as_deref(), Some("https://example.com/New"));
                assert_eq!(args.name.as_deref(), Some("New name"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn preserves_bare_selector_and_list_aliases() {
        let open = parse_args(["github".to_string()]).unwrap();
        assert!(matches!(
            open.command,
            Command::Open {
                selector: Some(ref value)
            } if value == "github"
        ));

        let list = parse_args(["--list".to_string(), "--json".to_string()]).unwrap();
        assert!(list.json);
        assert!(matches!(list.command, Command::List));
    }
}
