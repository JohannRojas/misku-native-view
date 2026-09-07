#!/usr/bin/env node

"use strict";

const { spawn, spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const packageJson = require("../package.json");
const { discoverFavicon } = require("./favicon.js");

const packageRoot = path.resolve(__dirname, "..");
const packagedNativeExe = path.join(packageRoot, "runtime", "misku-native-views.exe");
const shortcutScript = path.join(packageRoot, "scripts", "manage-shortcut.ps1");
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const WINDOWS_RESERVED_NAME = /^(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)/i;
const MAX_JSON_BYTES = 1024 * 1024;
const MAX_NATIVE_OUTPUT_BYTES = 16 * 1024 * 1024;
const MAX_FAVICON_CONCURRENCY = 4;
const STALE_FAVICON_AGE_MS = 24 * 60 * 60 * 1000;
const TEMP_FAVICON_PATTERN =
  /^misku-nv-favicon-\d+-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.(?:ico|png)$/;

function fail(message) {
  const error = new Error(message);
  error.isMiskuError = true;
  throw error;
}

function printHelp() {
  console.log(`Misku Native Views CLI

Uso:
  misku-nv                 Abre el gestor gráfico
  misku-nv manage          Abre el gestor gráfico
  misku-nv <url> [--name <nombre>] [--id <id>] [--icon <ruta>] [--no-open]
  misku-nv <id|uuid>
  misku-nv create <url> [url...]
  misku-nv update <id|uuid> [opciones]
  misku-nv remove <id|uuid> [--purge-data]
  misku-nv --list [--json]
  misku-nv repair

Cada URL crea una app independiente con UUID, perfil WebView, manifiesto y acceso
directo propios. Repetir exactamente la misma URL crea otra instancia.

Opciones de create:
  -n, --name <nombre>       Nombre visible.
  -i, --id <id>             Alias visible; no es la identidad interna.
  --icon <ruta>             Importa un icono .ico o .png y omite la descarga.
  --no-favicon              Usa el icono generico sin consultar la pagina.
  --allow-origin <origen>   Origen adicional permitido dentro de la WebView.
  --allow-http              Solo permite HTTP para localhost/loopback.
  --no-open                 Crea e instala sin abrir.

Opciones de update:
  --suspend-on-minimize     Pausa al minimizar (puede interrumpir audio y avisos).
  --keep-active             Mantiene activa la app al minimizar (predeterminado).
  --refresh-icon            Vuelve a descargar el favicon sin cambiar el UUID.

El comando publico permanece: misku-nv`);
}

function getContext(environment = process.env) {
  const localAppData =
    environment.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local");
  const roamingAppData =
    environment.APPDATA || path.join(os.homedir(), "AppData", "Roaming");
  const hasHomeOverride = Boolean(environment.MISKU_NV_HOME);
  const home = path.resolve(
    environment.MISKU_NV_HOME ||
      path.join(localAppData, "Misku Native Views")
  );
  const localRegistry = path.join(home, "apps.toml");
  const legacyRegistry = path.resolve(
    path.join(roamingAppData, "Misku Native Views", "apps.toml")
  );
  const registry =
    !hasHomeOverride &&
    !fs.existsSync(localRegistry) &&
    fs.existsSync(legacyRegistry)
      ? legacyRegistry
      : localRegistry;
  const programsDir = path.resolve(
    environment.MISKU_NV_PROGRAMS_DIR ||
      path.join(
        roamingAppData,
        "Microsoft",
        "Windows",
        "Start Menu",
        "Programs",
        "Misku Native Views"
      )
  );
  const nativeExe = path.resolve(
    environment.MISKU_NV_NATIVE_EXE || packagedNativeExe
  );

  return {
    home,
    programsDir,
    nativeExe,
    registry,
    appsDir: path.join(home, "apps"),
    runtimesDir: path.join(home, "runtimes")
  };
}

function extractGlobalOptions(rawArgs) {
  const args = [];
  let configPath = null;
  let configSeen = false;
  let outputJson = false;

  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === "--json") {
      outputJson = true;
      continue;
    }
    if (arg === "--config" || arg === "-c") {
      const value = rawArgs[index + 1];
      if (!value) {
        fail(`falta el valor despues de ${arg}`);
      }
      if (configSeen) {
        fail("--config solo se puede indicar una vez");
      }
      configPath = path.resolve(value);
      configSeen = true;
      index += 1;
      continue;
    }
    if (arg.startsWith("--config=")) {
      const value = arg.slice("--config=".length);
      if (!value) {
        fail("falta el valor despues de --config=");
      }
      if (configSeen) {
        fail("--config solo se puede indicar una vez");
      }
      configPath = path.resolve(value);
      configSeen = true;
      continue;
    }
    args.push(arg);
  }

  return { args, configPath, outputJson };
}

function getExplicitConfig(args) {
  return extractGlobalOptions(args).configPath;
}

function withConfig(args, configPath) {
  return getExplicitConfig(args) ? [...args] : ["--config", configPath, ...args];
}

function assertRegularFile(filePath, label) {
  let stats;
  try {
    stats = fs.lstatSync(filePath);
  } catch (error) {
    fail(`no se pudo leer ${label} ${filePath}: ${error.message}`);
  }
  if (!stats.isFile() || stats.isSymbolicLink()) {
    fail(`${label} debe ser un archivo normal y no un enlace: ${filePath}`);
  }
  return stats;
}

function ensureDirectoryNoLinks(directory, label = "directorio administrado") {
  try {
    fs.mkdirSync(directory, { recursive: true });
  } catch (error) {
    fail(`no se pudo crear ${label} ${directory}: ${error.message}`);
  }

  let stats;
  try {
    stats = fs.lstatSync(directory);
  } catch (error) {
    fail(`no se pudo inspeccionar ${label} ${directory}: ${error.message}`);
  }
  if (!stats.isDirectory() || stats.isSymbolicLink()) {
    fail(`${label} no puede ser un enlace ni otro tipo de archivo: ${directory}`);
  }
  return directory;
}

