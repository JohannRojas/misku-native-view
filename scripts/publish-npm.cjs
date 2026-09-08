'use strict';
// Re-running a partial release is safe only if npm has these exact bytes.
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { releasePolicy, validateVersions } = require('./release-policy.cjs');
const directory = path.resolve(process.argv[2]);
const policy = releasePolicy(validateVersions());
if (process.argv[3] !== policy.npm_tag) throw new Error('Canal npm incompatible con la versión');
const name = require('../package.json').name;
const files = fs.readdirSync(directory).filter(p => p.endsWith('.tgz'));
if (files.length !== 1 || files[0] !== `${name}-${policy.version}.tgz`) throw new Error('Tarball inesperado');
const tarball = path.join(directory, files[0]);
const integrity = `sha512-${crypto.createHash('sha512').update(fs.readFileSync(tarball)).digest('base64')}`;
const existing = spawnSync('npm', ['view', `${name}@${policy.version}`, 'dist.integrity', '--json'], { encoding: 'utf8', timeout: 60000 });
if (existing.error) throw existing.error;
if (existing.status === 0) {
  if (JSON.parse(existing.stdout) !== integrity) throw new Error('npm ya contiene esta versión con bytes diferentes. Usa una nueva versión.');
  console.log('npm ya contiene el mismo tarball; se continúa la publicación de los assets.');
} else {
  let error;
  try { error = JSON.parse(existing.stdout).error; } catch { /* Fail closed. */ }
  if (error?.code !== 'E404') throw new Error(`No se pudo comprobar npm: ${existing.stderr}`);
  const publish = spawnSync('npm', ['publish', tarball, '--access', 'public', '--provenance', '--ignore-scripts', '--tag', policy.npm_tag], { stdio: 'inherit', timeout: 180000 });
  if (publish.error || publish.status !== 0) throw publish.error || new Error('npm publish falló');
}
