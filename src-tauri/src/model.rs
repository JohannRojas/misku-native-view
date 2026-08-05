use image::{
    ExtendedColorType, ImageFormat, ImageReader, Limits, RgbaImage,
    codecs::ico::{IcoEncoder, IcoFrame},
    imageops::{self, FilterType},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env, fs,
    fs::{File, OpenOptions},
    io::{Cursor, Write},
    path::{Component, Path, PathBuf},
};
use url::{Host, Origin, Url};
use uuid::Uuid;

pub(crate) const CURRENT_SCHEMA_VERSION: u32 = 2;
const MAX_ID_LENGTH: usize = 64;
const MAX_ICON_BYTES: u64 = 10 * 1024 * 1024;
const MAX_ICON_DIMENSION: u32 = 2048;
const MAX_ICON_ALLOC_BYTES: u64 = 32 * 1024 * 1024;
const WINDOWS_ICON_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
const LEGACY_NAMESPACE: Uuid = Uuid::from_u128(0x6f6d_e1d5_79d4_4d5f_b38a_6495_a75a_7c40);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct AppsConfig {
    #[serde(default = "legacy_schema_version")]
    pub(crate) schema_version: u32,
    #[serde(default)]
    pub(crate) apps: Vec<AppProfile>,
}

impl Default for AppsConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            apps: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct AppProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) instance_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) profile_key: Option<String>,
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) url: String,
    #[serde(default = "default_width")]
    pub(crate) width: f64,
    #[serde(default = "default_height")]
    pub(crate) height: f64,
    #[serde(default = "default_min_width")]
    pub(crate) min_width: f64,
    #[serde(default = "default_min_height")]
    pub(crate) min_height: f64,
    #[serde(default = "default_true")]
    pub(crate) isolated_profile: bool,
    #[serde(default)]
    pub(crate) devtools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) user_agent: Option<String>,
    #[serde(default = "default_true")]
    pub(crate) resizable: bool,
    #[serde(default = "default_true")]
    pub(crate) zoom_hotkeys_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) allowed_origins: Vec<String>,
    #[serde(default)]
    pub(crate) allow_insecure_http: bool,
}

impl AppProfile {
    fn ensure_identity(&mut self) {
        if self.instance_id.is_none() {
            let seed = format!("legacy\0{}\0{}", self.id, self.url);
            self.instance_id = Some(Uuid::new_v5(&LEGACY_NAMESPACE, seed.as_bytes()));
        }
        if self.profile_key.is_none() {
            self.profile_key = Some(self.id.clone());
        }
    }

    pub(crate) fn instance_id(&self) -> Uuid {
        self.instance_id
            .expect("loaded and created profiles always have an instance id")
    }

    pub(crate) fn profile_key(&self) -> &str {
        self.profile_key
            .as_deref()
            .expect("loaded and created profiles always have a profile key")
    }

    pub(crate) fn uses_legacy_data_root(&self) -> bool {
        self.profile_key() != self.instance_id().to_string()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CreateRequest {
    pub(crate) urls: Vec<String>,
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) icon: Option<String>,
    pub(crate) allowed_origins: Vec<String>,
    pub(crate) allow_insecure_http: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct UpdateRequest {
    pub(crate) selector: String,
    pub(crate) url: Option<String>,
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) icon: Option<String>,
    pub(crate) clear_icon: bool,
    pub(crate) allowed_origins: Option<Vec<String>>,
    pub(crate) allow_insecure_http: Option<bool>,
}

pub(crate) struct ConfigLock {
    _file: File,
    #[cfg(not(windows))]
    path: PathBuf,
}

#[cfg(not(windows))]
impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn acquire_config_lock(config_path: &Path) -> Result<ConfigLock, String> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("no se pudo crear {}: {error}", parent.display()))?;
    }

    let file_name = config_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("apps.toml");
    let lock_path = config_path.with_file_name(format!("{file_name}.lock"));

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&lock_path)
            .map_err(|error| {
                format!(
                    "otra operacion esta modificando {} ({error})",
                    config_path.display()
                )
            })?;
        Ok(ConfigLock { _file: file })
    }

    #[cfg(not(windows))]
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|error| {
                format!(
                    "otra operacion esta modificando {} ({error})",
                    config_path.display()
                )
            })?;
        writeln!(file, "{}", std::process::id())
            .map_err(|error| format!("no se pudo escribir {}: {error}", lock_path.display()))?;
        Ok(ConfigLock {
            _file: file,
            path: lock_path,
        })
    }
}

pub(crate) fn load_config(path: &Path) -> Result<AppsConfig, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("no se pudo leer {}: {error}", path.display()))?;
    parse_config(&content, path)
}

pub(crate) fn load_config_for_write(path: &Path) -> Result<AppsConfig, String> {
    if path.exists() {
        load_config(path)
    } else {
        Ok(AppsConfig::default())
    }
}

