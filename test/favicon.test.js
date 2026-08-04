"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const test = require("node:test");

const {
  MAX_ICON_BYTES,
  classifyAddress,
  detectFaviconImage,
  discoverFavicon,
  downloadResource,
  extractFaviconLinks,
  extractManifestIcons,
  isValidIco,
  isValidPng,
  normalizePageUrl
} = require("../bin/favicon.js");
const {
  extractFaviconWrapperOptions,
  failPureFaviconRefreshIfNeeded,
  hydrateProfileFavicons,
  printFaviconWarnings
} = require("../bin/misku.js");

const FIRST_UUID = "18c70bd8-7a9c-4ded-8bb2-d680b2022c89";
const SECOND_UUID = "03da2d54-d706-4fc3-8435-faa55b635bd2";
const REPO_ICON = fs.readFileSync(
  path.resolve(__dirname, "..", "src-tauri", "icons", "icon.ico")
);

function createHeaderOnlyPng(width = 32, height = 32) {
  const result = Buffer.alloc(33);
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]).copy(result, 0);
  result.writeUInt32BE(13, 8);
  result.write("IHDR", 12, "ascii");
  result.writeUInt32BE(width, 16);
  result.writeUInt32BE(height, 20);
  result[24] = 8;
  result[25] = 6;
  return result;
}

async function listen(server) {
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  return "http://127.0.0.1:" + address.port;
}

async function close(server) {
  await new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}

test("extrae favicon, apple icon y manifest resolviendo rutas relativas", () => {
  const page = new URL("https://example.com/product/page");
  const extracted = extractFaviconLinks(
    [
      '<base href="/assets/">',
      '<link rel="apple-touch-icon" href="touch.png" type="image/png">',
      '<link href="/favicon.ico" rel="shortcut icon">',
      '<link rel="manifest" href="/site.webmanifest">'
    ].join(""),
    page
  );

  assert.deepEqual(
    extracted.icons.map((url) => url.href),
    [
      "https://example.com/favicon.ico",
      "https://example.com/assets/touch.png"
    ]
  );
  assert.deepEqual(
    extracted.manifests.map((url) => url.href),
    ["https://example.com/site.webmanifest"]
  );

  assert.doesNotThrow(() =>
    extractFaviconLinks(
      '<link rel="icon" href="/&#x110000;.ico">',
      page
    )
  );

  assert.deepEqual(
    extractManifestIcons(
      JSON.stringify({
        icons: [
          { src: "icon.png", type: "image/png" },
          { src: "/app.ico", type: "image/x-icon" }
        ]
      }),
      new URL("https://example.com/site.webmanifest")
    ).map((url) => url.href),
    [
      "https://example.com/app.ico",
      "https://example.com/icon.png"
    ]
  );
});

test("valida firmas PNG/ICO y rechaza contenido HTML o truncado", () => {
  const png = createHeaderOnlyPng();
  assert.equal(isValidPng(png), true);
  assert.equal(detectFaviconImage(png, "image/png"), "png");
  assert.equal(isValidIco(REPO_ICON), true);
  assert.equal(detectFaviconImage(REPO_ICON, "image/x-icon"), "ico");

  assert.equal(isValidPng(png.subarray(0, 20)), false);
  assert.equal(isValidIco(REPO_ICON.subarray(0, 20)), false);
  assert.throws(
    () => detectFaviconImage(REPO_ICON, "text/html; charset=utf-8"),
    /devolvio HTML/
  );
});

test("descubre un favicon relativo y el fallback usando solo loopback", async () => {
  const server = http.createServer((request, response) => {
    if (request.url === "/with-link") {
      response.setHeader("Content-Type", "text/html");
      response.end('<link rel="icon" href="/assets/site.ico">');
      return;
    }
    if (request.url === "/assets/site.ico" || request.url === "/favicon.ico") {
      response.setHeader("Content-Type", "image/x-icon");
      response.end(REPO_ICON);
      return;
    }
    if (request.url === "/fallback") {
      response.setHeader("Content-Type", "text/html");
      response.end("<title>Fallback</title>");
      return;
    }
    response.statusCode = 404;
    response.end();
  });
  const origin = await listen(server);

  try {
    const linked = await discoverFavicon(origin + "/with-link", {
      allowHttp: true,
      totalTimeoutMs: 2_000
    });
    assert.equal(linked.extension, "ico");
    assert.equal(linked.sourceUrl, origin + "/assets/site.ico");
    assert.deepEqual(linked.bytes, REPO_ICON);

    const fallback = await discoverFavicon(origin + "/fallback", {
      allowHttp: true,
      totalTimeoutMs: 2_000
    });
    assert.equal(fallback.sourceUrl, origin + "/favicon.ico");

    await assert.rejects(
      discoverFavicon(origin + "/with-link", {
        allowHttp: false,
        totalTimeoutMs: 500
      }),
      /solo admite HTTPS/
    );
  } finally {
    await close(server);
  }
});