function assertDirectoryNoLinksIfExists(directory, label = "directorio administrado") {
  if (!fs.existsSync(directory)) {
    return false;
  }
  const stats = fs.lstatSync(directory);
  if (!stats.isDirectory() || stats.isSymbolicLink()) {
    fail(`${label} no puede ser un enlace ni otro tipo de archivo: ${directory}`);
  }
  return true;
}

function ensureRegistry(configPath) {
  ensureDirectoryNoLinks(path.dirname(configPath), "directorio de configuracion");
  if (fs.existsSync(configPath)) {
    assertRegularFile(configPath, "la configuracion");
    return;
  }

  const initial = [
    "# Registro administrado por misku-nv.",
    "schema_version = 2",
    "apps = []",
    ""
  ].join("\n");
  try {
    fs.writeFileSync(configPath, initial, { encoding: "utf8", flag: "wx" });
  } catch (error) {
    if (error.code !== "EEXIST") {
      fail(`no se pudo crear la configuracion ${configPath}: ${error.message}`);
    }
    assertRegularFile(configPath, "la configuracion");
  }
}

function fileSha256(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function randomToken() {
  return `${process.pid}-${Date.now()}-${crypto.randomBytes(6).toString("hex")}`;
}

function replaceFileAtomic(temporary, destination) {
  if (fs.existsSync(destination)) {
    assertRegularFile(destination, "el archivo de destino");
  }

  try {
    fs.renameSync(temporary, destination);
    return;
  } catch (error) {
    if (
      !fs.existsSync(destination) ||
      !["EACCES", "EEXIST", "EPERM"].includes(error.code)
    ) {
      throw error;
    }
  }

  const backup = `${destination}.${randomToken()}.replace-backup`;
  fs.renameSync(destination, backup);
  try {
    fs.renameSync(temporary, destination);
  } catch (error) {
    try {
      fs.renameSync(backup, destination);
    } catch (restoreError) {
      error.message += `; tampoco se pudo restaurar ${destination}: ${restoreError.message}`;
    }
    throw error;
  }
  fs.rmSync(backup, { force: true, maxRetries: 3, retryDelay: 50 });
}

function copyFileAtomic(source, destination) {
  ensureDirectoryNoLinks(path.dirname(destination));
  const temporary = `${destination}.${randomToken()}.tmp`;
  try {
    fs.copyFileSync(source, temporary, fs.constants.COPYFILE_EXCL);
    const descriptor = fs.openSync(temporary, "r+");
    try {
      fs.fsyncSync(descriptor);
    } finally {
      fs.closeSync(descriptor);
    }
    replaceFileAtomic(temporary, destination);
  } finally {
    fs.rmSync(temporary, { force: true });
  }
}

function writeJsonAtomic(filePath, value) {
  ensureDirectoryNoLinks(path.dirname(filePath));
  if (fs.existsSync(filePath)) {
    assertRegularFile(filePath, "el archivo JSON");
  }

  const content = `${JSON.stringify(value, null, 2)}\n`;
  if (fs.existsSync(filePath) && fs.readFileSync(filePath, "utf8") === content) {
    return;
  }

  const temporary = `${filePath}.${randomToken()}.tmp`;
  try {
    fs.writeFileSync(temporary, content, {
      encoding: "utf8",
      flag: "wx"
    });
    const descriptor = fs.openSync(temporary, "r+");
    try {
      fs.fsyncSync(descriptor);
    } finally {
      fs.closeSync(descriptor);
    }
    replaceFileAtomic(temporary, filePath);
  } finally {
    fs.rmSync(temporary, { force: true });
  }
}

function ensureRuntime(context) {

  assertRegularFile(context.nativeExe, "el runtime nativo");
  ensureDirectoryNoLinks(context.home, "directorio principal administrado");
  ensureDirectoryNoLinks(context.runtimesDir, "directorio de runtimes");

  const hash = fileSha256(context.nativeExe);
  const version = String(packageJson.version).replace(/[^a-zA-Z0-9._-]/g, "-");
  const runtimeDir = safeJoin(
    context.runtimesDir,
    `${version}-${hash.slice(0, 12)}`
  );
  ensureDirectoryNoLinks(runtimeDir, "directorio del runtime");
  const destination = safeJoin(runtimeDir, "misku-native-views.exe");

  if (fs.existsSync(destination)) {
    assertRegularFile(destination, "el runtime administrado");
    if (fileSha256(destination) === hash) {
      return destination;
    }
  }

  copyFileAtomic(context.nativeExe, destination);
  if (fileSha256(destination) !== hash) {
    fail(`el runtime administrado no coincide con el paquete: ${destination}`);
  }
  return destination;
}

function looksLikeUrl(value) {
  return (
    /^https?:\/\//i.test(value) ||
    /^localhost(?::\d+)?(?:\/|$)/i.test(value) ||
    /^[^\s.]+\.[^\s]+/.test(value)
  );
}

function parseFriendlyUrlCommand(rawArgs) {
  const createArgs = [];
  const urls = [];
  let openAfterCreate = true;
  let outputJson = false;
  let fetchFavicon = true;

  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === "--no-favicon") {
      fetchFavicon = false;
      continue;
    }
    if (arg === "--no-open") {
      openAfterCreate = false;
      continue;
    }
    if (arg === "--open") {
      openAfterCreate = true;
      continue;
    }
    if (arg === "--json") {
      outputJson = true;
      continue;
    }
    if (
      arg === "--name" ||
      arg === "-n" ||
      arg === "--id" ||
      arg === "-i" ||
      arg === "--icon" ||
      arg === "--allow-origin" ||
      arg === "--config" ||
      arg === "-c"
    ) {
      const value = rawArgs[index + 1];
      if (!value) {
        fail(`falta el valor despues de ${arg}`);
      }
      createArgs.push(arg, arg === "--icon" ? path.resolve(value) : value);
      index += 1;
      continue;
    }
    if (arg === "--allow-http") {
      createArgs.push(arg);
      continue;
    }
    if (
      arg.startsWith("--name=") ||
      arg.startsWith("--id=") ||
      arg.startsWith("--allow-origin=") ||
      arg.startsWith("--config=")
    ) {
      createArgs.push(arg);
      continue;
    }
    if (arg.startsWith("--icon=")) {
      createArgs.push(`--icon=${path.resolve(arg.slice("--icon=".length))}`);
      continue;
    }
    if (!arg.startsWith("-") && looksLikeUrl(arg)) {
      urls.push(arg);
      continue;
    }
    return null;
  }

  if (urls.length === 0) {
    return null;
  }
  return {
    createArgs: [...createArgs, ...urls],
    urls,
    openAfterCreate,
    outputJson,
    fetchFavicon
  };
}

