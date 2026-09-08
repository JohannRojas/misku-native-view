//! Managed native installations also work when Node and PowerShell are absent.
use crate::{
    model::{self, AppProfile, UpdateRequest},
    platform,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn managed_home() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("MISKU_NV_HOME") {
        return crate::absolute_path(Path::new(&path));
    }
    env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("Misku Native Views"))
        .ok_or_else(|| "No se pudo encontrar la carpeta local de aplicaciones".into())
}

pub(crate) fn default_registry() -> Result<PathBuf, String> {
    let local = managed_home()?.join("apps.toml");
    if env::var_os("MISKU_NV_HOME").is_none()
        && !local.exists()
        && let Some(roaming) = env::var_os("APPDATA")
    {
        let legacy = PathBuf::from(roaming).join("Misku Native Views/apps.toml");
        if legacy.is_file() {
            return Ok(legacy);
        }
    }
    Ok(local)
}

fn programs_directory() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("MISKU_NV_PROGRAMS_DIR") {
        return crate::absolute_path(Path::new(&path));
    }
    env::var_os("APPDATA")
        .map(|p| PathBuf::from(p).join("Microsoft/Windows/Start Menu/Programs/Misku Native Views"))
        .ok_or_else(|| "No se pudo encontrar el menú Inicio".into())
}

pub(crate) fn ensure_runtime() -> Result<PathBuf, String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let bytes = fs::read(&exe).map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let runtimes = model::ensure_managed_subdirectory(&managed_home()?, "runtimes")?;
    let directory = model::ensure_managed_subdirectory(
        &runtimes,
        &format!("{}-{}", env!("CARGO_PKG_VERSION"), &hash[..12]),
    )?;
    let destination = directory.join("misku-native-views.exe");
    if destination.exists() {
        regular_file(&destination)?;
        let installed = fs::read(&destination).map_err(|e| e.to_string())?;
        if Sha256::digest(installed) != Sha256::digest(&bytes) {
            return Err("El runtime instalado no supera la verificación de integridad".into());
        }
    } else {
        model::copy_file_atomic(&exe, &destination)?;
    }
    Ok(destination)
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct NativeInstallation {
    pub(crate) instance_id: uuid::Uuid,
    pub(crate) config: PathBuf,
    pub(crate) runtime: PathBuf,
    pub(crate) shortcut: PathBuf,
    pub(crate) registry: PathBuf,
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    manifest_hash: String,
}

fn shortcut_name(profile: &AppProfile) -> String {
    let name: String = profile
        .name
        .chars()
        .take(65)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    format!(
        "{}-{}-{}.lnk",
        name.trim_matches([' ', '.']),
        profile.id,
        &profile.instance_id().to_string()[..8]
    )
}

fn regular_file(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("No se permiten enlaces en los archivos administrados".into());
        }
    }
    if !meta.is_file() {
        return Err("Se esperaba un archivo regular".into());
    }
    Ok(())
}

pub(crate) fn materialize(
    registry: &Path,
    profile: &AppProfile,
    runtime: &Path,
) -> Result<NativeInstallation, String> {
    let apps = model::ensure_managed_subdirectory(&managed_home()?, "apps")?;
    let app_dir = model::ensure_managed_subdirectory(&apps, &profile.instance_id().to_string())?;
    let manifest = app_dir.join("app.toml");
    let mut usable = profile.clone();
    if let Some(icon) = &profile.icon
        && model::resolve_icon_path(registry, icon).is_err()
    {
        usable.icon = None;
    }
    let exported = model::export_profile(registry, &usable, &manifest)?;
    let programs = programs_directory()?;
    fs::create_dir_all(&programs).map_err(|e| e.to_string())?;
    // Canonicalize the trusted root and check the exact leaf before replacing it.
    let programs = programs.canonicalize().map_err(|e| e.to_string())?;
    let shortcut = programs.join(shortcut_name(profile));
    if shortcut.exists() {
        regular_file(&shortcut)?;
    }
    let icon = exported
        .icon
        .as_deref()
        .and_then(|p| model::resolve_icon_path(&manifest, p).ok())
        .unwrap_or_else(|| runtime.to_path_buf());
    platform::create_shortcut(
        &shortcut,
        runtime,
        &manifest,
        &profile.instance_id().to_string(),
        &icon,
    )?;
    let manifest_hash = file_hash(&manifest)?;
    let record = NativeInstallation {
        instance_id: profile.instance_id(),
        config: manifest,
        runtime: runtime.to_owned(),
        shortcut,
        registry: registry.to_owned(),
        fingerprint: fingerprint(profile)?,
        manifest_hash,
    };
    let record_path = app_dir.join("native-install.json");
    if record_path.exists() {
        regular_file(&record_path)?;
    }
    for previous in registered_shortcuts(&app_dir, profile.instance_id())? {
        if previous.canonicalize().ok() != record.shortcut.canonicalize().ok() {
            remove_owned_shortcut(&previous, profile.instance_id())?;
        }
    }
    let temp = app_dir.join(format!(".install-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(&record).map_err(|e| e.to_string())?)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        model::copy_file_atomic(&temp, &record_path)
    })();
    let _ = fs::remove_file(temp);
    result?;
    Ok(record)
}

fn fingerprint(profile: &AppProfile) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(profile).map_err(|e| e.to_string())?)
    ))
}

fn file_hash(path: &Path) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(fs::read(path).map_err(|e| e.to_string())?)
    ))
}