fn parse_config(content: &str, path: &Path) -> Result<AppsConfig, String> {
    let mut config: AppsConfig = toml::from_str(content)
        .map_err(|error| format!("configuracion invalida en {}: {error}", path.display()))?;

    if config.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "{} usa schema_version {}, pero esta version solo soporta hasta {}",
            path.display(),
            config.schema_version,
            CURRENT_SCHEMA_VERSION
        ));
    }

    let source_schema_version = config.schema_version;
    config.schema_version = CURRENT_SCHEMA_VERSION;
    for profile in &mut config.apps {
        profile.ensure_identity();
        if source_schema_version < CURRENT_SCHEMA_VERSION {
            profile.isolated_profile = true;
        }
        if source_schema_version < CURRENT_SCHEMA_VERSION
            && !profile.allow_insecure_http
            && let Ok(url) = Url::parse(&profile.url)
            && url.scheme() == "http"
            && is_loopback(&url)
        {
            profile.allow_insecure_http = true;
        }
    }
    validate_config(&config, Some(path))?;
    Ok(config)
}

pub(crate) fn save_config_atomic(path: &Path, config: &AppsConfig) -> Result<(), String> {
    validate_config(config, Some(path))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("no se pudo crear {}: {error}", parent.display()))?;
    }

    let mut serializable = config.clone();
    serializable.schema_version = CURRENT_SCHEMA_VERSION;
    let content = toml::to_string_pretty(&serializable)
        .map_err(|error| format!("no se pudo serializar apps.toml: {error}"))?;

    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("apps.toml");
    let temp_path = path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| format!("no se pudo crear {}: {error}", temp_path.display()))?;
        file.write_all(content.as_bytes())
            .map_err(|error| format!("no se pudo escribir {}: {error}", temp_path.display()))?;
        file.sync_all()
            .map_err(|error| format!("no se pudo sincronizar {}: {error}", temp_path.display()))?;
        drop(file);
        replace_file(&temp_path, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };

    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();

    unsafe {
        MoveFileExW(
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(destination_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| {
        format!(
            "no se pudo reemplazar {} de forma atomica: {error}",
            destination.display()
        )
    })
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| {
        format!(
            "no se pudo reemplazar {} de forma atomica: {error}",
            destination.display()
        )
    })
}

pub(crate) fn validate_config(
    config: &AppsConfig,

    config_path: Option<&Path>,
) -> Result<(), String> {
    if config.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "schema_version {} no soportado",
            config.schema_version
        ));
    }

    let mut ids = HashSet::new();
    let mut instance_ids = HashSet::new();
    let mut profile_keys = HashSet::new();

    for profile in &config.apps {
        validate_profile(profile, config_path)?;
        let id_key = profile.id.to_ascii_lowercase();
        if !ids.insert(id_key) {
            return Err(format!("id duplicado en apps.toml: {}", profile.id));
        }
        if !instance_ids.insert(profile.instance_id()) {
            return Err(format!(
                "instance_id duplicado en apps.toml: {}",
                profile.instance_id()
            ));
        }
        let profile_key = profile.profile_key().to_ascii_lowercase();
        if !profile_keys.insert(profile_key) {
            return Err(format!(
                "profile_key duplicado en apps.toml: {}",
                profile.profile_key()
            ));
        }
    }
    Ok(())
}

fn validate_profile(profile: &AppProfile, config_path: Option<&Path>) -> Result<(), String> {
    validate_id(&profile.id)?;
    validate_path_component(profile.profile_key())?;
    if profile.name.trim().is_empty() {
        return Err(format!("el perfil '{}' tiene nombre vacio", profile.id));
    }
    if !profile.isolated_profile {
        return Err(format!(
            "el perfil '{}' debe usar isolated_profile = true",
            profile.id
        ));
    }
    validate_dimensions(profile)?;

    let url = Url::parse(&profile.url)
        .map_err(|error| format!("url invalida en {}: {error}", profile.id))?;
    validate_parsed_url(&url, profile.allow_insecure_http)?;
    normalize_allowed_origins(&profile.allowed_origins, profile.allow_insecure_http)?;

    if let Some(icon) = profile.icon.as_deref() {
        validate_relative_path(icon)?;
        if let Some(config_path) = config_path.filter(|path| path.exists()) {
            resolve_icon_path(config_path, icon)?;
        }
    }
    Ok(())
}