function normalizeIconArgs(args) {
  const normalized = [];
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--icon") {
      const value = args[index + 1];
      if (!value) {
        fail("falta el valor despues de --icon");
      }
      normalized.push(arg, path.resolve(value));
      index += 1;
    } else if (arg.startsWith("--icon=")) {
      normalized.push(`--icon=${path.resolve(arg.slice("--icon=".length))}`);
    } else {
      normalized.push(arg);
    }
  }
  return normalized;
}

function extractFaviconWrapperOptions(args, command) {
  const cleaned = [];
  let disableFavicon = false;
  let refreshFavicon = false;

  for (const arg of args) {
    if (arg === "--no-favicon") {
      disableFavicon = true;
      continue;
    }
    if (arg === "--refresh-icon") {
      refreshFavicon = true;
      continue;
    }
    cleaned.push(arg);
  }

  const hasExplicitIcon = cleaned.some(
    (arg) =>
      arg === "--icon" ||
      arg.startsWith("--icon=") ||
      arg === "--clear-icon"
  );
  if (refreshFavicon && command !== "update") {
    fail("--refresh-icon solo se puede usar con update");
  }
  if (disableFavicon && command !== "create") {
    fail("--no-favicon solo se puede usar al crear una app");
  }
  if (refreshFavicon && hasExplicitIcon) {
    fail("--refresh-icon no se puede combinar con --icon ni --clear-icon");
  }

  return {
    args: cleaned,
    autoFavicon: command === "create" && !disableFavicon,
    refreshFavicon
  };
}

function runNative(executable, args, options = {}) {
  const result = spawnSync(executable, args, {
    cwd: options.cwd || process.cwd(),
    encoding: "utf8",
    stdio: options.inherit ? "inherit" : "pipe",
    windowsHide: true,
    maxBuffer: MAX_NATIVE_OUTPUT_BYTES,
    shell: false
  });
  if (result.error) {
    fail(result.error.message);
  }
  if (result.signal) {
    fail(`el proceso nativo termino por ${result.signal}`);
  }
  if ((result.status ?? 1) !== 0) {
    const message = [result.stderr, result.stdout]
      .filter(Boolean)
      .join("\n")
      .trim();
    fail(message || `el proceso nativo termino con codigo ${result.status}`);
  }
  return result;
}

function invokeJson(context, configPath, args) {
  const nativeArgs = [
    "--json",
    ...withConfig(
      args.filter((arg) => arg !== "--json"),
      configPath
    )
  ];
  const result = runNative(context.nativeExe, nativeArgs);
  if (result.stderr) {
    process.stderr.write(result.stderr);
  }
  try {
    return JSON.parse(result.stdout || "null");
  } catch (error) {
    fail(
      `el runtime devolvio JSON invalido: ${error.message}\n${result.stdout || ""}`
    );
  }
}

function expectArray(value, operation) {
  if (!Array.isArray(value)) {
    fail(`el runtime devolvio un resultado inesperado para ${operation}`);
  }
  return value;
}

function assertUuid(value) {
  if (!UUID_PATTERN.test(String(value))) {
    fail(`UUID de app invalido: ${value}`);
  }
  return String(value).toLowerCase();
}

function safeJoin(root, ...parts) {
  const absoluteRoot = path.resolve(root);
  const candidate = path.resolve(absoluteRoot, ...parts);
  const relative = path.relative(absoluteRoot, candidate);
  if (
    relative === "" ||
    (!relative.startsWith(`..${path.sep}`) &&
      relative !== ".." &&
      !path.isAbsolute(relative))
  ) {
    return candidate;
  }
  fail(`ruta fuera del directorio administrado: ${candidate}`);
}

function writeTemporaryFavicon(favicon) {
  if (!favicon || !Buffer.isBuffer(favicon.bytes)) {
    fail("la descarga del favicon no devolvio bytes validos");
  }
  if (!["ico", "png"].includes(favicon.extension)) {
    fail("la descarga del favicon devolvio un formato no permitido");
  }

  const filePath = safeJoin(
    os.tmpdir(),
    `misku-nv-favicon-${process.pid}-${crypto.randomUUID()}.${favicon.extension}`
  );
  try {
    fs.writeFileSync(filePath, favicon.bytes, {
      flag: "wx",
      mode: 0o600
    });
    const descriptor = fs.openSync(filePath, "r+");
    try {
      fs.fsyncSync(descriptor);
    } finally {
      fs.closeSync(descriptor);
    }
    return { filePath };
  } catch (error) {
    fs.rmSync(filePath, { force: true });
    throw error;
  }
}

function removeTemporaryFavicon(temporary) {
  fs.rmSync(temporary.filePath, { force: true });
}

