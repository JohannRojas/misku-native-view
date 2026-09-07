const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;

async function fixture(page, { apps = [], failLoad = false, warning = null } = {}) {
  await page.addInitScript(({ apps, failLoad, warning }) => {
    window.calls = [];
    window.__TAURI__ = { core: { invoke: async (command, args) => {
      window.calls.push({ command, args });
      if (command === 'manager_snapshot') {
        if (failLoad) { failLoad = false; throw new Error('Registro no disponible'); }
        return { apps, version: '0.3.0' };
      }
      if (command === 'manager_create') {
        if (args.input.url.includes('invalid')) throw new Error('La URL no es válida');
        const profile = { name: args.input.name || 'Example', url: args.input.url, instance_id: 'one', allowed_origins: [], suspend_when_minimized: args.input.suspendWhenMinimized };
        apps.push(profile); return { profile, warning };
      }
      if (command === 'manager_update') { Object.assign(apps.find(a => a.instance_id === args.id), args.input); return { warning }; }
      if (command === 'manager_remove') apps = apps.filter(a => a.instance_id !== args.id);
    } } };
  }, { apps, failLoad, warning });
  await page.goto('/');
}

test('crear, buscar, editar y eliminar conservando la confirmación', async ({ page }) => {
  await fixture(page);
  await page.getByRole('button', { name: 'Añadir app', exact: true }).first().click();
  await page.locator('#url').fill('example.com/Account');
  await page.locator('#name').fill('Trabajo');
  await page.locator('#save').click();
  await expect(page.getByRole('button', { name: 'Abrir Trabajo', exact: true })).toBeVisible();
  await page.keyboard.press('Control+k');
  await expect(page.locator('#search')).toBeFocused();
  await page.locator('#search').fill('no coincide');
  await expect(page.locator('#status')).toHaveText('No hay apps que coincidan con tu búsqueda.');
  await page.locator('#search').fill('');
  await page.getByRole('button', { name: 'Editar Trabajo', exact: true }).click();
  await page.locator('#name').fill('Personal');
  await page.locator('#save').click();
  await page.getByRole('button', { name: 'Eliminar Personal', exact: true }).click();
  await page.locator('#delete-cancel').click();
  await expect(page.getByRole('button', { name: 'Abrir Personal', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Eliminar Personal', exact: true }).click();
  await page.locator('#delete-confirm').click();
  await expect(page.locator('#count')).toHaveText('0 apps');
  const create = await page.evaluate(() => window.calls.find(c => c.command === 'manager_create'));
  expect(create.args.input.url).toBe('https://example.com/Account');
  expect(create.args.input.suspendWhenMinimized).toBe(false);
});

test('un error de validación conserva el formulario y permite corregirlo', async ({ page }) => {
  await fixture(page);
  await page.locator('#add').click();
  await page.locator('#url').fill('invalid.test');
  await page.locator('#save').click();
  await expect(page.locator('#form-error')).toContainText('URL');
  await expect(page.locator('#save')).toBeEnabled();
  await page.locator('#url').fill('example.com');
  await page.locator('#save').click();
  await expect(page.locator('#count')).toHaveText('1 app');
});

test('un fallo de integración muestra la app guardada sin volver a crearla', async ({ page }) => {
  await fixture(page, { warning: 'La app se guardó. Pulsa Abrir para reintentar.' });
  await page.locator('#add').click();
  await page.locator('#url').fill('example.com');
  await page.locator('#save').click();
  await expect(page.locator('#editor')).not.toBeVisible();
  await expect(page.locator('#count')).toHaveText('1 app');
  await expect(page.locator('#error')).toBeVisible();
  await page.getByRole('button', { name: 'Abrir Example', exact: true }).click();
  expect(await page.evaluate(() => window.calls.filter(c => c.command === 'manager_create').length)).toBe(1);
});

test('recupera un fallo inicial y trata los nombres como texto', async ({ page }) => {
  await fixture(page, { failLoad: true, apps: [{ instance_id: 'one', name: '<img src=x onerror=alert(1)>', url: 'https://example.com' }] });
  await expect(page.locator('#error')).toBeVisible();
  await page.locator('#retry').click();
  await expect(page.locator('.app-name')).toHaveText('<img src=x onerror=alert(1)>');
  await expect(page.locator('.app-name img')).toHaveCount(0);
});

for (const width of [440, 1280]) test(`accesibilidad y controles sin desbordamiento a ${width}px`, async ({ page }) => {
  await page.setViewportSize({ width, height: 720 });
  await fixture(page, { apps: [{ instance_id: 'one', name: 'Una app de trabajo', url: 'https://example.com/a/very/long/path' }] });
  await expect(page.locator('#count')).toHaveText('1 app');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.locator('#add').click();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
