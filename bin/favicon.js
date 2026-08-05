"use strict";

const dns = require("node:dns").promises;
const http = require("node:http");
const https = require("node:https");
const net = require("node:net");

const MAX_HTML_BYTES = 512 * 1024;
const MAX_MANIFEST_BYTES = 256 * 1024;
const MAX_ICON_BYTES = 2 * 1024 * 1024;
const MAX_CANDIDATES = 8;
const MAX_REDIRECTS = 3;
const REQUEST_TIMEOUT_MS = 4_000;
const TOTAL_TIMEOUT_MS = 10_000;
const USER_AGENT = "Misku-Native-Views-Favicon/0.2";

function isLoopbackHostname(hostname) {
  const normalized = String(hostname)
    .replace(/^\[|\]$/g, "")
    .toLowerCase();
  if (normalized === "localhost" || normalized.endsWith(".localhost")) {
    return true;
  }
  if (net.isIP(normalized) === 4) {
    return Number(normalized.split(".")[0]) === 127;
  }
  return normalized === "::1" || normalized === "0:0:0:0:0:0:0:1";
}

function parseIpv6Words(rawAddress) {
  let address = String(rawAddress).replace(/^\[|\]$/g, "").toLowerCase();
  if (address.includes(".")) {
    const separator = address.lastIndexOf(":");
    const ipv4 = separator >= 0 ? address.slice(separator + 1) : "";
    if (net.isIP(ipv4) !== 4) {
      throw new Error("direccion IPv6 invalida: " + rawAddress);
    }
    const octets = ipv4.split(".").map(Number);
    address =
      address.slice(0, separator) +
      ":" +
      ((octets[0] << 8) | octets[1]).toString(16) +
      ":" +
      ((octets[2] << 8) | octets[3]).toString(16);
  }

  const halves = address.split("::");
  if (halves.length > 2) {
    throw new Error("direccion IPv6 invalida: " + rawAddress);
  }
  const parseHalf = (half) =>
    half
      ? half.split(":").map((word) => {
          if (!/^[0-9a-f]{1,4}$/.test(word)) {
            throw new Error("direccion IPv6 invalida: " + rawAddress);
          }
          return Number.parseInt(word, 16);
        })
      : [];
  const left = parseHalf(halves[0]);
  const right = parseHalf(halves[1] || "");
  const missing = 8 - left.length - right.length;
  if (
    missing < 0 ||
    (halves.length === 1 && missing !== 0) ||
    (halves.length === 2 && missing < 1)
  ) {
    throw new Error("direccion IPv6 invalida: " + rawAddress);
  }
  return [...left, ...Array(missing).fill(0), ...right];
}