fn validate_dimensions(profile: &AppProfile) -> Result<(), String> {
    for (name, value) in [
        ("width", profile.width),
        ("height", profile.height),
        ("min_width", profile.min_width),
        ("min_height", profile.min_height),
    ] {
        if !value.is_finite() || !(1.0..=16_384.0).contains(&value) {
            return Err(format!(
                "{name} invalido en '{}': debe estar entre 1 y 16384",
                profile.id
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > MAX_ID_LENGTH {
        return Err(format!(
            "id invalido '{id}': usa entre 1 y {MAX_ID_LENGTH} caracteres"
        ));
    }
    if id != id.to_ascii_lowercase()
        || !id.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        || !id
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        || !id
            .chars()
            .last()
            .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        return Err(format!(
            "id invalido '{id}': usa solo minusculas ASCII, numeros y guiones internos"
        ));
    }
    if is_reserved_windows_name(id) || id == "misku-native-views" {
        return Err(format!("id reservado o no permitido: '{id}'"));
    }
    if Uuid::parse_str(id).is_ok() {
        return Err("el id visible no puede tener forma de UUID".to_string());
    }
    Ok(())
}

fn validate_path_component(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    let mut components = path.components();
    let is_single_normal =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if !is_single_normal
        || value == "."
        || value == ".."
        || value.ends_with(['.', ' '])
        || value.contains(':')
        || is_reserved_windows_name(value)
    {
        return Err(format!("componente de ruta no permitido: '{value}'"));
    }
    Ok(())
}

fn is_reserved_windows_name(value: &str) -> bool {
    let upper = value.trim_end_matches(['.', ' ']).to_ascii_uppercase();
    let device_name = upper
        .split('.')
        .next()
        .unwrap_or(upper.as_str())
        .trim_end_matches(' ');
    matches!(device_name, "CON" | "PRN" | "AUX" | "NUL")
        || (device_name.len() == 4
            && (device_name.starts_with("COM") || device_name.starts_with("LPT"))
            && device_name
                .chars()
                .last()
                .is_some_and(|character| ('1'..='9').contains(&character)))
}
pub(crate) fn parse_app_url(raw: &str, allow_insecure_http: bool) -> Result<Url, String> {
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&candidate).map_err(|error| format!("url invalida '{raw}': {error}"))?;
    validate_parsed_url(&url, allow_insecure_http)
        .map_err(|error| format!("url invalida '{raw}': {error}"))?;
    Ok(url)
}

fn validate_parsed_url(url: &Url, allow_insecure_http: bool) -> Result<(), String> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err("no se permiten credenciales embebidas en la URL".to_string());
    }
    if url.host().is_none() {
        return Err("la URL debe incluir un host".to_string());
    }

    match url.scheme() {
        "https" => Ok(()),
        "http" if allow_insecure_http && is_loopback(url) => Ok(()),
        "http" => {
            Err("HTTP solo se permite para localhost/loopback junto con --allow-http".to_string())
        }
        scheme => Err(format!("esquema no soportado '{scheme}'; usa HTTPS")),
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

fn normalize_origin(raw: &str, allow_insecure_http: bool) -> Result<String, String> {
    let url =
        Url::parse(raw).map_err(|error| format!("origen permitido invalido '{raw}': {error}"))?;
    validate_parsed_url(&url, allow_insecure_http)
        .map_err(|error| format!("origen permitido invalido '{raw}': {error}"))?;
    if url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
        return Err(format!(
            "origen permitido invalido '{raw}': no incluyas path, query ni fragmento"
        ));
    }
    match url.origin() {
        Origin::Tuple(..) => Ok(url.origin().ascii_serialization()),
        Origin::Opaque(_) => Err(format!("origen opaco no permitido: '{raw}'")),
    }
}

fn normalize_allowed_origins(
    origins: &[String],
    allow_insecure_http: bool,
) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for origin in origins {
        let normalized = normalize_origin(origin, allow_insecure_http)?;
        if seen.insert(normalized.clone()) {
            result.push(normalized);
        }
    }
    Ok(result)
}

pub(crate) fn allowed_origin_set(profile: &AppProfile) -> Result<HashSet<String>, String> {
    let url = Url::parse(&profile.url)
        .map_err(|error| format!("url invalida en {}: {error}", profile.id))?;
    let mut origins: HashSet<String> =
        normalize_allowed_origins(&profile.allowed_origins, profile.allow_insecure_http)?
            .into_iter()
            .collect();
    origins.insert(url.origin().ascii_serialization());
    Ok(origins)
}

pub(crate) fn normalize_id(raw: &str) -> Result<String, String> {
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

    let mut normalized = id.trim_matches('-').to_string();
    if normalized.len() > MAX_ID_LENGTH {
        normalized.truncate(MAX_ID_LENGTH);
        normalized = normalized.trim_end_matches('-').to_string();
    }
    validate_id(&normalized)
        .map_err(|_| format!("no se pudo generar un id valido desde '{raw}'"))?;
    Ok(normalized)
}

fn infer_id_from_url(url: &Url) -> Result<String, String> {
    let host = url
        .host_str()
        .ok_or_else(|| format!("url sin host: {url}"))?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').filter(|part| !part.is_empty()).collect();
    let mut base = if labels.len() <= 2 {
        labels.first().copied().unwrap_or(&host).to_string()
    } else {
        labels[..labels.len() - 1].join("-")
    };

    if let Some(segment) = url
        .path_segments()
        .and_then(|mut segments| segments.find(|part| !part.is_empty()))
    {
        let segment = normalize_id(segment).unwrap_or_default();
        if !segment.is_empty() && segment != "home" {
            base.push('-');
            base.push_str(&segment);
        }
    }
    normalize_id(&base)
}