function cleanupStaleTemporaryFavicons(
  maximumAgeMs = STALE_FAVICON_AGE_MS,
  now = Date.now()
) {
  let entries;
  try {
    entries = fs.readdirSync(os.tmpdir(), { withFileTypes: true });
  } catch {
    return;
  }

  for (const entry of entries) {
    if (!entry.isFile() || !TEMP_FAVICON_PATTERN.test(entry.name)) {
      continue;
    }
    try {
      const filePath = safeJoin(os.tmpdir(), entry.name);
      const stats = fs.lstatSync(filePath);
      if (
        stats.isSymbolicLink() ||
        !stats.isFile() ||
        now - stats.mtimeMs < maximumAgeMs
      ) {
        continue;
      }
      fs.rmSync(filePath, { force: true });
    } catch {
      // La limpieza stale es best-effort y nunca bloquea la creacion.
    }
  }
}

async function mapWithConcurrency(items, maximumConcurrency, mapper) {
  if (items.length === 0) {
    return [];
  }
  const results = new Array(items.length);
  let nextIndex = 0;
  const workers = Array.from(
    { length: Math.min(maximumConcurrency, items.length) },
    async () => {
      while (nextIndex < items.length) {
        const index = nextIndex;
        nextIndex += 1;
        results[index] = await mapper(items[index], index);
      }
    }
  );
  await Promise.all(workers);
  return results;
}

async function hydrateProfileFavicons(
  context,
  configPath,
  results,
  options = {}
) {
  const force = Boolean(options.force);
  const discover = options.discoverFavicon || discoverFavicon;
  const invoke = options.invokeJson || invokeJson;
  const writeTemporary =
    options.writeTemporaryFavicon || writeTemporaryFavicon;
  const removeTemporary =
    options.removeTemporaryFavicon || removeTemporaryFavicon;
  const hydrated = results.map((result) => ({ ...result }));
  const targets = hydrated
    .map((result, index) => ({
      index,
      result,
      profile: result.profile || result
    }))
    .filter(({ profile }) => force || !profile.icon);
  const cache = new Map();

  if (options.cleanupStale !== false) {
    cleanupStaleTemporaryFavicons();
  }

  const prepared = await mapWithConcurrency(
    targets,
    MAX_FAVICON_CONCURRENCY,
    async (target) => {
      const profile = target.profile;
      const cacheKey =
        String(profile.url) + "\u0000" + Boolean(profile.allow_insecure_http);
      if (!cache.has(cacheKey)) {
        cache.set(
          cacheKey,
          Promise.resolve().then(() =>
            discover(profile.url, {
              allowHttp: Boolean(profile.allow_insecure_http)
            })
          )
        );
      }
      try {
        return { ...target, favicon: await cache.get(cacheKey) };
      } catch (error) {
        return { ...target, error };
      }
    }
  );

  const warnings = [];
  const cleanupWarnings = [];
  for (const item of prepared) {
    const profile = item.profile;
    if (item.error) {
      warnings.push({
        profile: profile.id || profile.instance_id,
        message: item.error.message || String(item.error)
      });
      continue;
    }

    let temporary = null;
    try {
      temporary = writeTemporary(item.favicon);
      const updateResults = expectArray(
        await invoke(context, configPath, [
          "update",
          assertUuid(profile.instance_id),
          "--icon",
          temporary.filePath
        ]),
        "update favicon"
      );
      const updated = updateResults[0]?.profile;
      if (
        !updated ||
        assertUuid(updated.instance_id) !==
          assertUuid(profile.instance_id)
      ) {
        fail("el runtime no confirmo la importacion del favicon");
      }
      hydrated[item.index] = {
        ...item.result,
        profile: updated
      };
    } catch (error) {
      warnings.push({
        profile: profile.id || profile.instance_id,
        message: error.message || String(error)
      });
    } finally {
      if (temporary) {
        try {
          removeTemporary(temporary);
        } catch (error) {
          cleanupWarnings.push({
            profile: profile.id || profile.instance_id,
            message: error.message || String(error)
          });
        }
      }
    }
  }

  return { results: hydrated, warnings, cleanupWarnings };
}

function printFaviconWarnings(warnings, fallback = "default") {
  const fallbackMessage =
    fallback === "previous"
      ? "Se conserva el icono anterior."
      : "La app usara el icono predeterminado.";
  for (const warning of warnings) {
    console.error(
      "misku-nv: favicon no disponible para " +
        warning.profile +
        ": " +
        warning.message +
        ". " +
        fallbackMessage
    );
  }
}

function printTemporaryCleanupWarnings(warnings) {
  for (const warning of warnings) {
    console.error(
      "misku-nv: no se pudo limpiar el favicon temporal de " +
        warning.profile +
        ": " +
        warning.message
    );
  }
}

