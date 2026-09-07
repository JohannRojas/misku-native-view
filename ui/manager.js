"use strict";
(() => {
  const $ = (id) => document.getElementById(id);
  const invoke = (command, args) => {
    if (!window.__TAURI__?.core?.invoke) return Promise.reject(new Error("Abre el gestor desde la aplicación Misku."));
    return window.__TAURI__.core.invoke(command, args);
  };
  let apps = [], editing = null, removing = null, busy = false, lastFocused = null;
  function errorText(error) { return String(error?.message || error); }
  function report(error) { $("error-summary").textContent = "No se pudo completar la operación."; $("error-detail").textContent = errorText(error); $("error").hidden = false; }
  function notify(text) { $("status").textContent = text; }
  function button(text, action, className = "", label = text) {
    const el = document.createElement("button"); el.type = "button"; el.textContent = text; el.className = className; el.setAttribute("aria-label", label); el.addEventListener("click", action); return el;
  }
  function render() {
    const term = $("search").value.trim().toLocaleLowerCase();
    const visible = apps.filter(app => `${app.name} ${app.url}`.toLocaleLowerCase().includes(term));
    const fragment = document.createDocumentFragment();
    for (const app of visible) {
      const row = document.createElement("li"); row.className = "app-row";
      const icon = document.createElement("span"); icon.className = "app-icon"; icon.textContent = app.name.trim().slice(0, 1).toUpperCase(); icon.setAttribute("aria-hidden", "true");
      const copy = document.createElement("div"); copy.className = "app-copy";
      const name = document.createElement("div"); name.className = "app-name"; name.textContent = app.name;
      const host = document.createElement("div"); host.className = "app-host"; host.textContent = app.url; host.title = app.url;
      copy.append(name, host);
      const actions = document.createElement("div"); actions.className = "row-actions";
      actions.append(button("Abrir", async (event) => {
        const control = event.currentTarget; control.disabled = true; row.setAttribute("aria-busy", "true");
        try { await invoke("manager_open", { id: app.instance_id }); notify(`${app.name} abierta.`); }
        catch (error) { report(error); }
        finally { control.disabled = false; row.removeAttribute("aria-busy"); }
      }, "", `Abrir ${app.name}`), button("Editar", () => edit(app), "subtle", `Editar ${app.name}`), button("Eliminar", () => confirmRemove(app), "subtle", `Eliminar ${app.name}`));
      row.append(icon, copy, actions); fragment.append(row);
    }
    $("apps").replaceChildren(fragment); $("empty").hidden = apps.length > 0;
    $("count").textContent = `${apps.length} ${apps.length === 1 ? "app" : "apps"}`;
    if (term) notify(visible.length ? `${visible.length} resultados` : "No hay apps que coincidan con tu búsqueda.");
    else if (!apps.length) notify("");
  }
  async function load() {
    try { const snapshot = await invoke("manager_snapshot"); apps = snapshot.apps; $("version").textContent = `Misku ${snapshot.version}`; $("error").hidden = true; notify(""); render(); }
    catch (error) { notify(""); report(error); }
  }
  function edit(app = null) {
    lastFocused = document.activeElement; editing = app; $("app-form").reset(); $("form-error").hidden = true;
    document.querySelector('.advanced').open = false;
    $("editor-title").textContent = app ? "Editar app" : "Añadir app"; $("save").textContent = app ? "Guardar cambios" : "Crear app";
    $("url").value = app?.url || ""; $("name").value = app?.name || ""; $("name").required = !!app;
    $("suspend").checked = app?.suspend_when_minimized || false; $("http").checked = app?.allow_insecure_http || false;
    $("origins").value = (app?.allowed_origins || []).join("\n"); $("open-option").hidden = !!app;
    $("editor").showModal(); $(app ? "name" : "url").focus();
  }
  function finishDialog(id) { $(id).close(); (lastFocused?.isConnected ? lastFocused : $("add")).focus(); }
  $("app-form").addEventListener("submit", async (event) => {
    event.preventDefault(); if (busy) return;
    let url = $("url").value.trim(); if (!url.includes("://")) url = `https://${url}`;
    const input = { url, name: $("name").value.trim(), allowedOrigins: $("origins").value.split(/\r?\n/).map(x => x.trim()).filter(Boolean), suspendWhenMinimized: $("suspend").checked, allowInsecureHttp: $("http").checked };
    busy = true; $("save").disabled = true; $("cancel").disabled = true; $("form-error").hidden = true;
    try {
      const saved = editing
        ? await invoke("manager_update", { id: editing.instance_id, input })
        : await invoke("manager_create", { input, openAfter: $("open-after").checked });
      finishDialog("editor"); await load(); notify(editing ? "Cambios guardados. Se aplican al volver a abrir la app." : "App creada y añadida al menú Inicio.");
      if (saved.warning) { notify("App guardada."); report(saved.warning); }
    } catch (error) { $("form-error").textContent = errorText(error); $("form-error").hidden = false; }
    finally { busy = false; $("save").disabled = false; $("cancel").disabled = false; }
  });
  function confirmRemove(app) { lastFocused = document.activeElement; removing = app; $("delete-description").textContent = `¿Quieres eliminar «${app.name}»?`; $("delete-error").hidden = true; $("delete-dialog").showModal(); $("delete-cancel").focus(); }
  $("delete-confirm").addEventListener("click", async () => {
    if (busy || !removing) return; busy = true; $("delete-confirm").disabled = true; $("delete-cancel").disabled = true;
    try { await invoke("manager_remove", { id: removing.instance_id }); finishDialog("delete-dialog"); await load(); notify("App eliminada. Sus sesiones se han conservado."); }
    catch (error) { $("delete-error").textContent = errorText(error); $("delete-error").hidden = false; }
    finally { busy = false; $("delete-confirm").disabled = false; $("delete-cancel").disabled = false; }
  });
  $("add").addEventListener("click", () => edit()); $("empty-add").addEventListener("click", () => edit());
  $("cancel").addEventListener("click", () => finishDialog("editor")); $("delete-cancel").addEventListener("click", () => finishDialog("delete-dialog"));
  for (const id of ["editor", "delete-dialog"]) $(id).addEventListener("cancel", (e) => { if (busy) e.preventDefault(); });
  $("search").addEventListener("input", render); $("retry").addEventListener("click", load);
  document.addEventListener("keydown", (e) => { if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k" && !$("editor").open && !$("delete-dialog").open) { e.preventDefault(); $("search").focus(); } });
  window.addEventListener("focus", () => { if (!busy && !$("editor").open && !$("delete-dialog").open) load(); });
  load();
})();