fn allocate_unique_id(config: &AppsConfig, base: &str) -> String {
    let exists = |candidate: &str| {
        config
            .apps
            .iter()
            .any(|profile| profile.id.eq_ignore_ascii_case(candidate))
    };
    if !exists(base) {
        return base.to_string();
    }

    for index in 2_u32.. {
        let suffix = format!("-{index}");
        let max_base = MAX_ID_LENGTH.saturating_sub(suffix.len());
        let mut prefix = base.chars().take(max_base).collect::<String>();
        prefix = prefix.trim_end_matches('-').to_string();
        let candidate = format!("{prefix}{suffix}");
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!("the numeric suffix space is effectively unbounded")
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

pub(crate) fn create_profiles(
    config: &mut AppsConfig,
    config_path: &Path,
    request: CreateRequest,
) -> Result<Vec<AppProfile>, String> {
    if request.urls.is_empty() {
        return Err("create necesita al menos una URL".to_string());
    }
    if request.urls.len() > 1
        && (request.id.is_some() || request.name.is_some() || request.icon.is_some())
    {
        return Err("--id, --name y --icon solo se pueden usar al crear una sola URL".to_string());
    }

    let mut candidate = config.clone();
    candidate.schema_version = CURRENT_SCHEMA_VERSION;
    let mut created = Vec::new();

    for raw_url in &request.urls {
        let url = parse_app_url(raw_url, request.allow_insecure_http)?;
        let base_id = match request.id.as_deref() {
            Some(value) => normalize_id(value)?,
            None => infer_id_from_url(&url)?,
        };
        let id = if request.id.is_some() {
            if candidate
                .apps
                .iter()
                .any(|profile| profile.id.eq_ignore_ascii_case(&base_id))
            {
                return Err(format!(
                    "ya existe la app '{base_id}'; usa update para modificarla"
                ));
            }
            base_id
        } else {
            allocate_unique_id(&candidate, &base_id)
        };

        let instance_id = Uuid::new_v4();
        let icon = if let Some(icon) = request.icon.as_deref() {
            Some(import_icon(config_path, icon, instance_id)?)
        } else if let Some(inferred) = infer_icon_reference(config_path, &id) {
            let inferred_path = config_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(inferred);
            let inferred_path = inferred_path.to_str().ok_or_else(|| {
                format!(
                    "la ruta del icono inferido no es Unicode: {}",
                    inferred_path.display()
                )
            })?;
            Some(import_icon(config_path, inferred_path, instance_id)?)
        } else {
            None
        };

        let profile = AppProfile {
            instance_id: Some(instance_id),
            profile_key: Some(instance_id.to_string()),
            id: id.clone(),
            name: request.name.clone().unwrap_or_else(|| name_from_id(&id)),
            url: url.to_string(),
            width: default_width(),
            height: default_height(),
            min_width: default_min_width(),
            min_height: default_min_height(),
            isolated_profile: true,
            devtools: false,
            user_agent: None,
            resizable: true,
            zoom_hotkeys_enabled: true,
            icon,
            allowed_origins: normalize_allowed_origins(
                &request.allowed_origins,
                request.allow_insecure_http,
            )?,
            allow_insecure_http: request.allow_insecure_http,
        };
        validate_profile(&profile, Some(config_path))?;
        candidate.apps.push(profile.clone());
        created.push(profile);
    }

    validate_config(&candidate, Some(config_path))?;
    *config = candidate;
    Ok(created)
}

pub(crate) fn update_profile(
    config: &mut AppsConfig,
    config_path: &Path,
    request: UpdateRequest,
) -> Result<AppProfile, String> {
    let index = find_profile_index(config, &request.selector)?;
    let mut updated = config.apps[index].clone();

    if let Some(id) = request.id.as_deref() {
        let id = normalize_id(id)?;
        if config
            .apps
            .iter()
            .enumerate()
            .any(|(other_index, profile)| {
                other_index != index && profile.id.eq_ignore_ascii_case(&id)
            })
        {
            return Err(format!("ya existe la app '{id}'"));
        }
        updated.id = id;
    }
    if let Some(name) = request.name {
        if name.trim().is_empty() {
            return Err("--name no puede estar vacio".to_string());
        }
        updated.name = name;
    }
    if let Some(allow_insecure_http) = request.allow_insecure_http {
        updated.allow_insecure_http = allow_insecure_http;
    }
    if let Some(url) = request.url.as_deref() {
        updated.url = parse_app_url(url, updated.allow_insecure_http)?.to_string();
    }
    if let Some(origins) = request.allowed_origins {
        updated.allowed_origins = normalize_allowed_origins(&origins, updated.allow_insecure_http)?;
    }
    if request.clear_icon {
        updated.icon = None;
    } else if let Some(icon) = request.icon.as_deref() {
        updated.icon = Some(import_icon(config_path, icon, updated.instance_id())?);
    }

    let mut candidate = config.clone();
    candidate.schema_version = CURRENT_SCHEMA_VERSION;
    candidate.apps[index] = updated.clone();
    validate_config(&candidate, Some(config_path))?;
    *config = candidate;
    Ok(updated)
}

pub(crate) fn remove_profile(
    config: &mut AppsConfig,
    selector: &str,
) -> Result<AppProfile, String> {
    let index = find_profile_index(config, selector)?;
    let mut candidate = config.clone();
    candidate.schema_version = CURRENT_SCHEMA_VERSION;
    let removed = candidate.apps.remove(index);
    validate_config(&candidate, None)?;
    *config = candidate;
    Ok(removed)
}

fn find_profile_index(config: &AppsConfig, selector: &str) -> Result<usize, String> {
    if let Ok(instance_id) = Uuid::parse_str(selector)
        && let Some(index) = config
            .apps
            .iter()
            .position(|profile| profile.instance_id() == instance_id)
    {
        return Ok(index);
    }
    config
        .apps
        .iter()
        .position(|profile| profile.id.eq_ignore_ascii_case(selector))
        .ok_or_else(|| format!("no existe la app '{selector}'. Usa --list para ver opciones"))
}

pub(crate) fn select_profile(
    config: &AppsConfig,
    selector: Option<&str>,
) -> Result<AppProfile, String> {
    match selector {
        Some(selector) => {
            let index = find_profile_index(config, selector)?;
            Ok(config.apps[index].clone())
        }
        None => config
            .apps
            .first()
            .cloned()
            .ok_or_else(|| "no hay apps configuradas".to_string()),
    }
}

pub(crate) fn export_profile(
    source_config: &Path,
    profile: &AppProfile,
    destination_config: &Path,
) -> Result<AppProfile, String> {
    if paths_refer_to_same_location(source_config, destination_config)? {
        return Err("el manifiesto exportado no puede reemplazar el registro central".to_string());
    }
    if let Some(parent) = destination_config.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("no se pudo crear {}: {error}", parent.display()))?;
    }

    let mut exported = profile.clone();
    if let Some(icon) = profile.icon.as_deref() {
        let source = resolve_icon_path(source_config, icon)?;
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("icono sin extension: {}", source.display()))?
            .to_ascii_lowercase();
        let relative = format!("icons/{}.{}", profile.instance_id(), extension);
        let destination_base = destination_config
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let icons_directory = ensure_managed_subdirectory(destination_base, "icons")?;
        let destination = icons_directory.join(format!("{}.{}", profile.instance_id(), extension));
        copy_file_atomic(&source, &destination)?;
        exported.icon = Some(relative);
    }

    let output = AppsConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        apps: vec![exported.clone()],
    };
    save_config_atomic(destination_config, &output)?;
    Ok(exported)
}