function failPureFaviconRefreshIfNeeded(refreshOnly, warnings) {
  if (!refreshOnly || warnings.length === 0) {
    return;
  }
  fail(
    "no se pudo refrescar el favicon; se conservo el icono anterior"
  );
}
function sanitizeFilePart(value, fallback, maximumLength) {
  const normalized = String(value || "")
    .normalize("NFKC")
    .replace(/[\u0000-\u001f\u007f<>:"/\\|?*\p{Cf}]/gu, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[. ]+$/g, "")
    .slice(0, maximumLength)
    .replace(/[. ]+$/g, "");
  return normalized || fallback;
}

function sanitizeShortcutName(name, id, instanceId) {
  const base = sanitizeFilePart(name || id, "App", 80);
  const safeId = sanitizeFilePart(id, "app", 32);
  return `${base}-${safeId}-${assertUuid(instanceId).slice(0, 8)}`;
}

function sameShortcutName(left, right) {
  return (
    String(left).normalize("NFKC").toUpperCase() ===
    String(right).normalize("NFKC").toUpperCase()
  );
}

function assertSafeShortcutName(value) {
  const name = String(value);
  if (
    !name ||
    name.length > 180 ||
    name === "." ||
    name === ".." ||
    /[\u0000-\u001f<>:"/\\|?*]/.test(name) ||
    /[. ]$/.test(name) ||
    WINDOWS_RESERVED_NAME.test(name) ||
    path.basename(name) !== name
  ) {
    fail(`nombre de acceso directo invalido: ${name}`);
  }
  return name;
}

function quoteWindowsArgument(value) {
  const text = String(value);
  if (text.includes("\u0000")) {
    fail("un argumento de Windows no puede contener NUL");
  }
  return `"${text
    .replace(/(\\*)"/g, "$1$1\\\"")
    .replace(/(\\+)$/g, "$1$1")}"`;
}

function runShortcutScript(context, mode, options) {
  if (!fs.existsSync(shortcutScript)) {
    fail(`no encontre el administrador de accesos directos: ${shortcutScript}`);
  }
  assertSafeShortcutName(options.shortcutName);
  const args = [
    "-NoLogo",
    "-NoProfile",
    "-NonInteractive",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    shortcutScript,
    "-Mode",
    mode,
    "-ProgramsDir",
    context.programsDir,
    "-ShortcutName",
    options.shortcutName
  ];
  if (mode === "Install") {
    args.push(
      "-TargetPath",
      options.targetPath,
      "-TargetArguments",
      options.targetArguments,
      "-WorkingDirectory",
      options.workingDirectory,
      "-IconPath",
      options.iconPath
    );
  }
  const result = spawnSync("powershell.exe", args, {
    encoding: "utf8",
    stdio: "pipe",
    windowsHide: true,
    maxBuffer: 1024 * 1024,
    shell: false
  });
  if (result.error) {
    fail(result.error.message);
  }
  if ((result.status ?? 1) !== 0) {
    fail((result.stderr || result.stdout || "fallo PowerShell").trim());
  }

  return result.stdout.trim();
}

function readJsonIfExists(filePath) {
  if (!fs.existsSync(filePath)) {
    return null;
  }
  const stats = assertRegularFile(filePath, "los metadatos de instalacion");
  if (stats.size > MAX_JSON_BYTES) {
    fail(`los metadatos de instalacion son demasiado grandes: ${filePath}`);
  }
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    fail(`los metadatos de instalacion son invalidos (${filePath}): ${error.message}`);
  }
}

function validateInstallMetadata(metadata, instanceId) {
  if (metadata === null) {
    return null;
  }
  if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) {
    fail("los metadatos de instalacion no son un objeto");
  }
  if (assertUuid(metadata.instanceId) !== instanceId) {
    fail(`los metadatos no pertenecen a la app ${instanceId}`);
  }
  if (metadata.shortcutName) {
    assertSafeShortcutName(metadata.shortcutName);
    if (!metadata.shortcutName.toLowerCase().endsWith(`-${instanceId.slice(0, 8)}`)) {
      fail(`el acceso directo registrado no pertenece a la app ${instanceId}`);
    }
  }
  return metadata;
}

function profileFingerprint(profile) {
  return crypto.createHash("sha256").update(JSON.stringify(profile)).digest("hex");
}

// Opening an unchanged app must not export files or launch PowerShell. Treat the
// installation record as a cache: anything missing, stale or unsafe rebuilds it.
function readMaterializedProfile(context, configPath, profile) {
  const instanceId = assertUuid(profile.instance_id);
  const appDir = safeJoin(context.appsDir, instanceId);
  try {
    for (const directory of [context.home, context.appsDir, appDir]) {
      if (!assertDirectoryNoLinksIfExists(directory, "instalacion")) return null;
    }
    const metadata = validateInstallMetadata(
      readJsonIfExists(safeJoin(appDir, "install.json")), instanceId
    );
    if (!metadata || metadata.sourceConfig !== path.resolve(configPath) ||
        metadata.sourceFingerprint !== profileFingerprint(profile) ||
        metadata.runtimeVersion !== packageJson.version) return null;
    const manifest = safeJoin(appDir, "app.toml");
    const shortcut = safeJoin(context.programsDir, `${metadata.shortcutName}.lnk`);
    if (metadata.manifest !== manifest || metadata.shortcutPath !== shortcut) return null;
    const runtimeDir = path.dirname(metadata.runtime || "");
    if (path.dirname(runtimeDir) !== path.resolve(context.runtimesDir) ||
        !path.basename(runtimeDir).startsWith(`${packageJson.version}-`) ||
        !/^[0-9a-f]{12}$/.test(path.basename(runtimeDir).split("-").at(-1)) ||
        path.basename(metadata.runtime) !== "misku-native-views.exe") return null;
    for (const directory of [context.runtimesDir, runtimeDir, context.programsDir]) {
      if (!assertDirectoryNoLinksIfExists(directory, "instalacion")) return null;
    }
    for (const file of [manifest, metadata.runtime, shortcut]) assertRegularFile(file, "instalacion");
    if (metadata.manifestHash !== fileSha256(manifest)) return null;
    if (metadata.iconPath && metadata.iconPath !== metadata.runtime) {
      const iconDir = safeJoin(appDir, "icons");
      if (path.dirname(metadata.iconPath) !== iconDir ||
          !assertDirectoryNoLinksIfExists(iconDir, "iconos")) return null;
      assertRegularFile(metadata.iconPath, "icono");
    }
    return metadata;
  } catch {
    return null;
  }
}

function ensureMaterializedProfile(context, configPath, profile) {
  return readMaterializedProfile(context, configPath, profile) ||
    materializeProfile(context, configPath, { profile });
}

function nativeShortcutName(context, appDir, instanceId) {
  const record = readJsonIfExists(safeJoin(appDir, 'native-install.json'));
  if (!record || record.instance_id !== instanceId || typeof record.shortcut !== 'string') return null;
  const shortcut = path.resolve(record.shortcut.replace(/^\\\\\?\\/, ''));
  if (path.dirname(shortcut).toUpperCase() !== path.resolve(context.programsDir).toUpperCase()) fail('Acceso directo nativo fuera del directorio administrado');
  const name = path.basename(shortcut, '.lnk');
  assertSafeShortcutName(name);
  if (!name.toLowerCase().endsWith(`-${instanceId.slice(0, 8)}`)) fail('Acceso directo nativo ajeno a esta app');
  return name;
}

function materializeProfile(context, configPath, operationResult) {
  const sourceProfile = operationResult.profile || operationResult;
  const instanceId = assertUuid(sourceProfile.instance_id);
  ensureDirectoryNoLinks(context.home, "directorio principal administrado");
  ensureDirectoryNoLinks(context.appsDir, "directorio de apps");
  const appDir = safeJoin(context.appsDir, instanceId);
  ensureDirectoryNoLinks(appDir, "directorio de la app");
  const manifest = safeJoin(appDir, "app.toml");

  const exportResults = expectArray(
    invokeJson(context, configPath, [
      "export",
      instanceId,
      "--output",
      manifest
    ]),
    "export"
  );
  const exported = exportResults[0]?.profile;
  if (
    !exported ||
    assertUuid(exported.instance_id) !== instanceId
  ) {
    fail(`no se pudo exportar el manifiesto independiente de ${sourceProfile.id}`);
  }

  const runtime = ensureRuntime(context);
  const shortcutName = sanitizeShortcutName(
    exported.name,
    exported.id,
    instanceId
  );
  const installMetadataPath = safeJoin(appDir, "install.json");
  let previous = null;
  try { previous = validateInstallMetadata(readJsonIfExists(installMetadataPath), instanceId); }
  catch { /* A corrupt cache is rebuilt; never follow paths from it. */ }

  let iconPath = runtime;
  if (exported.icon) {
    const iconParts = String(exported.icon).split(/[\\/]/).filter(Boolean);
    const candidate = safeJoin(appDir, ...iconParts);
    if (
      fs.existsSync(candidate) &&
      path.extname(candidate).toLowerCase() === ".ico"
    ) {
      assertRegularFile(candidate, "el icono exportado");
      iconPath = candidate;
    }
  }
  const targetArguments = [
    "--config",
    quoteWindowsArgument(manifest),
    "--app",
    quoteWindowsArgument(instanceId)
  ].join(" ");

  runShortcutScript(context, "Install", {
    shortcutName,
    targetPath: runtime,
    targetArguments,
    workingDirectory: appDir,
    iconPath
  });

  if (
    previous?.shortcutName &&
    !sameShortcutName(previous.shortcutName, shortcutName)
  ) {
    runShortcutScript(context, "Remove", {
      shortcutName: previous.shortcutName
    });
  }

  const metadata = {
    schemaVersion: 1,
    instanceId,
    id: exported.id,
    name: exported.name,
    manifest,
    runtime,
    shortcutName,
    shortcutPath: safeJoin(context.programsDir, `${shortcutName}.lnk`),
    sourceConfig: path.resolve(configPath),
    sourceFingerprint: profileFingerprint(sourceProfile),
    runtimeVersion: packageJson.version,
    manifestHash: fileSha256(manifest),
    iconPath
  };
  writeJsonAtomic(installMetadataPath, metadata);
  const nativeName = nativeShortcutName(context, appDir, instanceId);
  if (nativeName && !sameShortcutName(nativeName, shortcutName)) {
    runShortcutScript(context, 'Remove', { shortcutName: nativeName });
  }
  return metadata;
}

function removeMaterializedProfile(context, profile) {
  const instanceId = assertUuid(profile.instance_id);
  let metadata = null;
  const appDir = safeJoin(context.appsDir, instanceId);

  if (assertDirectoryNoLinksIfExists(context.appsDir, "directorio de apps")) {
    if (fs.existsSync(appDir)) {
      assertDirectoryNoLinksIfExists(appDir, "directorio de la app");
      metadata = validateInstallMetadata(
        readJsonIfExists(safeJoin(appDir, "install.json")),
        instanceId
      );
    }
  }

  const shortcutName =
    metadata?.shortcutName ||
    (profile.id && profile.name
      ? sanitizeShortcutName(profile.name, profile.id, instanceId)
      : null);
  if (shortcutName) {
    runShortcutScript(context, "Remove", { shortcutName });
  }

  const nativeName = fs.existsSync(appDir) && nativeShortcutName(context, appDir, instanceId);
  if (nativeName && !sameShortcutName(nativeName, shortcutName)) runShortcutScript(context, 'Remove', { shortcutName: nativeName });

  if (fs.existsSync(appDir)) {
    const relative = path.relative(path.resolve(context.appsDir), appDir);
    if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) {
      fail(`se rechazo borrar una ruta no administrada: ${appDir}`);
    }
    fs.rmSync(appDir, {
      recursive: true,
      force: false,
      maxRetries: 3,
      retryDelay: 100
    });
  }
}