test("limita bytes y bloquea redirects HTTP fuera de loopback", async () => {
  const server = http.createServer((request, response) => {
    if (request.url === "/oversize") {
      response.statusCode = 200;
      response.setHeader("Content-Length", String(MAX_ICON_BYTES + 1));
      response.end();
      return;
    }
    response.statusCode = 302;
    response.setHeader("Location", "http://169.254.169.254/latest/meta-data/");
    response.end();
  });
  const origin = await listen(server);

  try {
    await assert.rejects(
      downloadResource(origin + "/oversize", {
        allowHttp: true,
        maxBytes: MAX_ICON_BYTES,
        timeoutMs: 1_000
      }),
      /demasiado grande/
    );
    await assert.rejects(
      downloadResource(origin + "/redirect-private", {
        allowHttp: true,
        timeoutMs: 1_000
      }),
      /solo admite HTTPS/
    );
  } finally {
    await close(server);
  }

  await assert.rejects(
    downloadResource("http://localhost/favicon.ico", {
      allowHttp: true,
      timeoutMs: 100,
      resolveHost: async () => [
        { address: "::ffff:c0a8:1", family: 6 }
      ]
    }),
    /cambio de red|HTTP solo puede resolver a una direccion loopback/
  );
  await assert.rejects(
    downloadResource("https://private.example/favicon.ico", {
      timeoutMs: 100,
      resolveHost: async () => [
        { address: "192.168.1.10", family: 4 }
      ]
    }),
    /cambio de red|destino privado/
  );

  assert.equal(classifyAddress("127.0.0.1"), "loopback");
  assert.equal(classifyAddress("10.0.0.1"), "private");
  assert.equal(classifyAddress("169.254.169.254"), "blocked");
  assert.equal(classifyAddress("8.8.8.8"), "public");
  assert.equal(classifyAddress("::ffff:7f00:1"), "loopback");
  assert.equal(classifyAddress("::ffff:c0a8:1"), "private");
  assert.equal(classifyAddress("::ffff:a9fe:a9fe"), "blocked");
  assert.equal(classifyAddress("::ffff:127.0.0.1"), "loopback");
  assert.equal(classifyAddress("198.18.0.1"), "blocked");
  assert.equal(classifyAddress("192.0.2.10"), "blocked");
  assert.equal(classifyAddress("64:ff9b::c0a8:101"), "private");
  assert.equal(classifyAddress("2002:c0a8:101::"), "blocked");
  assert.equal(classifyAddress("2001:4860::5efe:c0a8:101"), "private");
  assert.equal(classifyAddress("2001:4860::5efe:808:808"), "public");
  assert.equal(classifyAddress("fd00::5efe:808:808"), "private");
  assert.equal(classifyAddress("fec0::5efe:808:808"), "private");
  assert.equal(classifyAddress("fe80::5efe:808:808"), "blocked");
  assert.equal(classifyAddress("2001:db8::5efe:808:808"), "blocked");
  assert.equal(classifyAddress("fec0::1"), "private");
  assert.equal(classifyAddress("2001:db8::1"), "blocked");
  assert.equal(classifyAddress("3fff::1"), "blocked");
  assert.equal(classifyAddress("5f00::1"), "blocked");
  assert.equal(classifyAddress("2606:4700:4700::1111"), "public");
  assert.throws(
    () => normalizePageUrl("http://example.com", true),
    /solo admite HTTPS/
  );
});