function classifyAddress(rawAddress) {
  const address = String(rawAddress).replace(/^\[|\]$/g, "").toLowerCase();
  const family = net.isIP(address);
  if (family === 4) {
    const parts = address.split(".").map(Number);
    const [first, second, third] = parts;
    if (first === 127) {
      return "loopback";
    }
    if (
      first === 10 ||
      (first === 100 && second >= 64 && second <= 127) ||
      (first === 172 && second >= 16 && second <= 31) ||
      (first === 192 && second === 168)
    ) {
      return "private";
    }
    if (
      first === 0 ||
      first >= 224 ||
      (first === 169 && second === 254) ||
      (first === 192 &&
        ((second === 0 && (third === 0 || third === 2)) ||
          (second === 88 && third === 99))) ||
      (first === 198 &&
        (second === 18 ||
          second === 19 ||
          (second === 51 && third === 100))) ||
      (first === 203 && second === 0 && third === 113)
    ) {
      return "blocked";
    }
    return "public";
  }

  if (family === 6) {
    const words = parseIpv6Words(address);
    const allZero = words.every((word) => word === 0);
    const loopback =
      words.slice(0, 7).every((word) => word === 0) && words[7] === 1;
    if (allZero) {
      return "blocked";
    }
    if (loopback) {
      return "loopback";
    }

    const classifyEmbeddedIpv4 = (high, low) =>
      classifyAddress(
        [high >> 8, high & 0xff, low >> 8, low & 0xff].join(".")
      );
    const mapped =
      words.slice(0, 5).every((word) => word === 0) &&
      words[5] === 0xffff;
    const compatible = words.slice(0, 6).every((word) => word === 0);
    if (mapped || compatible) {
      return classifyEmbeddedIpv4(words[6], words[7]);
    }

    const wellKnownNat64 =
      words[0] === 0x0064 &&
      words[1] === 0xff9b &&
      words.slice(2, 6).every((word) => word === 0);
    if (wellKnownNat64) {
      return classifyEmbeddedIpv4(words[6], words[7]);
    }
    if (
      words[0] === 0x0064 &&
      words[1] === 0xff9b &&
      words[2] === 0x0001
    ) {
      return "blocked";
    }

    if ((words[0] & 0xfe00) === 0xfc00) {
      return "private";
    }
    if ((words[0] & 0xffc0) === 0xfec0) {
      return "private";
    }
    if (
      (words[0] & 0xffc0) === 0xfe80 ||
      (words[0] & 0xff00) === 0xff00
    ) {
      return "blocked";
    }

    const isGlobalUnicast = (words[0] & 0xe000) === 0x2000;
    if (!isGlobalUnicast) {
      return "blocked";
    }

    const ietfProtocolAssignments =
      words[0] === 0x2001 && (words[1] & 0xfe00) === 0;
    const documentation =
      (words[0] === 0x2001 && words[1] === 0x0db8) ||
      (words[0] === 0x3fff && (words[1] & 0xf000) === 0);
    const sixToFour = words[0] === 0x2002;
    if (ietfProtocolAssignments || documentation || sixToFour) {
      return "blocked";
    }

    const isatap =
      (words[4] === 0 || words[4] === 0x0200) &&
      words[5] === 0x5efe;
    if (isatap) {
      return classifyEmbeddedIpv4(words[6], words[7]);
    }
    return "public";
  }

  throw new Error("direccion IP invalida: " + rawAddress);
}
function normalizePageUrl(raw, allowHttp = false) {
  const value = String(raw);
  const candidate = value.includes("://") ? value : "https://" + value;
  let url;
  try {
    url = new URL(candidate);
  } catch (error) {
    throw new Error("URL invalida para favicon: " + error.message);
  }
  validateNetworkUrl(url, allowHttp);
  url.hash = "";
  return url;
}

function validateNetworkUrl(url, allowHttp) {
  if (url.username || url.password) {
    throw new Error("el favicon no permite URLs con credenciales embebidas");
  }
  if (!url.hostname) {
    throw new Error("la URL del favicon debe incluir un host");
  }
  if (url.protocol === "https:") {
    return;
  }
  if (
    url.protocol === "http:" &&
    allowHttp &&
    isLoopbackHostname(url.hostname)
  ) {
    return;
  }
  throw new Error(
    "el favicon solo admite HTTPS, o HTTP loopback junto con --allow-http"
  );
}

async function withAbsoluteTimeout(promise, timeoutMs, message) {
  let timer = null;
  try {
    return await Promise.race([
      Promise.resolve(promise),
      new Promise((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error(message)), timeoutMs);
      })
    ]);
  } finally {
    if (timer) {
      clearTimeout(timer);
    }
  }
}

async function resolvePinnedAddress(url, options) {
  const hostname = url.hostname.replace(/^\[|\]$/g, "");
  const resolveHost = options.resolveHost || dns.lookup;
  let addresses;

  if (net.isIP(hostname)) {
    addresses = [{ address: hostname, family: net.isIP(hostname) }];
  } else {
    addresses = await withAbsoluteTimeout(
      resolveHost(hostname, { all: true, verbatim: true }),
      options.timeoutMs,
      "timeout resolviendo DNS para " + hostname
    );
  }

  if (!Array.isArray(addresses) || addresses.length === 0) {
    throw new Error("DNS no devolvio direcciones para " + hostname);
  }

  const normalized = addresses.map((entry) => {
    const address = typeof entry === "string" ? entry : entry.address;
    const family =
      typeof entry === "string" ? net.isIP(entry) : Number(entry.family);
    if (!address || ![4, 6].includes(family) || net.isIP(address) !== family) {
      throw new Error("DNS devolvio una direccion invalida para " + hostname);
    }
    return { address, family, scope: classifyAddress(address) };
  });

  if (normalized.some((entry) => entry.scope === "blocked")) {
    throw new Error("se bloqueo un destino de red sensible para " + hostname);
  }

  const scopes = new Set(normalized.map((entry) => entry.scope));
  if (scopes.size !== 1) {
    throw new Error(
      "DNS devolvio destinos de distinta confianza para " + hostname
    );
  }
  const scope = normalized[0].scope;

  if (options.expectedScope && scope !== options.expectedScope) {
    throw new Error(
      "se bloqueo un cambio de red durante la descarga desde " + hostname
    );
  }
  if (!options.allowPrivate && scope !== "public") {
    throw new Error("se bloqueo un destino privado no solicitado: " + hostname);
  }

  return { ...normalized[0], scope };
}