function openInstalledProfile(metadata) {
  assertRegularFile(metadata.runtime, "el runtime instalado");
  assertRegularFile(metadata.manifest, "el manifiesto instalado");
  const child = spawn(
    metadata.runtime,
    ["--config", metadata.manifest, "--app", metadata.instanceId],
    {
      cwd: path.dirname(metadata.manifest),
      detached: true,
      stdio: "ignore",
      windowsHide: true,
      shell: false
    }
  );
  child.once("error", (error) => {
    console.error(`misku-nv: no se pudo abrir ${metadata.instanceId}: ${error.message}`);
  });
  child.unref();
}

function printOperations(results, forceJson = false) {
  if (forceJson) {
    console.log(JSON.stringify(results, null, 2));
    return;
  }
  for (const result of results) {
    const labels = {
      created: "creado",
      updated: "actualizado",
      removed: "eliminado",
      exported: "exportado"
    };
    const profile = result.profile;
    console.log(
      `${labels[result.action] || result.action}: ${profile.id} -> ${profile.url} (uuid: ${profile.instance_id})`
    );
  }
}

function selectorFromArgs(args) {
  if (args[0] === "run" || args[0] === "open") {
    return args.find((arg, index) => index > 0 && !arg.startsWith("-")) || null;
  }
  const appIndex = args.findIndex((arg) => arg === "--app" || arg === "-a");
  if (appIndex >= 0) {
    return args[appIndex + 1] || null;
  }
  if (args.length === 1 && !args[0].startsWith("-")) {
    return args[0];
  }
  return null;
}