test("hidrata URLs duplicadas una sola vez pero importa un ICO por UUID", async () => {
  let discoveries = 0;
  const imported = [];
  const results = [
    {
      action: "created",
      profile: {
        instance_id: FIRST_UUID,
        id: "same",
        url: "https://example.com/repeated",
        allow_insecure_http: false,
        icon: null
      }
    },
    {
      action: "created",
      profile: {
        instance_id: SECOND_UUID,
        id: "same-2",
        url: "https://example.com/repeated",
        allow_insecure_http: false,
        icon: null
      }
    }
  ];

  const outcome = await hydrateProfileFavicons(
    {},
    "C:\\managed\\apps.toml",
    results,
    {
      discoverFavicon: async () => {
        discoveries += 1;
        return {
          bytes: REPO_ICON,
          extension: "ico",
          sourceUrl: "https://example.com/favicon.ico"
        };
      },
      invokeJson: async (_context, _config, args) => {
        const instanceId = args[1];
        const temporary = args[3];
        imported.push({
          instanceId,
          temporary,
          bytes: fs.readFileSync(temporary)
        });
        const original = results.find(
          (result) => result.profile.instance_id === instanceId
        ).profile;
        return [
          {
            action: "updated",
            profile: {
              ...original,
              icon: "icons/" + instanceId + ".ico"
            }
          }
        ];
      }
    }
  );

  assert.equal(discoveries, 1);
  assert.equal(imported.length, 2);
  assert.notEqual(imported[0].temporary, imported[1].temporary);
  assert.deepEqual(imported[0].bytes, REPO_ICON);
  assert.deepEqual(imported[1].bytes, REPO_ICON);
  assert.equal(fs.existsSync(imported[0].temporary), false);
  assert.equal(fs.existsSync(imported[1].temporary), false);
  assert.deepEqual(
    outcome.results.map((result) => result.profile.icon),
    [
      "icons/" + FIRST_UUID + ".ico",
      "icons/" + SECOND_UUID + ".ico"
    ]
  );
  assert.deepEqual(outcome.warnings, []);
});

test("--icon evita red y --refresh-icon no se combina con iconos explicitos", async () => {
  const parsed = extractFaviconWrapperOptions(
    ["create", "https://example.com", "--no-favicon"],
    "create"
  );
  assert.equal(parsed.autoFavicon, false);
  assert.deepEqual(parsed.args, ["create", "https://example.com"]);

  assert.throws(
    () =>
      extractFaviconWrapperOptions(
        ["update", FIRST_UUID, "--refresh-icon", "--clear-icon"],
        "update"
      ),
    /no se puede combinar/
  );

  const result = {
    action: "created",
    profile: {
      instance_id: FIRST_UUID,
      id: "manual",
      url: "https://example.com",
      icon: "icons/manual.ico"
    }
  };
  const outcome = await hydrateProfileFavicons({}, "apps.toml", [result], {
    discoverFavicon: async () => {
      throw new Error("no debio consultar la red");
    },
    invokeJson: async () => {
      throw new Error("no debio importar un icono");
    }
  });
  assert.deepEqual(outcome.results, [result]);
  assert.deepEqual(outcome.warnings, []);
});

test("el timeout absoluto cubre DNS colgado y respuestas que siguen enviando datos", async () => {
  const dnsStartedAt = Date.now();
  await assert.rejects(
    downloadResource("https://timeout.example/favicon.ico", {
      timeoutMs: 40,
      resolveHost: () => new Promise(() => {})
    }),
    /timeout resolviendo DNS/
  );
  assert.ok(
    Date.now() - dnsStartedAt < 750,
    "una resolucion DNS colgada debe finalizar dentro del presupuesto"
  );

  let interval = null;
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { "Content-Type": "image/x-icon" });
    response.write(Buffer.from([0]));
    interval = setInterval(() => response.write(Buffer.from([0])), 10);
    response.once("close", () => clearInterval(interval));
  });
  const origin = await listen(server);
  const requestStartedAt = Date.now();

  try {
    await assert.rejects(
      downloadResource(origin + "/slow", {
        allowHttp: true,
        maxBytes: MAX_ICON_BYTES,
        timeoutMs: 80
      }),
      /timeout descargando/
    );
    assert.ok(
      Date.now() - requestStartedAt < 750,
      "una respuesta activa no debe reiniciar el timeout absoluto"
    );
  } finally {
    if (interval) {
      clearInterval(interval);
    }
    await close(server);
  }
});

