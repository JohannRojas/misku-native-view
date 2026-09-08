'use strict';
const fs = require('node:fs');
// Tauri 2.11.4 patches its bundle token for NSIS, then restores the unsigned
// build output. Apply that exact metadata change to the npm copy as well.
// The installer smoke test still requires a byte-for-byte SHA-256 match.
const runtime = process.argv[2];
const bytes = fs.readFileSync(runtime);
// Check the PE security directory directly: this also works when prepack is
// invoked by Windows PowerShell 5 with PowerShell 7's module search path.
const pe = bytes.readUInt32LE(0x3c);
if (bytes.toString('ascii', 0, 2) !== 'MZ' || bytes.readUInt32LE(pe) !== 0x4550) throw new Error('Runtime PE inválido');
const optional = pe + 24;
const magic = bytes.readUInt16LE(optional);
if (magic !== 0x20b && magic !== 0x10b) throw new Error('Cabecera PE desconocida');
const security = optional + (magic === 0x20b ? 112 : 96) + 4 * 8;
if (bytes.readUInt32LE(security) || bytes.readUInt32LE(security + 4)) {
  throw new Error('Un runtime firmado debe extraerse del instalador; no se modifica su firma');
}
const original = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK');
const nsis = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS');
const offset = bytes.indexOf(original);
if (offset < 0 || bytes.lastIndexOf(original) !== offset) {
  throw new Error('No se encontró un único marcador de Tauri; revisa el empaquetado antes de publicar');
}
nsis.copy(bytes, offset);
fs.writeFileSync(runtime, bytes);