function findProfile(profiles, selector) {
  const normalized = String(selector).toLowerCase();
  return profiles.find(
    (profile) =>
      String(profile.instance_id).toLowerCase() === normalized ||
      String(profile.id).toLowerCase() === normalized
  );
}

function isListCommand(args) {
  return args.includes("--list") || args.includes("-l") || args[0] === "list";
}

function stripWrapperOptions(args) {
  return args.filter(
    (arg) =>
      arg !== "--no-open" &&
      arg !== "--open" &&
      arg !== "--no-favicon" &&
      arg !== "--refresh-icon"
  );
}

function materializeOperations(context, configPath, results) {
  const installed = [];
  const failures = [];
  for (const result of results) {
    try {
      installed.push(materializeProfile(context, configPath, result));
    } catch (error) {
      const profile = result.profile || result;
      failures.push({
        profile: profile?.instance_id || profile?.id || "desconocida",
        message: error.message || String(error)
      });
    }
  }
  return { installed, failures };
}

function failMaterializationIfNeeded(outcome) {
  if (outcome.failures.length === 0) {
    return;
  }
  const details = outcome.failures
    .map((item) => `- ${item.profile}: ${item.message}`)
    .join("\n");
  fail(
    `el registro se actualizo, pero ${outcome.failures.length} app(s) no quedaron materializadas.\n` +
      `${details}\nEjecuta misku-nv repair para completar la instalacion.`
  );
}

async function finishCreation(context, configPath, results, options, services = {}) {
  const materialize = services.materialize || materializeOperations;
  const open = services.open || openInstalledProfile;
  const hydrate = services.hydrate || hydrateProfileFavicons;
  let outcome;
  if (options.open) {
    outcome = materialize(context, configPath, results);
    outcome.installed.forEach(open);
    failMaterializationIfNeeded(outcome);
  }
  let faviconWarnings = [];
  let cleanupWarnings = [];
  if (options.favicon) {
    // The process remains alive to complete the bounded job; the window is
    // already usable. --no-open still waits for a fully materialized result.
    const hydrated = await hydrate(context, configPath, results);
    results = hydrated.results;
    faviconWarnings = hydrated.warnings;
    cleanupWarnings = hydrated.cleanupWarnings;
  }
  if (!options.open || options.favicon) {
    outcome = materialize(context, configPath, results);
    failMaterializationIfNeeded(outcome);
  }
  return { results, faviconWarnings, cleanupWarnings };
}

function repairInstallation(context, configPath, profiles) {
  const outcome = materializeOperations(
    context,
    configPath,
    profiles.map((profile) => ({ profile }))
  );
  const active = new Set(
    profiles.map((profile) => assertUuid(profile.instance_id))
  );
  let removed = 0;

  if (assertDirectoryNoLinksIfExists(context.appsDir, "directorio de apps")) {
    for (const entry of fs.readdirSync(context.appsDir, { withFileTypes: true })) {
      if (!UUID_PATTERN.test(entry.name) || active.has(entry.name.toLowerCase())) {
        continue;
      }
      try {
        removeMaterializedProfile(context, { instance_id: entry.name });
        removed += 1;
      } catch (error) {
        outcome.failures.push({
          profile: entry.name,
          message: `no se pudo limpiar una instalacion huerfana: ${error.message}`
        });
      }
    }
  }

  return { ...outcome, removed };
}

