// Explicit pipes are necessary for a Windows GUI-subsystem executable.
const { spawnSync } = require('node:child_process');
const result = spawnSync(process.argv[2], ['--version'], { encoding: 'utf8', windowsHide: true, timeout: 15000 });
if (result.stdout) process.stdout.write(result.stdout);
if (result.stderr) process.stderr.write(result.stderr);
if (result.error) throw result.error;
process.exit(result.status ?? 1);