fn registered_shortcuts(directory: &Path, id: uuid::Uuid) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for (file, identity, shortcut) in [
        ("native-install.json", "instance_id", "shortcut"),
        ("install.json", "instanceId", "shortcutPath"),
    ] {
        let path = directory.join(file);
        if !path.exists() {
            continue;
        }
        regular_file(&path)?;
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
            continue;
        }
        if let Ok(data) = fs::read(path)
            && let Ok(record) = serde_json::from_slice::<serde_json::Value>(&data)
            && record.get(identity).and_then(|v| v.as_str()) == Some(&id.to_string())
            && let Some(path) = record.get(shortcut).and_then(|v| v.as_str())
        {
            paths.push(PathBuf::from(path));
        }
    }
    Ok(paths)
}

pub(crate) fn ensure_installed(
    registry: &Path,
    profile: &AppProfile,
    runtime: &Path,
) -> Result<NativeInstallation, String> {
    let apps = model::ensure_managed_subdirectory(&managed_home()?, "apps")?;
    let directory = model::ensure_managed_subdirectory(&apps, &profile.instance_id().to_string())?;
    let record_path = directory.join("native-install.json");
    if regular_file(&record_path).is_ok()
        && let Ok(bytes) = fs::read(record_path)
        && let Ok(record) = serde_json::from_slice::<NativeInstallation>(&bytes)
        && record.instance_id == profile.instance_id()
        && record.registry == registry
        && record.runtime == runtime
        && record.config == directory.join("app.toml")
        && record.fingerprint == fingerprint(profile)?
        && programs_directory()?
            .canonicalize()
            .is_ok_and(|dir| record.shortcut == dir.join(shortcut_name(profile)))
        && regular_file(&record.config).is_ok()
        && regular_file(&record.shortcut).is_ok()
        && file_hash(&record.config).ok().as_ref() == Some(&record.manifest_hash)
        && model::load_profile_for_open(&record.config, Some(&profile.instance_id().to_string()))
            .is_ok()
    {
        return Ok(record);
    }
    materialize(registry, profile, runtime)
}

pub(crate) fn open(config: &Path, profile: &AppProfile, runtime: &Path) -> Result<(), String> {
    regular_file(runtime)?;
    let mut command = Command::new(runtime);
    command
        .args(["--config"])
        .arg(config)
        .args(["--app", &profile.instance_id().to_string()]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .spawn()
        .map_err(|e| format!("No se pudo abrir la app: {e}"))?;
    Ok(())
}

fn remove_owned_shortcut(path: &Path, id: uuid::Uuid) -> Result<(), String> {
    let programs = programs_directory()?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let suffix = format!("-{}.lnk", &id.to_string()[..8]);
    if path.parent().and_then(|p| p.canonicalize().ok()).as_ref() != Some(&programs)
        || !path
            .file_name()
            .is_some_and(|p| p.to_string_lossy().ends_with(&suffix))
    {
        return Err("El acceso directo no pertenece a esta app".into());
    }
    if path.exists() {
        regular_file(path)?;
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn remove_shortcut(profile: &AppProfile) -> Result<(), String> {
    let programs = programs_directory()?;
    if programs.exists() {
        let directory = managed_home()?
            .join("apps")
            .join(profile.instance_id().to_string());
        if directory.exists() {
            for previous in registered_shortcuts(&directory, profile.instance_id())? {
                remove_owned_shortcut(&previous, profile.instance_id())?;
            }
        }
        remove_owned_shortcut(
            &programs
                .canonicalize()
                .map_err(|e| e.to_string())?
                .join(shortcut_name(profile)),
            profile.instance_id(),
        )?;
    }
    Ok(())
}

/// Browser-discovered icons are an optional enhancement after the page opens.
pub(crate) fn save_discovered_icon(
    config: &Path,
    original: &AppProfile,
    bytes: &[u8],
) -> Result<Option<PathBuf>, String> {
    if bytes.is_empty() || bytes.len() > 2 * 1024 * 1024 {
        return Ok(None);
    }
    let registry = default_registry()?;
    let target = if config == registry {
        registry
    } else {
        // Only follow our own installation record inside the managed UUID folder.
        let expected = managed_home()?
            .join("apps")
            .join(original.instance_id().to_string());
        if expected.canonicalize().ok() != config.parent().and_then(|p| p.canonicalize().ok()) {
            return Ok(None);
        }
        let record_path = expected.join("native-install.json");
        regular_file(&record_path)?;
        let record: NativeInstallation =
            serde_json::from_slice(&fs::read(record_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if record.instance_id != original.instance_id() {
            return Ok(None);
        }
        record.registry
    };
    let _lock = model::acquire_config_lock(&target)?;
    let mut current = model::load_config(&target)?;
    let latest = model::select_profile(&current, Some(&original.instance_id().to_string()))?;
    if latest.url != original.url || latest.icon.is_some() {
        return Ok(None);
    }
    let temporary = target.with_file_name(format!(".favicon-{}.png", uuid::Uuid::new_v4()));
    {
        use std::io::Write;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .and_then(|mut f| f.write_all(bytes))
            .map_err(|e| e.to_string())?;
    }
    let result = model::update_profile(
        &mut current,
        &target,
        UpdateRequest {
            selector: original.instance_id().to_string(),
            icon: Some(temporary.to_string_lossy().into()),
            ..Default::default()
        },
    );
    let _ = fs::remove_file(temporary);
    let updated = result?;
    model::save_config_atomic(&target, &current)?;
    let runtime = env::current_exe().map_err(|e| e.to_string())?;
    materialize(&target, &updated, &runtime)?;
    updated
        .icon
        .as_deref()
        .map(|p| model::resolve_icon_path(&target, p))
        .transpose()
}