function createPinnedLookup(address) {
  return (_hostname, lookupOptions, callback) => {
    if (typeof lookupOptions === "function") {
      callback = lookupOptions;
      lookupOptions = {};
    }
    if (lookupOptions && lookupOptions.all) {
      callback(null, [{ address: address.address, family: address.family }]);
      return;
    }
    callback(null, address.address, address.family);
  };
}

async function requestOnce(url, options) {
  const startedAt = Date.now();
  const pinned = await resolvePinnedAddress(url, options);
  if (url.protocol === "http:" && pinned.scope !== "loopback") {
    throw new Error("HTTP solo puede resolver a una direccion loopback");
  }
  const elapsed = Date.now() - startedAt;
  const remaining = options.timeoutMs - elapsed;
  if (remaining <= 0) {
    throw new Error("timeout descargando " + url.href);
  }
  const transport = url.protocol === "https:" ? https : http;

  return new Promise((resolve, reject) => {
    let settled = false;
    let absoluteTimer = null;
    let request = null;
    const finish = (error, result) => {
      if (settled) {
        return;
      }
      settled = true;
      if (absoluteTimer) {
        clearTimeout(absoluteTimer);
      }
      if (error) {
        reject(error);
      } else {
        resolve(result);
      }
    };

    request = transport.request(
      url,
      {
        method: "GET",
        agent: false,
        lookup: createPinnedLookup(pinned),
        servername: net.isIP(url.hostname.replace(/^\[|\]$/g, ""))
          ? undefined
          : url.hostname,
        headers: {
          Accept: options.accept,
          "Accept-Encoding": "identity",
          "User-Agent": USER_AGENT
        }
      },
      (response) => {
        const status = response.statusCode || 0;
        const headers = response.headers;
        const isRedirect = [301, 302, 303, 307, 308].includes(status);

        if (isRedirect || status < 200 || status >= 300) {
          response.destroy();
          finish(null, {
            status,
            headers,
            body: Buffer.alloc(0),
            scope: pinned.scope
          });
          return;
        }

        const encoding = String(headers["content-encoding"] || "identity")
          .trim()
          .toLowerCase();
        if (encoding !== "identity") {
          response.destroy();
          finish(new Error("codificacion HTTP no permitida: " + encoding));
          return;
        }

        const declaredLength = Number(headers["content-length"]);
        if (
          Number.isFinite(declaredLength) &&
          declaredLength > options.maxBytes
        ) {
          response.destroy();
          finish(
            new Error(
              "respuesta demasiado grande: maximo " +
                options.maxBytes +
                " bytes"
            )
          );
          return;
        }

        const chunks = [];
        let total = 0;
        response.on("data", (chunk) => {
          total += chunk.length;
          if (total > options.maxBytes) {
            response.destroy(
              new Error(
                "respuesta demasiado grande: maximo " +
                  options.maxBytes +
                  " bytes"
              )
            );
            return;
          }
          chunks.push(chunk);
        });
        response.on("end", () => {
          finish(null, {
            status,
            headers,
            body: Buffer.concat(chunks, total),
            scope: pinned.scope
          });
        });
        response.on("error", (error) => finish(error));
      }
    );

    absoluteTimer = setTimeout(() => {
      request.destroy(new Error("timeout descargando " + url.href));
    }, remaining);
    absoluteTimer.unref?.();
    request.setTimeout(remaining, () => {
      request.destroy(new Error("timeout de inactividad descargando " + url.href));
    });
    request.on("error", (error) => finish(error));
    request.end();
  });
}

