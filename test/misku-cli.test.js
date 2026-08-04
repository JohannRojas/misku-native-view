"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  extractGlobalOptions,
  findProfile,
  getContext,
  getExplicitConfig,
  parseFriendlyUrlCommand,
  quoteWindowsArgument,
  safeJoin,
  sanitizeShortcutName,
  sameShortcutName,
  withConfig,
  writeJsonAtomic
} = require("../bin/misku.js");

const FIRST_UUID = "18c70bd8-7a9c-4ded-8bb2-d680b2022c89";
const SECOND_UUID = "03da2d54-d706-4fc3-8435-faa55b635bd2";

test("parsea varias URLs como creaciones independientes y conserva duplicados", () => {
  const parsed = parseFriendlyUrlCommand([
    "https://example.com/one",
    "https://example.com/two",
    "https://example.com/one",
    "--no-open",
    "--json"
  ]);

  assert.ok(parsed);
  assert.deepEqual(parsed.urls, [
    "https://example.com/one",
    "https://example.com/two",
    "https://example.com/one"
  ]);
  assert.deepEqual(parsed.createArgs, parsed.urls);
  assert.equal(parsed.openAfterCreate, false);
  assert.equal(parsed.outputJson, true);
});

test("extrae opciones globales antes o despues del comando", () => {
  const firstConfig = path.resolve("managed", "apps.toml");
  const before = extractGlobalOptions([
    "--json",
    "--config",
    firstConfig,
    "create",
    "https://example.com"
  ]);
  assert.deepEqual(before, {
    args: ["create", "https://example.com"],
    configPath: firstConfig,
    outputJson: true
  });

  const after = extractGlobalOptions([
    "update",
    FIRST_UUID,
    "--name",
    "Nueva",
    `--config=${firstConfig}`,
    "--json"
  ]);
  assert.deepEqual(after, {
    args: ["update", FIRST_UUID, "--name", "Nueva"],
    configPath: firstConfig,
    outputJson: true
  });

  assert.throws(
    () =>
      extractGlobalOptions([
        "--config",
        firstConfig,
        `--config=${path.resolve("other.toml")}`,
        "--list"
      ]),
    /solo se puede indicar una vez/
  );
});

test("safeJoin admite hijos y rechaza traversal fuera del directorio administrado", () => {
  const root = path.resolve("managed-home");

  assert.equal(
    safeJoin(root, "apps", FIRST_UUID, "app.toml"),
    path.join(root, "apps", FIRST_UUID, "app.toml")
  );
  assert.throws(
    () => safeJoin(root, "..", "outside", "apps.toml"),
    /ruta fuera del directorio administrado/
  );
  assert.throws(
    () => safeJoin(root, path.parse(root).root, "outside"),
    /ruta fuera del directorio administrado/
  );
});

test("sanitizeShortcutName limpia nombre e id y diferencia cada UUID", () => {
  const first = sanitizeShortcutName(
    '  Mi: "App" / trabajo...  ',
    "mi/app:*",
    FIRST_UUID
  );
  const second = sanitizeShortcutName(
    '  Mi: "App" / trabajo...  ',
    "mi/app:*",
    SECOND_UUID
  );

  assert.equal(first, "Mi App trabajo-mi app-18c70bd8");
  assert.equal(second, "Mi App trabajo-mi app-03da2d54");
  assert.doesNotMatch(first, /[\u0000-\u001f<>:"/\\|?*]/);
  assert.notEqual(first, second);
});

test("compara nombres de shortcut sin borrar el mismo archivo por casing", () => {
  assert.equal(
    sameShortcutName("Mi App-app-18c70bd8", "mi app-APP-18C70BD8"),
    true
  );
  assert.equal(
    sameShortcutName("Mi App-app-18c70bd8", "Otra App-app-18c70bd8"),
    false
  );
});

test("quoteWindowsArgument escapa comillas y barras finales para CreateShortcut", () => {
  assert.equal(
    quoteWindowsArgument("C:\\Program Files\\Misku\\app.toml"),
    '"C:\\Program Files\\Misku\\app.toml"'
  );
  assert.equal(
    quoteWindowsArgument('valor "entre comillas"'),
    '"valor \\"entre comillas\\""'
  );
  assert.equal(
    quoteWindowsArgument("C:\\Misku\\"),
    '"C:\\Misku\\\\"'
  );
  assert.throws(
    () => quoteWindowsArgument("valor\u0000invalido"),
    /no puede contener NUL/
  );
});

test("findProfile encuentra por alias o UUID sin depender de mayusculas", () => {
  const profiles = [
    { id: "Correo", instance_id: FIRST_UUID },
    { id: "Chat", instance_id: SECOND_UUID }
  ];

  assert.equal(findProfile(profiles, "correo"), profiles[0]);
  assert.equal(findProfile(profiles, SECOND_UUID.toUpperCase()), profiles[1]);
  assert.equal(findProfile(profiles, "inexistente"), undefined);
});

test("reutiliza el registro legado de APPDATA hasta que exista uno local", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "misku-cli-legacy-"));
  const localAppData = path.join(root, "local");
  const roamingAppData = path.join(root, "roaming");
  const legacyRegistry = path.join(
    roamingAppData,
    "Misku Native Views",
    "apps.toml"
  );
  const localRegistry = path.join(
    localAppData,
    "Misku Native Views",
    "apps.toml"
  );

  try {
    fs.mkdirSync(path.dirname(legacyRegistry), { recursive: true });
    fs.writeFileSync(legacyRegistry, "apps = []\n");
    let context = getContext({
      LOCALAPPDATA: localAppData,
      APPDATA: roamingAppData
    });
    assert.equal(context.registry, legacyRegistry);
    assert.equal(context.home, path.dirname(localRegistry));

    fs.mkdirSync(path.dirname(localRegistry), { recursive: true });
    fs.writeFileSync(localRegistry, "apps = []\n");
    context = getContext({
      LOCALAPPDATA: localAppData,
      APPDATA: roamingAppData
    });
    assert.equal(context.registry, localRegistry);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("la configuracion es explicita y nunca se deriva del directorio actual", () => {
  const managedHome = path.resolve("custom-managed-home");
  const context = getContext({
    MISKU_NV_HOME: managedHome,
    MISKU_NV_PROGRAMS_DIR: path.join(managedHome, "shortcuts"),
    MISKU_NV_NATIVE_EXE: path.join(managedHome, "runtime.exe")
  });
  const explicitConfig = path.resolve("another-home", "apps.toml");

  assert.equal(context.registry, path.join(managedHome, "apps.toml"));
  assert.equal(getExplicitConfig(["--list"]), null);
  assert.deepEqual(
    withConfig(["--list"], context.registry),
    ["--config", context.registry, "--list"]
  );
  assert.equal(
    getExplicitConfig(["--config", explicitConfig, "--list"]),
    explicitConfig
  );
  assert.deepEqual(
    withConfig(["--config", explicitConfig, "--list"], context.registry),
    ["--config", explicitConfig, "--list"]
  );
});

test("writeJsonAtomic reemplaza un destino existente sin dejar temporales", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "misku-cli-json-"));
  const destination = path.join(root, "install.json");

  try {
    writeJsonAtomic(destination, { version: 1, value: "anterior" });
    writeJsonAtomic(destination, { version: 2, value: "actual" });
    writeJsonAtomic(destination, { version: 2, value: "actual" });

    assert.deepEqual(
      JSON.parse(fs.readFileSync(destination, "utf8")),
      { version: 2, value: "actual" }
    );
    assert.deepEqual(fs.readdirSync(root), ["install.json"]);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
