#!/usr/bin/env node

const { spawn, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const packageRoot = path.resolve(__dirname, "..");
const portableDir = path.join(packageRoot, "portable");
const nativeExe = path.join(portableDir, "misku-native-views.exe");
const packageConfig = path.join(portableDir, "apps.toml");
const packageIcons = path.join(portableDir, "icons");

function fail(message) {
  console.error(`misku-nv: ${message}`);
  process.exit(1);
}

function printHelp() {
  console.log(`Misku Native Views CLI

Uso:
  misku-nv <url> [--name <nombre>] [--id <id>] [--icon <ruta>] [--no-open]
  misku-nv <id>
  misku-nv --list
  misku-nv add [opciones] <url> [url...]

Ejemplos:
  misku-nv https://github.com --name GitHub
  misku-nv github
  misku-nv --list

Opciones rapidas:
  -n, --name <nombre>  Nombre visible de la app.
  -i, --id <id>        Id del perfil. Si ya existe, se actualiza.
  --icon <ruta>        Icono .ico o .png relativo a apps.toml.
  --no-open            Crea o actualiza el perfil sin abrirlo.
  -c, --config <ruta>  Archivo apps.toml alternativo.`);
}

function getExplicitConfigArgs(args) {
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];

    if (arg === "--config" || arg === "-c") {
      const value = args[index + 1];
      return value ? ["--config", value] : null;
    }

    if (arg.startsWith("--config=")) {
      return ["--config", arg.slice("--config=".length)];
    }
  }

  return null;
}

function hasExplicitConfig(args) {
  return getExplicitConfigArgs(args) !== null;
}

function getUserConfigDir() {
  const appData = process.env.APPDATA || path.join(os.homedir(), "AppData", "Roaming");
  return path.join(appData, "Misku Native Views");
}

function copyDirectoryIfMissing(source, destination) {
  if (!fs.existsSync(source) || fs.existsSync(destination)) {
    return;
  }

  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.cpSync(source, destination, {
    recursive: true,
    errorOnExist: false,
    force: false
  });
}

function ensureUserConfig() {
  const userConfigDir = getUserConfigDir();
  const userConfig = path.join(userConfigDir, "apps.toml");
  fs.mkdirSync(userConfigDir, { recursive: true });

  if (!fs.existsSync(userConfig)) {
    if (!fs.existsSync(packageConfig)) {
      fail(`no encontre la configuracion incluida en ${packageConfig}`);
    }
    fs.copyFileSync(packageConfig, userConfig);
  }

  copyDirectoryIfMissing(packageIcons, path.join(userConfigDir, "icons"));

  return userConfig;
}

function withDefaultConfig(args, rawArgs) {
  if (hasExplicitConfig(args)) {
    return args;
  }

  const explicitConfigArgs = getExplicitConfigArgs(rawArgs);
  if (explicitConfigArgs) {
    return [...explicitConfigArgs, ...args];
  }

  const cwdConfig = path.join(process.cwd(), "apps.toml");
  if (fs.existsSync(cwdConfig)) {
    return args;
  }

  return ["--config", ensureUserConfig(), ...args];
}

function looksLikeUrl(value) {
  return /^https?:\/\//i.test(value)
    || /^localhost(?::\d+)?(?:\/|$)/i.test(value)
    || /^[^\s.]+\.[^\s]+/.test(value);
}

function pushOptionWithValue(target, args, index, canonicalName) {
  const value = args[index + 1];
  if (!value) {
    fail(`falta el valor despues de ${args[index]}`);
  }
  target.push(canonicalName || args[index], value);
  return index + 1;
}

function parseFriendlyUrlCommand(rawArgs) {
  let url = null;
  let openAfterAdd = true;
  const addArgs = [];

  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];

    if (arg === "--no-open") {
      openAfterAdd = false;
      continue;
    }

    if (arg === "--open") {
      openAfterAdd = true;
      continue;
    }

    if (arg === "--name" || arg === "-n") {
      index = pushOptionWithValue(addArgs, rawArgs, index, arg);
      continue;
    }

    if (arg === "--id" || arg === "-i") {
      index = pushOptionWithValue(addArgs, rawArgs, index, arg);
      continue;
    }

    if (arg === "--icon") {
      index = pushOptionWithValue(addArgs, rawArgs, index, arg);
      continue;
    }

    if (arg === "--config" || arg === "-c") {
      index = pushOptionWithValue(addArgs, rawArgs, index, arg);
      continue;
    }

    if (arg.startsWith("--name=")) {
      addArgs.push("--name", arg.slice("--name=".length));
      continue;
    }

    if (arg.startsWith("--id=")) {
      addArgs.push("--id", arg.slice("--id=".length));
      continue;
    }

    if (arg.startsWith("--icon=")) {
      addArgs.push("--icon", arg.slice("--icon=".length));
      continue;
    }

    if (arg.startsWith("--config=")) {
      addArgs.push("--config", arg.slice("--config=".length));
      continue;
    }

    if (!arg.startsWith("-") && !url && looksLikeUrl(arg)) {
      url = arg;
      addArgs.push(arg);
      continue;
    }

    return null;
  }

  if (!url) {
    return null;
  }

  return { addArgs, openAfterAdd };
}

function runNative(args, options = {}) {
  const result = spawnSync(nativeExe, args, {
    cwd: process.cwd(),
    encoding: "utf8",
    stdio: options.capture ? "pipe" : "inherit",
    windowsHide: false
  });

  if (result.error) {
    fail(result.error.message);
  }

  if (result.signal) {
    fail(`proceso terminado por ${result.signal}`);
  }

  return result;
}

function openNative(args) {
  const child = spawn(nativeExe, args, {
    cwd: process.cwd(),
    detached: true,
    stdio: "ignore",
    windowsHide: false
  });

  child.unref();
}

function parseUpsertedId(output) {
  const match = output.match(/^(?:creado|actualizado):\s+([^\s]+)\s+->/m);
  return match ? match[1] : null;
}

if (process.platform !== "win32") {
  fail("esta version empaquetada solo soporta Windows.");
}

if (!fs.existsSync(nativeExe)) {
  fail(`no encontre el binario nativo en ${nativeExe}. Ejecuta "pnpm run build:package" antes de empaquetar.`);
}

const rawArgs = process.argv.slice(2);

if (rawArgs.length === 0 || (rawArgs.length === 1 && (rawArgs[0] === "--help" || rawArgs[0] === "-h"))) {
  printHelp();
  process.exit(0);
}

const friendlyCommand = parseFriendlyUrlCommand(rawArgs);
if (friendlyCommand) {
  const addArgs = withDefaultConfig(["add", ...friendlyCommand.addArgs], rawArgs);
  const addResult = runNative(addArgs, { capture: friendlyCommand.openAfterAdd });

  if (friendlyCommand.openAfterAdd) {
    if (addResult.stdout) {
      process.stdout.write(addResult.stdout);
    }
    if (addResult.stderr) {
      process.stderr.write(addResult.stderr);
    }
  }

  if (addResult.status !== 0) {
    process.exit(addResult.status ?? 1);
  }

  if (!friendlyCommand.openAfterAdd) {
    process.exit(0);
  }

  const appId = parseUpsertedId(addResult.stdout || "");
  if (!appId) {
    fail("no pude detectar el id del perfil creado.");
  }

  openNative(withDefaultConfig(["--app", appId], rawArgs));
  process.exit(0);
}

const args = withDefaultConfig(rawArgs, rawArgs);
const result = runNative(args);
process.exit(result.status ?? 1);