fn infer_icon_reference(config_path: &Path, id: &str) -> Option<String> {
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    for extension in ["ico", "png"] {
        let relative = format!("icons/{id}.{extension}");
        if base.join(&relative).is_file() {
            return Some(relative);
        }
    }
    None
}

fn import_icon(config_path: &Path, raw: &str, instance_id: Uuid) -> Result<String, String> {
    let config_base = config_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(config_base)
        .map_err(|error| format!("no se pudo crear {}: {error}", config_base.display()))?;
    let source_path = Path::new(raw);
    let source = if source_path.is_absolute() {
        source_path.to_path_buf()
    } else {
        config_base.join(source_path)
    };
    let source = source
        .canonicalize()
        .map_err(|error| format!("no se pudo leer el icono {}: {error}", source.display()))?;
    let metadata = fs::metadata(&source)
        .map_err(|error| format!("no se pudo leer el icono {}: {error}", source.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_ICON_BYTES {
        return Err(format!(
            "icono invalido {}: debe ser archivo y medir maximo {} MiB",
            source.display(),
            MAX_ICON_BYTES / 1024 / 1024
        ));
    }
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| format!("icono sin extension: {}", source.display()))?;
    if !matches!(extension.as_str(), "ico" | "png") {
        return Err("el icono debe ser .ico o .png".to_string());
    }

    let bytes = fs::read(&source)
        .map_err(|error| format!("no se pudo leer el icono {}: {error}", source.display()))?;
    if bytes.len() as u64 > MAX_ICON_BYTES {
        return Err(format!(
            "icono invalido {}: debe medir maximo {} MiB",
            source.display(),
            MAX_ICON_BYTES / 1024 / 1024
        ));
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("no se pudo detectar el icono {}: {error}", source.display()))?;
    let format = reader
        .format()
        .ok_or_else(|| format!("formato de icono desconocido: {}", source.display()))?;
    if !matches!(format, ImageFormat::Ico | ImageFormat::Png) {
        return Err("el contenido del icono debe ser ICO o PNG".to_string());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_ICON_DIMENSION);
    limits.max_image_height = Some(MAX_ICON_DIMENSION);
    limits.max_alloc = Some(MAX_ICON_ALLOC_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("icono invalido {}: {error}", source.display()))?;
    let source_image = decoded.to_rgba8();
    let source_width = source_image.width();
    let source_height = source_image.height();
    if source_width == 0 || source_height == 0 {
        return Err("el icono no puede tener dimensiones vacias".to_string());
    }

    let asset_id = Uuid::new_v4();
    let relative = format!("icons/{instance_id}-{asset_id}.ico");
    let icons_directory = ensure_managed_subdirectory(config_base, "icons")?;
    let destination = icons_directory.join(format!("{instance_id}-{asset_id}.ico"));
    ensure_regular_destination_if_exists(&destination)?;
    let file_name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("icon.ico");
    let temporary = destination.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));

    let result = (|| {
        let master_size = *WINDOWS_ICON_SIZES
            .last()
            .expect("WINDOWS_ICON_SIZES no puede estar vacio");
        let (master_width, master_height) = if source_width >= source_height {
            (
                master_size,
                ((u64::from(source_height) * u64::from(master_size)) / u64::from(source_width))
                    .max(1) as u32,
            )
        } else {
            (
                ((u64::from(source_width) * u64::from(master_size)) / u64::from(source_height))
                    .max(1) as u32,
                master_size,
            )
        };
        let resized = imageops::resize(
            &source_image,
            master_width,
            master_height,
            FilterType::Lanczos3,
        );
        let mut master = RgbaImage::new(master_size, master_size);
        imageops::overlay(
            &mut master,
            &resized,
            i64::from((master_size - master_width) / 2),
            i64::from((master_size - master_height) / 2),
        );

        let mut frames = Vec::with_capacity(WINDOWS_ICON_SIZES.len());
        for size in WINDOWS_ICON_SIZES {
            let canvas = if size == master_size {
                master.clone()
            } else {
                imageops::resize(&master, size, size, FilterType::Lanczos3)
            };
            frames.push(
                IcoFrame::as_png(canvas.as_raw(), size, size, ExtendedColorType::Rgba8)
                    .map_err(|error| format!("no se pudo codificar el icono: {error}"))?,
            );
        }

        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("no se pudo crear {}: {error}", temporary.display()))?;
        IcoEncoder::new(&mut output)
            .encode_images(&frames)
            .map_err(|error| format!("no se pudo generar el ICO: {error}"))?;
        output
            .flush()
            .and_then(|_| output.sync_all())
            .map_err(|error| format!("no se pudo sincronizar {}: {error}", temporary.display()))?;
        replace_file(&temporary, &destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;

    Ok(relative)
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn ensure_managed_subdirectory(base: &Path, name: &str) -> Result<PathBuf, String> {
    validate_path_component(name)?;
    fs::create_dir_all(base)
        .map_err(|error| format!("no se pudo crear {}: {error}", base.display()))?;
    let canonical_base = base
        .canonicalize()
        .map_err(|error| format!("no se pudo resolver {}: {error}", base.display()))?;
    let candidate = canonical_base.join(name);

    if !candidate.exists() {
        fs::create_dir(&candidate)
            .map_err(|error| format!("no se pudo crear {}: {error}", candidate.display()))?;
    }
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| format!("no se pudo inspeccionar {}: {error}", candidate.display()))?;
    if !metadata.is_dir() || metadata_is_reparse_point(&metadata) {
        return Err(format!(
            "el directorio administrado no puede ser un enlace: {}",
            candidate.display()
        ));
    }
    let canonical_candidate = candidate
        .canonicalize()
        .map_err(|error| format!("no se pudo resolver {}: {error}", candidate.display()))?;
    if canonical_candidate.parent() != Some(canonical_base.as_path()) {
        return Err(format!(
            "el directorio administrado escapa de {}",
            canonical_base.display()
        ));
    }
    Ok(canonical_candidate)
}

fn ensure_regular_destination_if_exists(destination: &Path) -> Result<(), String> {
    if !destination.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(destination)
        .map_err(|error| format!("no se pudo inspeccionar {}: {error}", destination.display()))?;
    if !metadata.is_file() || metadata_is_reparse_point(&metadata) {
        return Err(format!(
            "el destino administrado no puede ser un enlace: {}",
            destination.display()
        ));
    }
    Ok(())
}

fn copy_file_atomic(source: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| format!("destino sin directorio: {}", destination.display()))?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("no se pudo inspeccionar {}: {error}", parent.display()))?;
    if !parent_metadata.is_dir() || metadata_is_reparse_point(&parent_metadata) {
        return Err(format!(
            "el directorio de destino no puede ser un enlace: {}",
            parent.display()
        ));
    }
    ensure_regular_destination_if_exists(destination)?;
    if source == destination {
        return Ok(());
    }
    let file_name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("asset");
    let temp = destination.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        fs::copy(source, &temp).map_err(|error| {
            format!(
                "no se pudo copiar {} a {}: {error}",
                source.display(),
                temp.display()
            )
        })?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&temp)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("no se pudo sincronizar {}: {error}", temp.display()))?;
        replace_file(&temp, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(crate) fn resolve_icon_path(config_path: &Path, icon: &str) -> Result<PathBuf, String> {
    validate_relative_path(icon)?;

    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    let canonical_base = base
        .canonicalize()
        .map_err(|error| format!("no se pudo resolver {}: {error}", base.display()))?;
    let candidate = base.join(icon);
    let canonical_candidate = candidate
        .canonicalize()
        .map_err(|error| format!("no se pudo leer el icono {}: {error}", candidate.display()))?;
    if !canonical_candidate.starts_with(&canonical_base) || !canonical_candidate.is_file() {
        return Err(format!(
            "el icono debe estar dentro de {}",
            canonical_base.display()
        ));
    }
    Ok(canonical_candidate)
}

fn validate_relative_path(raw: &str) -> Result<(), String> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!("la ruta debe ser relativa: '{raw}'"));
    }
    let mut count = 0;
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_string_lossy();
                if value.contains(':')
                    || value.ends_with(['.', ' '])
                    || is_reserved_windows_name(&value)
                {
                    return Err(format!("ruta no permitida: '{raw}'"));
                }
                count += 1;
            }
            _ => return Err(format!("ruta no permitida: '{raw}'")),
        }
    }
    if count == 0 {
        return Err("la ruta no puede estar vacia".to_string());
    }
    Ok(())
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