async function downloadResource(startUrl, options = {}) {
  let current =
    startUrl instanceof URL ? new URL(startUrl.href) : new URL(startUrl);
  const allowHttp = Boolean(options.allowHttp);
  const origin = options.origin || null;
  const visited = new Set();
  let expectedScope =
    options.expectedScope ||
    (current.protocol === "http:" ? "loopback" : "public");
  let requireHttps = current.protocol === "https:";
  const maxRedirects = options.maxRedirects ?? MAX_REDIRECTS;
  const deadline =
    options.deadline ??
    Date.now() + (options.timeoutMs ?? REQUEST_TIMEOUT_MS);

  for (let redirects = 0; redirects <= maxRedirects; redirects += 1) {
    validateNetworkUrl(current, allowHttp);
    if (requireHttps && current.protocol !== "https:") {
      throw new Error("se bloqueo un redirect HTTPS -> HTTP");
    }
    if (origin && current.origin !== origin) {
      throw new Error(
        "se bloqueo un favicon fuera del origen: " + current.origin
      );
    }
    if (visited.has(current.href)) {
      throw new Error("se detecto un ciclo de redirects");
    }
    visited.add(current.href);

    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      throw new Error("se agoto el tiempo total para obtener el favicon");
    }

    const response = await requestOnce(current, {
      accept: options.accept || "*/*",
      maxBytes: options.maxBytes ?? MAX_ICON_BYTES,
      timeoutMs: Math.min(
        options.timeoutMs ?? REQUEST_TIMEOUT_MS,
        remaining
      ),
      resolveHost: options.resolveHost,
      expectedScope,
      allowPrivate: expectedScope !== "public"
    });

    if (!expectedScope) {
      expectedScope = response.scope;
    }

    if ([301, 302, 303, 307, 308].includes(response.status)) {
      const location = response.headers.location;
      if (!location) {
        throw new Error("redirect sin Location desde " + current.href);
      }
      current = new URL(location, current);
      requireHttps ||= current.protocol === "https:";
      continue;
    }

    if (response.status < 200 || response.status >= 300) {
      throw new Error(
        "HTTP " + response.status + " descargando " + current.href
      );
    }

    return {
      body: response.body,
      contentType: String(response.headers["content-type"] || "")
        .split(";")[0]
        .trim()
        .toLowerCase(),
      finalUrl: current,
      networkScope: expectedScope
    };
  }

  throw new Error("demasiados redirects obteniendo el favicon");
}

function decodeHtmlNumericEntity(match, digits, radix) {
  const codePoint = Number.parseInt(digits, radix);
  if (
    !Number.isInteger(codePoint) ||
    codePoint < 0 ||
    codePoint > 0x10ffff ||
    (codePoint >= 0xd800 && codePoint <= 0xdfff)
  ) {
    return match;
  }
  return String.fromCodePoint(codePoint);
}

function decodeHtmlEntities(value) {
  return String(value)
    .replace(/&amp;/gi, "&")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;|&apos;/gi, "'")
    .replace(/&#x([0-9a-f]+);/gi, (match, digits) =>
      decodeHtmlNumericEntity(match, digits, 16)
    )
    .replace(/&#([0-9]+);/g, (match, digits) =>
      decodeHtmlNumericEntity(match, digits, 10)
    );
}
function parseTagAttributes(tag) {
  const attributes = {};
  const pattern =
    /([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>\x60]+)))?/g;
  let match;
  while ((match = pattern.exec(tag)) !== null) {
    const name = match[1].replace(/^</, "").toLowerCase();
    if (name === "link" || name === "base") {
      continue;
    }
    const value = match[2] ?? match[3] ?? match[4] ?? "";
    if (!(name in attributes)) {
      attributes[name] = decodeHtmlEntities(value);
    }
  }
  return attributes;
}

function resolveHtmlBase(html, documentUrl) {
  const match = String(html).match(/<base\b[^>]{0,4096}>/i);
  if (!match) {
    return documentUrl;
  }
  const attributes = parseTagAttributes(match[0]);
  if (!attributes.href) {
    return documentUrl;
  }
  try {
    const candidate = new URL(attributes.href, documentUrl);
    return candidate.origin === documentUrl.origin ? candidate : documentUrl;
  } catch {
    return documentUrl;
  }
}