async function main(rawArgs = process.argv.slice(2)) {
  if (process.platform !== "win32") {
    fail("esta version empaquetada solo soporta Windows.");
  }


  const invocation = extractGlobalOptions(rawArgs);
  const args = invocation.args;
  if (
    (args.length === 1 && ["--help", "-h", "help"].includes(args[0]))
  ) {
    printHelp();
    return;
  }
  if (args.length === 1 && ["--version", "-V"].includes(args[0])) {
    console.log(`misku-nv ${packageJson.version}`);
    return;
  }

  const context = getContext();
  const configPath = invocation.configPath || context.registry;

  if (args.length === 0 || (args.length === 1 && ["manage", "--manager"].includes(args[0]))) {
    const child = spawn(context.nativeExe, withConfig(["--manager"], configPath), {
      detached: true, stdio: "ignore", windowsHide: true, shell: false
    });
    child.on("error", (error) => { console.error(`misku-nv: ${error.message}`); process.exitCode = 1; });
    child.unref();
    return;
  }

  if (
    args.length > 1 &&
    ["create", "add", "upsert", "update", "remove", "delete", "export"].includes(args[0]) &&
    args.slice(1).some((arg) => arg === "--help" || arg === "-h")
  ) {
    runNative(
      context.nativeExe,
      withConfig(args, configPath),
      { inherit: true }
    );
    return;
  }

  ensureRegistry(configPath);

  const friendly = parseFriendlyUrlCommand(args);
  if (friendly) {
    ensureRuntime(context);
    let results = expectArray(
      invokeJson(context, configPath, ["create", ...friendly.createArgs]),
      "create"
    );
    const creation = await finishCreation(context, configPath, results, {
      open: friendly.openAfterCreate, favicon: friendly.fetchFavicon
    });
    results = creation.results;
    printOperations(results, invocation.outputJson || friendly.outputJson);
    printFaviconWarnings(creation.faviconWarnings, "default");
    printTemporaryCleanupWarnings(creation.cleanupWarnings);
    return;
  }

  const command = args[0];
  if (["create", "add", "upsert"].includes(command)) {
    ensureRuntime(context);
    const shouldOpen = args.includes("--open") && !args.includes("--no-open");
    const faviconOptions = extractFaviconWrapperOptions(args, "create");
    const nativeArgs = normalizeIconArgs(
      stripWrapperOptions(faviconOptions.args)
    );
    nativeArgs[0] = "create";
    let results = expectArray(
      invokeJson(context, configPath, nativeArgs),
      "create"
    );
    const creation = await finishCreation(context, configPath, results, {
      open: shouldOpen, favicon: faviconOptions.autoFavicon
    });
    results = creation.results;
    printOperations(results, invocation.outputJson);
    printFaviconWarnings(creation.faviconWarnings, "default");
    printTemporaryCleanupWarnings(creation.cleanupWarnings);
    return;
  }

  if (command === "update") {
    ensureRuntime(context);
    const faviconOptions = extractFaviconWrapperOptions(args, "update");
    const nativeArgs = normalizeIconArgs(faviconOptions.args);
    const refreshOnly =
      faviconOptions.refreshFavicon && nativeArgs.length === 2;
    let results;
    if (refreshOnly) {
      const profiles = expectArray(
        invokeJson(context, configPath, ["--list"]),
        "list"
      );
      const profile = findProfile(profiles, nativeArgs[1]);
      if (!profile) {
        fail("no existe la app '" + nativeArgs[1] + "'. Usa misku-nv --list");
      }
      results = [{ action: "updated", profile }];
    } else {
      results = expectArray(
        invokeJson(context, configPath, nativeArgs),
        "update"
      );
    }

    let faviconWarnings = [];
    let faviconCleanupWarnings = [];
    if (faviconOptions.refreshFavicon) {
      const faviconOutcome = await hydrateProfileFavicons(
        context,
        configPath,
        results,
        { force: true }
      );
      results = faviconOutcome.results;
      faviconWarnings = faviconOutcome.warnings;
      faviconCleanupWarnings = faviconOutcome.cleanupWarnings;
    }

    if (refreshOnly && faviconWarnings.length > 0) {
      printFaviconWarnings(faviconWarnings, "previous");
      printTemporaryCleanupWarnings(faviconCleanupWarnings);
    }
    failPureFaviconRefreshIfNeeded(refreshOnly, faviconWarnings);

    const outcome = materializeOperations(context, configPath, results);
    printOperations(results, invocation.outputJson);
    printFaviconWarnings(faviconWarnings, "previous");
    printTemporaryCleanupWarnings(faviconCleanupWarnings);
    failMaterializationIfNeeded(outcome);
    if (faviconWarnings.length > 0) {
      fail(
        "la app se actualizo, pero no se pudo refrescar su favicon; " +
          "se conservo el icono anterior"
      );
    }
    return;
  }
  if (command === "remove" || command === "delete") {
    const results = expectArray(
      invokeJson(context, configPath, args),
      "remove"
    );
    const cleanupFailures = [];
    for (const result of results) {
      try {
        removeMaterializedProfile(context, result.profile);
      } catch (error) {
        cleanupFailures.push({
          profile: result.profile?.instance_id || result.profile?.id,
          message: error.message || String(error)
        });
      }
    }
    printOperations(results, invocation.outputJson);
    if (cleanupFailures.length > 0) {
      const details = cleanupFailures
        .map((item) => `- ${item.profile}: ${item.message}`)
        .join("\n");
      fail(
        `la app se elimino del registro, pero quedaron recursos por limpiar:\n${details}\n` +
          "Ejecuta misku-nv repair para reintentar la limpieza."
      );
    }
    return;
  }

  if (command === "repair") {
    ensureRuntime(context);
    const profiles = expectArray(
      invokeJson(context, configPath, ["--list"]),
      "list"
    );
    const outcome = repairInstallation(context, configPath, profiles);
    console.log(
      `reparadas: ${outcome.installed.length} apps; huerfanas eliminadas: ${outcome.removed}`
    );
    failMaterializationIfNeeded(outcome);
    return;
  }

  if (isListCommand(args)) {
    const profiles = expectArray(
      invokeJson(context, configPath, ["--list"]),
      "list"
    );
    if (invocation.outputJson) {
      console.log(JSON.stringify(profiles, null, 2));
    } else if (profiles.length === 0) {
      console.log("No hay apps configuradas.");
    } else {
      for (const profile of profiles) {
        console.log(
          `${profile.id}  ${profile.instance_id}  ${profile.name}  ${profile.url}`
        );
      }
    }
    return;
  }

  const selector = selectorFromArgs(args);
  if (selector) {
    const profile = invokeJson(context, configPath, ["inspect", selector]);
    if (!profile) {
      fail(`no existe la app '${selector}'. Usa misku-nv --list`);
    }
    const metadata = ensureMaterializedProfile(context, configPath, profile);
    openInstalledProfile(metadata);
    return;
  }

  const nativeArgs = invocation.outputJson ? ["--json", ...args] : args;
  runNative(
    context.nativeExe,
    withConfig(nativeArgs, configPath),
    { inherit: true }
  );
}

if (require.main === module) {
  main().catch((error) => {
    console.error(`misku-nv: ${error.message || error}`);
    process.exitCode = 1;
  });
}

module.exports = {
  finishCreation,
  profileFingerprint,
  readMaterializedProfile,
  UUID_PATTERN,
  assertUuid,
  extractGlobalOptions,
  extractFaviconWrapperOptions,
  findProfile,
  getContext,
  getExplicitConfig,
  failPureFaviconRefreshIfNeeded,
  hydrateProfileFavicons,
  looksLikeUrl,
  printFaviconWarnings,
  normalizeIconArgs,
  parseFriendlyUrlCommand,
  quoteWindowsArgument,
  safeJoin,
  sanitizeShortcutName,
  sameShortcutName,
  selectorFromArgs,
  withConfig,
  writeJsonAtomic
};