fn normalize_absolute_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = absolute_path(path)?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!(
                        "ruta invalida fuera de la raiz: {}",
                        path.display()
                    ));
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Ok(normalized)
}

fn path_for_comparison(path: &Path) -> Result<PathBuf, String> {
    let normalized = normalize_absolute_path(path)?;
    let mut ancestor = normalized.as_path();
    let mut missing = Vec::new();

    while !ancestor.exists() {
        let leaf = ancestor
            .file_name()
            .ok_or_else(|| format!("no se pudo normalizar {}", path.display()))?;
        missing.push(leaf.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| format!("no se pudo normalizar {}", path.display()))?;
    }

    let mut resolved = ancestor
        .canonicalize()
        .map_err(|error| format!("no se pudo resolver {}: {error}", ancestor.display()))?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn paths_refer_to_same_location(left: &Path, right: &Path) -> Result<bool, String> {
    let left = path_for_comparison(left)?;
    let right = path_for_comparison(right)?;

    #[cfg(windows)]
    {
        Ok(left
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy()))
    }

    #[cfg(not(windows))]
    {
        Ok(left == right)
    }
}

pub(crate) fn purge_profile_data(profile: &AppProfile) -> Result<(), String> {
    #[cfg(windows)]
    let roots = {
        let mut roots = Vec::new();
        for variable in ["LOCALAPPDATA", "APPDATA"] {
            if let Some(base) = env::var_os(variable) {
                let root = PathBuf::from(base)
                    .join("dev.misku.native-views")
                    .join("profiles");
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
        }
        if roots.is_empty() {
            return Err("LOCALAPPDATA y APPDATA no estan definidos".to_string());
        }
        roots
    };

    #[cfg(not(windows))]
    let roots = vec![
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
            .ok_or_else(|| "no se pudo resolver el directorio de datos local".to_string())?
            .join("dev.misku.native-views")
            .join("profiles"),
    ];

    let mut errors = Vec::new();
    for root in roots {
        if let Err(error) = remove_managed_directory(&root, profile.profile_key()) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn remove_managed_directory(root: &Path, key: &str) -> Result<(), String> {
    validate_path_component(key)?;
    let target = root.join(key);
    if !target.exists() {
        return Ok(());
    }

    let root_metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("no se pudo inspeccionar {}: {error}", root.display()))?;
    if !root_metadata.is_dir() || metadata_is_reparse_point(&root_metadata) {
        return Err(format!(
            "la raiz administrada no puede ser un enlace: {}",
            root.display()
        ));
    }
    let target_metadata = fs::symlink_metadata(&target)
        .map_err(|error| format!("no se pudo inspeccionar {}: {error}", target.display()))?;
    if !target_metadata.is_dir() || metadata_is_reparse_point(&target_metadata) {
        return Err(format!(
            "el perfil administrado no puede ser un enlace: {}",
            target.display()
        ));
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("no se pudo resolver {}: {error}", root.display()))?;
    let canonical_target = target
        .canonicalize()
        .map_err(|error| format!("no se pudo resolver {}: {error}", target.display()))?;
    if canonical_target.parent() != Some(canonical_root.as_path()) {
        return Err(format!(
            "se rechazo borrar una ruta fuera de {}",
            canonical_root.display()
        ));
    }
    fs::remove_dir_all(&canonical_target)
        .map_err(|error| format!("no se pudo borrar {}: {error}", canonical_target.display()))
}
fn legacy_schema_version() -> u32 {
    1
}

pub(crate) fn default_width() -> f64 {
    1200.0
}

pub(crate) fn default_height() -> f64 {
    800.0
}

pub(crate) fn default_min_width() -> f64 {
    640.0
}

pub(crate) fn default_min_height() -> f64 {
    480.0
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        env::temp_dir().join(format!("misku-{name}-{}", Uuid::new_v4()))
    }

    fn empty_request(urls: &[&str]) -> CreateRequest {
        CreateRequest {
            urls: urls.iter().map(|url| (*url).to_string()).collect(),
            id: None,
            name: None,
            icon: None,
            allowed_origins: Vec::new(),
            allow_insecure_http: false,
        }
    }

    #[test]
    fn creates_distinct_apps_for_paths_on_same_host() {
        let root = test_dir("same-host");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("apps.toml");
        let mut config = AppsConfig::default();

        let created = create_profiles(
            &mut config,
            &path,
            empty_request(&[
                "https://github.com/openai/project",
                "https://github.com/microsoft/project",
            ]),
        )
        .unwrap();

        assert_eq!(created.len(), 2);
        assert_ne!(created[0].id, created[1].id);
        assert_ne!(created[0].instance_id(), created[1].instance_id());
        assert_ne!(created[0].profile_key(), created[1].profile_key());
        assert_eq!(config.apps.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creates_distinct_apps_for_the_exact_same_url() {
        let root = test_dir("same-url");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("apps.toml");
        let mut config = AppsConfig::default();

        let first = create_profiles(
            &mut config,
            &path,
            empty_request(&["https://mail.example.com/inbox"]),
        )
        .unwrap();
        let second = create_profiles(
            &mut config,
            &path,
            empty_request(&["https://mail.example.com/inbox"]),
        )
        .unwrap();

        assert_ne!(first[0].id, second[0].id);
        assert_ne!(first[0].instance_id(), second[0].instance_id());
        assert_eq!(config.apps.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn update_preserves_instance_and_profile_key() {
        let root = test_dir("update");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("apps.toml");
        let mut config = AppsConfig::default();
        let created = create_profiles(
            &mut config,
            &path,
            empty_request(&["https://example.com/A?Token=X"]),
        )
        .unwrap()
        .remove(0);

        let updated = update_profile(
            &mut config,
            &path,
            UpdateRequest {
                selector: created.id.clone(),
                url: Some("https://example.com/a?Token=x".to_string()),
                id: Some("renamed".to_string()),
                name: Some("Renamed".to_string()),
                icon: None,
                clear_icon: false,
                allowed_origins: None,
                allow_insecure_http: None,
            },
        )
        .unwrap();

        assert_eq!(updated.instance_id(), created.instance_id());
        assert_eq!(updated.profile_key(), created.profile_key());
        assert_eq!(updated.id, "renamed");
        assert_eq!(updated.url, "https://example.com/a?Token=x");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_profile_gets_stable_identity_and_keeps_data_key() {
        let root = test_dir("legacy");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("apps.toml");
        let content = r#"
[[apps]]
id = "legacy"
name = "Legacy"
url = "https://example.com/A"
isolated_profile = false
"#;
        fs::write(&path, content).unwrap();
        let first = load_config(&path).unwrap();
        let second = load_config(&path).unwrap();

        assert_eq!(first.apps[0].instance_id(), second.apps[0].instance_id());
        assert_eq!(first.apps[0].profile_key(), "legacy");
        assert!(first.apps[0].uses_legacy_data_root());
        assert!(first.apps[0].isolated_profile);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_loopback_http_is_migrated_with_explicit_runtime_permission() {
        let root = test_dir("legacy-loopback");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("apps.toml");
        fs::write(
            &path,
            r#"
[[apps]]
id = "local-dev"
name = "Local dev"
url = "http://localhost:3000/app"
"#,
        )
        .unwrap();

        let config = load_config(&path).unwrap();
        assert!(config.apps[0].allow_insecure_http);
        assert!(config.apps[0].uses_legacy_data_root());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn schema_two_rejects_shared_webview_storage() {
        let root = test_dir("shared-storage");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("apps.toml");
        let mut config = AppsConfig::default();
        create_profiles(
            &mut config,
            &path,
            empty_request(&["https://example.com/shared"]),
        )
        .unwrap();
        config.apps[0].isolated_profile = false;

        let error = validate_config(&config, Some(&path)).unwrap_err();
        assert!(error.contains("isolated_profile = true"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unsafe_ids_and_remote_http() {
        for id in ["../escape", "x/y", "CON", "trailing.", "UPPER"] {
            assert!(validate_id(id).is_err(), "{id} should be rejected");
        }
        assert!(parse_app_url("http://example.com", true).is_err());
        assert!(parse_app_url("http://localhost:3000", false).is_err());
        assert!(parse_app_url("http://localhost:3000", true).is_ok());
        assert!(parse_app_url("https://user:secret@example.com", false).is_err());
        assert!(is_reserved_windows_name("CON.ico"));
        assert!(is_reserved_windows_name("com1.png"));
        assert!(validate_relative_path("icons/CON.ico").is_err());
    }

    #[test]
    fn saves_atomically_and_exports_one_app_manifest() {
        let root = test_dir("atomic-export");
        fs::create_dir_all(&root).unwrap();
        let registry = root.join("apps.toml");
        let mut config = AppsConfig::default();
        let profile = create_profiles(
            &mut config,
            &registry,
            empty_request(&["https://example.com/App?Token=X"]),
        )
        .unwrap()
        .remove(0);
        save_config_atomic(&registry, &config).unwrap();
        assert!(!profile.uses_legacy_data_root());

        let equivalent_registry = root.join("missing").join("..").join("apps.toml");
        let registry_before = fs::read(&registry).unwrap();
        assert!(export_profile(&registry, &profile, &equivalent_registry).is_err());
        assert_eq!(fs::read(&registry).unwrap(), registry_before);

        let manifest = root
            .join("apps")
            .join(profile.instance_id().to_string())
            .join("app.toml");
        export_profile(&registry, &profile, &manifest).unwrap();
        let exported = load_config(&manifest).unwrap();

        assert_eq!(exported.apps.len(), 1);
        assert_eq!(exported.apps[0].instance_id(), profile.instance_id());
        assert_eq!(exported.apps[0].url, "https://example.com/App?Token=X");
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn normalizes_png_icons_to_multiresolution_ico() {
        use image::{ImageEncoder, codecs::png::PngEncoder};

        let root = test_dir("normalize-icon");
        fs::create_dir_all(&root).unwrap();
        let registry = root.join("apps.toml");
        let source = root.join("source.png");
        let pixels = vec![0x7f_u8; 48 * 24 * 4];
        let mut png = File::create(&source).unwrap();
        PngEncoder::new(&mut png)
            .write_image(&pixels, 48, 24, ExtendedColorType::Rgba8)
            .unwrap();
        png.sync_all().unwrap();

        let instance_id = Uuid::new_v4();
        let relative = import_icon(&registry, source.to_str().unwrap(), instance_id).unwrap();
        assert!(relative.starts_with(&format!("icons/{instance_id}-")));
        assert!(relative.ends_with(".ico"));

        let destination = root.join(&relative);
        let encoded = fs::read(&destination).unwrap();
        assert_eq!(&encoded[0..4], &[0, 0, 1, 0]);
        assert_eq!(u16::from_le_bytes([encoded[4], encoded[5]]), 7);
        let decoded = ImageReader::open(&destination).unwrap().decode().unwrap();
        assert!(decoded.width() <= 256);
        assert!(decoded.height() <= 256);

        let second_relative =
            import_icon(&registry, source.to_str().unwrap(), instance_id).unwrap();
        assert_ne!(second_relative, relative);
        assert!(destination.is_file());
        assert!(root.join(second_relative).is_file());

        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn copies_assets_atomically_with_a_flushable_handle() {
        let root = test_dir("copy-asset");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.ico");
        let target_dir = root.join("target");
        fs::create_dir_all(&target_dir).unwrap();
        let destination = target_dir.join("copied.ico");
        fs::write(&source, b"icon-bytes").unwrap();

        copy_file_atomic(&source, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"icon-bytes");
        assert!(fs::read_dir(&target_dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        fs::remove_dir_all(root).unwrap();
    }
}