function iconCandidateScore(attributes, url) {
  const type = String(attributes.type || "").toLowerCase();
  const pathname = url.pathname.toLowerCase();
  const rel = String(attributes.rel || "").toLowerCase();
  if (
    type.includes("image/x-icon") ||
    type.includes("image/vnd.microsoft.icon") ||
    pathname.endsWith(".ico")
  ) {
    return 0;
  }
  if (type.includes("image/png") || pathname.endsWith(".png")) {
    return rel.includes("apple-touch-icon") ? 20 : 10;
  }
  if (type.includes("svg") || pathname.endsWith(".svg")) {
    return 90;
  }
  return rel.includes("apple-touch-icon") ? 40 : 30;
}

function extractFaviconLinks(html, documentUrl) {
  const baseUrl = resolveHtmlBase(html, documentUrl);
  const icons = [];
  const manifests = [];
  const tags = String(html).match(/<link\b[^>]{0,4096}>/gi) || [];

  for (const tag of tags.slice(0, 64)) {
    const attributes = parseTagAttributes(tag);
    const relTokens = String(attributes.rel || "")
      .toLowerCase()
      .split(/\s+/)
      .filter(Boolean);
    if (!attributes.href) {
      continue;
    }

    let url;
    try {
      url = new URL(attributes.href, baseUrl);
      url.hash = "";
    } catch {
      continue;
    }

    if (relTokens.includes("manifest")) {
      manifests.push(url);
      continue;
    }
    if (
      !relTokens.includes("icon") &&
      !relTokens.includes("apple-touch-icon") &&
      !relTokens.includes("apple-touch-icon-precomposed")
    ) {
      continue;
    }
    icons.push({
      url,
      score: iconCandidateScore(attributes, url)
    });
  }

  icons.sort((left, right) => left.score - right.score);
  return {
    icons: icons.map((candidate) => candidate.url),
    manifests: manifests.slice(0, 2)
  };
}

function extractManifestIcons(rawManifest, manifestUrl) {
  let manifest;
  try {
    manifest = JSON.parse(String(rawManifest).replace(/^\uFEFF/, ""));
  } catch {
    return [];
  }
  if (!manifest || !Array.isArray(manifest.icons)) {
    return [];
  }

  return manifest.icons
    .slice(0, 32)
    .map((entry) => {
      if (!entry || typeof entry.src !== "string") {
        return null;
      }
      try {
        const url = new URL(entry.src, manifestUrl);
        return {
          url,
          score: iconCandidateScore(
            { type: entry.type || "", rel: "manifest-icon" },
            url
          )
        };
      } catch {
        return null;
      }
    })
    .filter(Boolean)
    .sort((left, right) => left.score - right.score)
    .map((entry) => entry.url);
}

function isValidPng(buffer) {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  if (
    !Buffer.isBuffer(buffer) ||
    buffer.length < 33 ||
    !buffer.subarray(0, 8).equals(signature) ||
    buffer.readUInt32BE(8) !== 13 ||
    buffer.toString("ascii", 12, 16) !== "IHDR"
  ) {
    return false;
  }
  const width = buffer.readUInt32BE(16);
  const height = buffer.readUInt32BE(20);
  return width > 0 && height > 0 && width <= 4096 && height <= 4096;
}

function isValidIco(buffer) {
  if (
    !Buffer.isBuffer(buffer) ||
    buffer.length < 22 ||
    buffer.readUInt16LE(0) !== 0 ||
    buffer.readUInt16LE(2) !== 1
  ) {
    return false;
  }
  const count = buffer.readUInt16LE(4);
  if (count < 1 || count > 32 || buffer.length < 6 + count * 16) {
    return false;
  }
  for (let index = 0; index < count; index += 1) {
    const entry = 6 + index * 16;
    const size = buffer.readUInt32LE(entry + 8);
    const offset = buffer.readUInt32LE(entry + 12);
    if (
      size === 0 ||
      offset < 6 + count * 16 ||
      offset + size > buffer.length
    ) {
      return false;
    }
  }
  return true;
}

function detectFaviconImage(buffer, contentType = "") {
  if (String(contentType).toLowerCase().startsWith("text/html")) {
    throw new Error("el candidato de icono devolvio HTML");
  }
  if (isValidIco(buffer)) {
    return "ico";
  }
  if (isValidPng(buffer)) {
    return "png";
  }
  throw new Error("el candidato no es un PNG o ICO valido");
}

