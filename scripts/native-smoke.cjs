'use strict';
// Real Windows/WebView2 integration. No test bridge is compiled into the app.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const { spawn, spawnSync } = require('node:child_process');
const { chromium } = require('@playwright/test');
const { setTimeout: delay } = require('node:timers/promises');
const exe = path.resolve(process.env.MISKU_TEST_EXE || 'runtime/misku-native-views.exe');
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'misku-native-test-'));
const evidence = path.resolve('artifacts/native-test');
const children = [], browsers = [];
let currentPage;
const env = { ...process.env, MISKU_NV_HOME: path.join(root, 'home'), MISKU_NV_PROGRAMS_DIR: path.join(root, 'shortcuts'), MISKU_NV_PROFILE_ROOT: path.join(root, 'profiles'), WEBVIEW2_USER_DATA_FOLDER: undefined };
const server = http.createServer((req, res) => {
  if (req.url === '/favicon.ico') { res.writeHead(404).end(); return; }
  res.setHeader('Content-Type', 'text/html');
  res.end('<!doctype html><title>Misku isolation test</title><h1>Local app</h1>');
});
async function freePort() {
  const net = require('node:net'); const listener = net.createServer();
  await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
  const port = listener.address().port; await new Promise(resolve => listener.close(resolve)); return port;
}
function launch(args, port) {
  const child = spawn(exe, args, { env: { ...env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1` }, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
  child.on('error', error => { fs.appendFileSync(path.join(evidence, 'stderr.txt'), `${error}\n`); });
  child.stdout.on('data', bytes => fs.appendFileSync(path.join(evidence, 'stdout.txt'), bytes));
  child.stderr.on('data', bytes => fs.appendFileSync(path.join(evidence, 'stderr.txt'), bytes));
  children.push(child); return child;
}
async function connect(port, child, predicate) {
  const deadline = Date.now() + 45000;
  let browser;
  while (Date.now() < deadline) {
    assert.equal(child.exitCode, null, `Runtime exited before WebView2: ${child.exitCode}`);
    try {
      if (!browser) { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1000 }); browsers.push(browser); }
      const page = browser.contexts().flatMap(c => c.pages()).find(p => predicate(p.url()));
      if (page) { await page.waitForLoadState('domcontentloaded'); currentPage = page; return page; }
    } catch { /* WebView2 creates its endpoint asynchronously. */ }
    await delay(250);
  }
  throw new Error(`WebView2 no disponible en ${port}`);
}
async function stop(child) {
  if (child.exitCode !== null) return;
  const exited = new Promise(resolve => child.once('exit', resolve));
  // PID belongs to the child launched above, never a name-based process kill.
  spawnSync('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true });
  await Promise.race([exited, delay(5000)]);
}
async function main() {
  fs.mkdirSync(evidence, { recursive: true });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${server.address().port}/app`;
  const registry = path.join(env.MISKU_NV_HOME, 'apps.toml');
  const managerPort = await freePort();
  const manager = launch(['--manager', '--config', registry], managerPort);
  const page = await connect(managerPort, manager, u => u.includes('tauri.localhost'));
  await page.locator('#add').waitFor();
  async function create(name) {
    await page.locator('#add').click();
    await page.locator('#url').fill(url); await page.locator('#name').fill(name);
    await page.locator('.advanced summary').click(); await page.locator('#http').check();
    await page.locator('#open-after').uncheck(); await page.locator('#save').click();
    await page.getByRole('button', { name: `Abrir ${name}`, exact: true }).waitFor();
    assert.equal(await page.locator('#error').isVisible(), false);
  }
  await create('Primera'); await create('Segunda');
  const snapshot = () => page.evaluate(() => window.__TAURI__.core.invoke('manager_snapshot'));
  const apps = (await snapshot()).apps;
  assert.equal(apps.length, 2); assert.notEqual(apps[0].instance_id, apps[1].instance_id);
  const shortcutFiles = fs.readdirSync(env.MISKU_NV_PROGRAMS_DIR).filter(x => x.endsWith('.lnk'));
  assert.equal(shortcutFiles.length, 2);
  const manifest = app => path.join(env.MISKU_NV_HOME, 'apps', app.instance_id, 'app.toml');
  const args = app => ['--config', manifest(app), '--app', app.instance_id];
  const portA = await freePort(); const appA = launch(args(apps[0]), portA);
  const viewA = await connect(portA, appA, u => u === url);
  await viewA.evaluate(() => localStorage.setItem('misku-test', 'first-profile'));
  // Remote content must never reach privileged manager commands.
  const denied = await viewA.evaluate(async () => {
    try { await window.__TAURI__.core.invoke('manager_snapshot'); return false; } catch { return true; }
  });
  assert.equal(denied, true);
  const duplicate = launch(args(apps[0]), await freePort());
  await Promise.race([new Promise(resolve => duplicate.once('exit', resolve)), delay(5000)]);
  assert.equal(duplicate.exitCode, 0); assert.equal(appA.exitCode, null);
  const before = fs.statSync(manifest(apps[0])).mtimeMs;
  await page.getByRole('button', { name: 'Abrir Primera', exact: true }).click();
  await page.getByRole('button', { name: 'Abrir Primera', exact: true }).waitFor({ state: 'visible' });
  await page.waitForFunction(() => !document.querySelector('.app-row[aria-busy]'));
  assert.equal(fs.statSync(manifest(apps[0])).mtimeMs, before, 'Repeated open must not export again');
  const portB = await freePort(); const appB = launch(args(apps[1]), portB);
  const viewB = await connect(portB, appB, u => u === url);
  assert.equal(await viewB.evaluate(() => localStorage.getItem('misku-test')), null);
  await stop(appB); await stop(appA);
  const portAgain = await freePort(); const again = launch(args(apps[0]), portAgain);
  const viewAgain = await connect(portAgain, again, u => u === url);
  assert.equal(await viewAgain.evaluate(() => localStorage.getItem('misku-test')), 'first-profile');
  await stop(again);
  await page.getByRole('button', { name: 'Editar Primera', exact: true }).click();
  await page.locator('#name').fill('Renombrada'); await page.locator('#save').click();
  await page.getByRole('button', { name: 'Abrir Renombrada', exact: true }).waitFor();
  assert.equal((await snapshot()).apps[0].instance_id, apps[0].instance_id);
  assert.equal(fs.readdirSync(env.MISKU_NV_PROGRAMS_DIR).filter(x => x.endsWith('.lnk')).length, 2);
  await page.getByRole('button', { name: 'Eliminar Renombrada', exact: true }).click();
  await page.locator('#delete-confirm').click();
  await page.waitForFunction(() => document.querySelector('#count').textContent === '1 app');
  assert.equal(fs.existsSync(manifest(apps[0])), true, 'Removal preserves the isolated installation data');
  currentPage = page;
  await page.screenshot({ path: path.join(evidence, 'manager.png') });
  console.log('Native manager, shortcuts, cache, IPC isolation, duplicate instance and persistent isolated storage passed.');
}
main().catch(async error => {
  fs.mkdirSync(evidence, { recursive: true });
  fs.writeFileSync(path.join(evidence, 'failure.txt'), error.stack || String(error));
  try { await currentPage?.screenshot({ path: path.join(evidence, 'failure.png') }); } catch {}
  console.error(error); process.exitCode = 1;
}).finally(async () => {
  for (const child of children.reverse()) await stop(child);
  for (const browser of browsers) { try { await browser.close(); } catch {} }
  server.close();
  if (path.dirname(root) !== os.tmpdir() || !path.basename(root).startsWith('misku-native-test-')) throw new Error('Unsafe cleanup');
  // Only our generated test root is removed; WebView2 can take a moment to exit.
  fs.rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
});
