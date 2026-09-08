'use strict';
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
function releasePolicy(version, tag) {
  const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;
  const match = semver.exec(version);
  if (!match || (match[4] && match[4].split('.').some(p => /^0\d+$/.test(p)))) throw new Error('Versión semver inválida');
  if (tag !== undefined && tag !== `v${version}`) throw new Error(`El tag debe ser v${version}`);
  return { version, npm_tag: match[4] ? 'next' : 'latest', prerelease: !!match[4] };
}
function validateVersions() {
  const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json')));
  const config = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json')));
  const cargo = fs.readFileSync(path.join(root, 'src-tauri/Cargo.toml'), 'utf8').match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (pkg.version !== config.version || pkg.version !== cargo) throw new Error('package.json, Cargo.toml y tauri.conf.json deben tener la misma versión');
  return pkg.version;
}
if (require.main === module) {
  const policy = releasePolicy(validateVersions(), process.argv[2]);
  for (const [key, value] of Object.entries(policy)) console.log(`${key}=${value}`);
}
module.exports = { releasePolicy, validateVersions };