function addSameOriginCandidate(target, seen, candidate, origin) {
  if (
    !candidate ||
    candidate.origin !== origin ||
    seen.has(candidate.href) ||
    target.length >= MAX_CANDIDATES
  ) {
    return;
  }
  seen.add(candidate.href);
  target.push(candidate);
}

async function discoverFavicon(rawPageUrl, options = {}) {
  const allowHttp = Boolean(options.allowHttp);
  const pageUrl = normalizePageUrl(rawPageUrl, allowHttp);
  const deadline =
    Date.now() + (options.totalTimeoutMs ?? TOTAL_TIMEOUT_MS);
  let page = null;
  let lastError = null;

  try {
    page = await downloadResource(pageUrl, {
      allowHttp,
      accept: "text/html,application/xhtml+xml;q=0.9,*/*;q=0.1",
      maxBytes: MAX_HTML_BYTES,
      deadline,
      resolveHost: options.resolveHost,
      timeoutMs: options.timeoutMs
    });
  } catch (error) {
    lastError = error;
  }

  const documentUrl = page?.finalUrl || pageUrl;
  const origin = documentUrl.origin;
  const seen = new Set();
  let attempts = 0;
  let extracted = { icons: [], manifests: [] };

  if (page) {
    extracted = extractFaviconLinks(page.body.toString("utf8"), documentUrl);
  }

  const tryCandidates = async (candidates, maximum) => {
    for (const candidate of candidates.slice(0, maximum)) {
      if (
        attempts >= MAX_CANDIDATES ||
        candidate.origin !== origin ||
        seen.has(candidate.href)
      ) {
        continue;
      }
      seen.add(candidate.href);
      attempts += 1;
      try {
        const icon = await downloadResource(candidate, {
          allowHttp,
          origin,
          expectedScope: page?.networkScope || null,
          accept:
            "image/x-icon,image/vnd.microsoft.icon,image/png;q=0.9,*/*;q=0.1",
          maxBytes: MAX_ICON_BYTES,
          deadline,
          resolveHost: options.resolveHost,
          timeoutMs: options.timeoutMs
        });
        const extension = detectFaviconImage(icon.body, icon.contentType);
        return {
          bytes: icon.body,
          extension,
          sourceUrl: icon.finalUrl.href
        };
      } catch (error) {
        lastError = error;
      }
    }
    return null;
  };

  const htmlResult = await tryCandidates(
    extracted.icons,
    MAX_CANDIDATES - 2
  );
  if (htmlResult) {
    return htmlResult;
  }

  const manifestCandidates = [];
  if (page && attempts < MAX_CANDIDATES - 1) {
    for (const manifestUrl of extracted.manifests.slice(0, 2)) {
      if (
        manifestUrl.origin !== origin ||
        attempts + manifestCandidates.length >= MAX_CANDIDATES - 1
      ) {
        continue;
      }
      try {
        const manifest = await downloadResource(manifestUrl, {
          allowHttp,
          origin,
          expectedScope: page.networkScope,
          accept: "application/manifest+json,application/json;q=0.9",
          maxBytes: MAX_MANIFEST_BYTES,
          deadline,
          resolveHost: options.resolveHost,
          timeoutMs: options.timeoutMs
        });
        for (const candidate of extractManifestIcons(
          manifest.body.toString("utf8"),
          manifest.finalUrl
        )) {
          if (manifestCandidates.length >= MAX_CANDIDATES - 1 - attempts) {
            break;
          }
          manifestCandidates.push(candidate);
        }
      } catch (error) {
        lastError = error;
      }
    }
  }

  const manifestResult = await tryCandidates(
    manifestCandidates,
    MAX_CANDIDATES - 1 - attempts
  );
  if (manifestResult) {
    return manifestResult;
  }

  const fallbackResult = await tryCandidates(
    [new URL("/favicon.ico", documentUrl)],
    1
  );
  if (fallbackResult) {
    return fallbackResult;
  }

  throw new Error(
    "no se encontro un favicon PNG/ICO valido" +
      (lastError ? ": " + lastError.message : "")
  );
}

module.exports = {
  MAX_ICON_BYTES,
  classifyAddress,
  detectFaviconImage,
  discoverFavicon,
  downloadResource,
  extractFaviconLinks,
  extractManifestIcons,
  isLoopbackHostname,
  isValidIco,
  isValidPng,
  normalizePageUrl
};