test("prioriza el icono HTML antes del manifest y cierra redirects sin cuerpo finito", async () => {
  let manifestRequests = 0;
  let redirectClosed = false;
  let redirectInterval = null;
  const server = http.createServer((request, response) => {
    if (request.url === "/page") {
      response.setHeader("Content-Type", "text/html");
      response.end(
        '<link rel="manifest" href="/site.webmanifest">' +
          '<link rel="icon" href="/html.ico">'
      );
      return;
    }
    if (request.url === "/site.webmanifest") {
      manifestRequests += 1;
      response.setHeader("Content-Type", "application/manifest+json");
      response.end(JSON.stringify({ icons: [{ src: "/manifest.ico" }] }));
      return;
    }
    if (
      request.url === "/html.ico" ||
      request.url === "/manifest.ico" ||
      request.url === "/redirect-target"
    ) {
      response.setHeader("Content-Type", "image/x-icon");
      response.end(REPO_ICON);
      return;
    }
    if (request.url === "/redirect-stream") {
      response.writeHead(302, { Location: "/redirect-target" });
      response.flushHeaders();
      redirectInterval = setInterval(() => response.write("x"), 10);
      response.once("close", () => {
        redirectClosed = true;
        clearInterval(redirectInterval);
      });
      return;
    }
    response.statusCode = 404;
    response.end();
  });
  const origin = await listen(server);

  try {
    const discovered = await discoverFavicon(origin + "/page", {
      allowHttp: true,
      totalTimeoutMs: 1_000
    });
    assert.equal(discovered.sourceUrl, origin + "/html.ico");
    assert.equal(manifestRequests, 0);

    const redirected = await downloadResource(origin + "/redirect-stream", {
      allowHttp: true,
      timeoutMs: 1_000
    });
    assert.deepEqual(redirected.body, REPO_ICON);
    for (let attempt = 0; attempt < 20 && !redirectClosed; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
    assert.equal(redirectClosed, true);
  } finally {
    if (redirectInterval) {
      clearInterval(redirectInterval);
    }
    await close(server);
  }
});

test("limita a cuatro las descargas de favicon concurrentes", async () => {
  const profiles = Array.from({ length: 9 }, (_value, index) => ({
    action: "created",
    profile: {
      instance_id: crypto.randomUUID(),
      id: "concurrent-" + index,
      url: "https://example.com/" + index,
      allow_insecure_http: false,
      icon: null
    }
  }));
  let active = 0;
  let maximumActive = 0;

  const outcome = await hydrateProfileFavicons({}, "apps.toml", profiles, {
    cleanupStale: false,
    discoverFavicon: async () => {
      active += 1;
      maximumActive = Math.max(maximumActive, active);
      await new Promise((resolve) => setTimeout(resolve, 25));
      active -= 1;
      return {
        bytes: REPO_ICON,
        extension: "ico",
        sourceUrl: "https://example.com/favicon.ico"
      };
    },
    invokeJson: async (_context, _config, args) => {
      const original = profiles.find(
        (result) => result.profile.instance_id === args[1]
      ).profile;
      return [
        {
          action: "updated",
          profile: {
            ...original,
            icon: "icons/" + original.instance_id + ".ico"
          }
        }
      ];
    }
  });

  assert.equal(maximumActive, 4);
  assert.equal(outcome.results.length, profiles.length);
  assert.deepEqual(outcome.warnings, []);
  assert.deepEqual(outcome.cleanupWarnings, []);
});

test("refresh conserva UUID y separa un fallo de limpieza del resultado importado", async () => {
  const previous = {
    action: "updated",
    profile: {
      instance_id: FIRST_UUID,
      id: "refresh",
      url: "https://example.com/refresh",
      allow_insecure_http: false,
      icon: "icons/previous.ico"
    }
  };
  const temporaryPath = "C:\\Temp\\misku-test-favicon.ico";

  const outcome = await hydrateProfileFavicons(
    {},
    "apps.toml",
    [previous],
    {
      force: true,
      cleanupStale: false,
      discoverFavicon: async () => ({
        bytes: REPO_ICON,
        extension: "ico",
        sourceUrl: "https://example.com/new.ico"
      }),
      writeTemporaryFavicon: () => ({ filePath: temporaryPath }),
      removeTemporaryFavicon: () => {
        throw new Error("limpieza simulada");
      },
      invokeJson: async (_context, _config, args) => {
        assert.equal(args[1], FIRST_UUID);
        assert.equal(args[3], temporaryPath);
        return [
          {
            action: "updated",
            profile: {
              ...previous.profile,
              icon: "icons/new.ico"
            }
          }
        ];
      }
    }
  );

  assert.equal(outcome.results[0].action, "updated");
  assert.equal(outcome.results[0].profile.instance_id, FIRST_UUID);
  assert.equal(outcome.results[0].profile.icon, "icons/new.ico");
  assert.deepEqual(outcome.warnings, []);
  assert.deepEqual(outcome.cleanupWarnings, [
    { profile: "refresh", message: "limpieza simulada" }
  ]);
});

test("refresh puro fallido aborta antes del exito y usa el mensaje de icono anterior", () => {
  const warnings = [{ profile: "refresh", message: "sin red" }];
  assert.throws(
    () => failPureFaviconRefreshIfNeeded(true, warnings),
    /no se pudo refrescar el favicon; se conservo el icono anterior/
  );
  assert.doesNotThrow(() =>
    failPureFaviconRefreshIfNeeded(false, warnings)
  );

  const messages = [];
  const originalError = console.error;
  console.error = (message) => messages.push(String(message));
  try {
    printFaviconWarnings(warnings, "default");
    printFaviconWarnings(warnings, "previous");
  } finally {
    console.error = originalError;
  }
  assert.match(messages[0], /usara el icono predeterminado/);
  assert.doesNotMatch(messages[0], /icono anterior/);
  assert.match(messages[1], /Se conserva el icono anterior/);
});
