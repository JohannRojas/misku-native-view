'use strict';
const fs = require('node:fs');
// Tauri 2.11.4 patches its bundle token for NSIS, then restores the unsigned
// build output. Apply that exact metadata change to the npm copy as well.
// The installer smoke test still requires a byte-for-byte SHA-256 match.
const runtime = process.argv[2];
const bytes = fs.readFileSync(runtime);
const original = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK');
const nsis = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS');
const offset = bytes.indexOf(original);
if (offset < 0 || bytes.lastIndexOf(original) !== offset) {
  throw new Error('No se encontró un único marcador de Tauri; revisa el empaquetado antes de publicar');
}
nsis.copy(bytes, offset);
fs.writeFileSync(runtime, bytes);
