"use strict";

// Every piece of data comes from savepointd through the `ipc` command.
// Nothing refreshes while the window is hidden. Texts come from lang/*.json
// through t() / tn() (see i18n.js).

const invoke = window.__TAURI__.core.invoke;
const call = (cmd, args = {}) => invoke("ipc", { req: { cmd, ...args } });

const $ = (sel, root = document) => root.querySelector(sel);
const view = $("#view");

let status = null;
let route = { name: "list" };
let query = "";
let timer = null;
let langPref = "auto";
let locationsOpen = false;

const LANG_KEY = "savepoint.lang";

// ---------- formatting ----------

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

function ago(unix) {
  const s = Math.max(0, Date.now() / 1000 - unix);
  if (s < 60) return t("time.now");
  const rtf = new Intl.RelativeTimeFormat(i18n.code, { numeric: "auto" });
  if (s < 3600) return rtf.format(-Math.floor(s / 60), "minute");
  if (s < 86400) return rtf.format(-Math.floor(s / 3600), "hour");
  const days = Math.floor(s / 86400);
  if (days < 7) return rtf.format(-days, "day");
  return new Intl.DateTimeFormat(i18n.code, { day: "numeric", month: "short" }).format(new Date(unix * 1000));
}

function stamp(unix) {
  const d = new Date(unix * 1000);
  const sameYear = d.getFullYear() === new Date().getFullYear();
  return new Intl.DateTimeFormat(i18n.code, {
    day: "numeric", month: "short", year: sameYear ? undefined : "numeric", hour: "2-digit", minute: "2-digit",
  }).format(d);
}

function bytes(n) {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB"];
  let v = n / 1024, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${new Intl.NumberFormat(i18n.code, { maximumFractionDigits: v < 10 ? 1 : 0 }).format(v)} ${units[i]}`;
}

const fileCount = (n) => tn("backup.files", n);

/** The text for something the service reported, which may be a code or a system message. */
function lookup(prefix, code, fallback) {
  const key = `${prefix}${code}`;
  const text = t(key);
  return text === key ? fallback : text;
}
const errText = (e) => lookup("err.", String(e), String(e));
const problemText = (e) => lookup("errcode.", e.code, e.detail);
const problemOf = (game) => (status?.errors ?? []).find((e) => e.game === game);

// ---------- feedback ----------

let toastTimer;
function toast(message, isError = false) {
  const el = $("#toast");
  el.textContent = message;
  el.classList.toggle("error", isError);
  el.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("show"), isError ? 4200 : 2400);
}

async function attempt(fn) {
  try {
    return await fn();
  } catch (e) {
    toast(errText(e), true);
    return undefined;
  }
}

/** `lines` are strings, or { text, warn } for a highlighted line. */
function confirmAction(title, lines, ok) {
  const dlg = $("#confirm-dialog");
  $("#confirm-title").textContent = title;
  $("#confirm-text").innerHTML = [].concat(lines)
    .map((l) => (typeof l === "string" ? `<p>${esc(l)}</p>` : `<p class="${l.warn ? "warn" : ""}">${esc(l.text)}</p>`))
    .join("");
  $("#confirm-ok").textContent = ok;
  dlg.returnValue = "";
  dlg.showModal();
  return new Promise((resolve) => dlg.addEventListener("close", () => resolve(dlg.returnValue === "ok"), { once: true }));
}

// ---------- data ----------

let lastStatus = "";

async function refresh() {
  const next = await attempt(() => call("status"));
  if (!next) return;
  status = next;
  if (status.lang !== langPref) await applyLanguage(status.lang);
  // Most polls bring nothing new; redrawing then only wastes work (and would
  // clear a selection the user is making).
  // The minute is included so "5 minutes ago" keeps counting while nothing else changes.
  const snapshot = JSON.stringify(next) + Math.floor(Date.now() / 60000);
  if (snapshot === lastStatus) return;
  lastStatus = snapshot;
  renderChrome();
  if (route.name === "list") renderList();
}

/** What the open game page shows; the page is only redrawn when this changes. */
let gameSnapshot = "";
const snapshotOf = (name) =>
  JSON.stringify([status?.games.find((g) => g.name === name), (status?.errors ?? []).filter((e) => e.game === name)]);

async function applyLanguage(pref) {
  langPref = pref;
  try { localStorage.setItem(LANG_KEY, pref); } catch { /* the cache is only a convenience */ }
  await setLanguage(pref);
  if (status) rerender();
}

function rerender() {
  if (route.name === "list") showList();
  else if (route.name === "game") renderGame(route.game, true);
  else if (route.name === "settings") renderSettings();
  else if (route.name === "hidden") renderHidden();
  if (status) renderChrome();
}

function schedule() {
  clearTimeout(timer);
  if (document.visibilityState !== "visible") return;
  // Faster while something is happening, otherwise a slow heartbeat.
  const active = status && (status.scanning || status.busy || status.games.some((g) => g.running));
  // The panel can stay open behind a game, so look less often when it is not in use.
  const delay = !document.hasFocus() ? 15000 : active ? 1500 : 5000;
  timer = setTimeout(async () => {
    await refresh();
    if (route.name === "game" && snapshotOf(route.game) !== gameSnapshot) await renderGame(route.game, true);
    schedule();
  }, delay);
}

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") refresh().then(schedule);
  else clearTimeout(timer);
});
window.addEventListener("focus", () => refresh().then(schedule));

// ---------- chrome ----------

function renderChrome() {
  const activity = $("#activity");
  let text = "";
  if (status.busy) text = t(status.busy.kind === "restore" ? "activity.restoring" : "activity.backing_up", { game: status.busy.game });
  else if (status.scanning) text = t("activity.scanning");
  activity.textContent = text;
  activity.classList.toggle("live", Boolean(text));
  $("#open-root").textContent = status.backup_dir;
  renderAlert();
}

/** The red strip: shown whenever a backup (or the game scan) has failed. */
function renderAlert() {
  const alert = $("#alert");
  const problems = [...(status.errors ?? [])].sort((a, b) => b.time - a.time);
  alert.hidden = problems.length === 0;
  if (!problems.length) return;
  const first = problems[0];
  const text = problemText(first);
  const label = problems.length > 1
    ? t("alert.many", { n: problems.length, game: first.game ?? "", text })
    : first.game ? t("alert.one", { game: first.game, text }) : t("alert.general", { text });
  const open = $("#alert-open");
  open.textContent = label;
  open.title = first.detail;
  alert.dataset.game = first.game ?? "";
}

// ---------- list ----------

function sortedGames() {
  return [...status.games].sort((a, b) =>
    (b.running - a.running) ||
    ((b.last_backup ?? 0) - (a.last_backup ?? 0)) ||
    a.name.localeCompare(b.name, i18n.code));
}

// Icons arrive once per game and are kept for as long as the panel is open.
const icons = new Map();

function iconBox(g, small = false) {
  const src = icons.get(g.name);
  const inner = src ? `<img alt="" src="${src}">` : esc([...g.name.trim()][0]?.toLocaleUpperCase(i18n.code) ?? "?");
  return `<span class="gicon${small ? " sm" : ""}" data-icon="${esc(g.name)}">${inner}</span>`;
}

async function loadIcons(games) {
  for (const g of games) {
    if (!g.has_icon || icons.has(g.name)) continue;
    try {
      icons.set(g.name, `data:image/png;base64,${await call("icon", { game: g.name })}`);
    } catch {
      continue;
    }
    for (const el of view.querySelectorAll(".gicon[data-icon]")) {
      if (el.dataset.icon === g.name) el.innerHTML = `<img alt="" src="${icons.get(g.name)}">`;
    }
  }
}

function gameMeta(g) {
  if (g.running) return `<span class="meta on">${t("meta.playing")}</span>`;
  const problem = problemOf(g.name);
  if (problem) return `<span class="meta err">${t(problem.code === "suspect_save" ? "meta.suspect" : "meta.failed")}</span>`;
  if (!g.local) return `<span class="meta">${t("meta.backup_only")}</span>`;
  if (!g.last_backup) return `<span class="meta">${t("meta.no_backup")}</span>`;
  return `<span class="meta">${ago(g.last_backup)}</span>`;
}

function showList() {
  route = { name: "list" };
  view.innerHTML = `
    <div class="toolbar">
      <input class="search" id="search" type="search" placeholder="${esc(t("list.search"))}" spellcheck="false" value="${esc(query)}">
      <span class="count" id="count"></span>
      <button class="btn icon quiet" id="rescan" aria-label="${esc(t("list.rescan"))}"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M23 4v6h-6"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg></button>
    </div>
    <ul class="games" id="games"></ul>`;
  $("#search").addEventListener("input", (e) => { query = e.target.value; renderList(); });
  $("#rescan").addEventListener("click", async () => {
    $("#rescan").classList.add("spin");
    await attempt(() => call("rescan"));
    await refresh();
  });
  if (status) renderList();
}

function renderList() {
  const list = $("#games");
  if (!list) return;
  const games = sortedGames();
  const q = query.trim().toLocaleLowerCase(i18n.code);
  const shown = q ? games.filter((g) => g.name.toLocaleLowerCase(i18n.code).includes(q)) : games;
  $("#count").textContent = tn("list.count", games.length);
  const rescan = $("#rescan");
  rescan.classList.toggle("spin", status.scanning);
  rescan.title = status.scanning ? t("list.scanning")
    : status.last_scan ? t("list.rescan_last", { when: ago(status.last_scan) }) : t("list.rescan");

  if (!games.length) {
    list.innerHTML = status.scanning
      ? `<li class="empty"><h2>${t("list.empty_scanning_title")}</h2><p>${t("list.empty_scanning_text")}</p></li>`
      : `<li class="empty"><h2>${t("list.empty_title")}</h2><p>${t("list.empty_text")}</p><button class="btn primary" data-act="add">${t("header.add_game")}</button></li>`;
    return;
  }
  if (!shown.length) {
    list.innerHTML = `<li class="empty"><p>${esc(t("list.no_match", { q: query }))}</p></li>`;
    return;
  }
  list.innerHTML = shown.map((g) => `
    <li class="tile" data-game="${esc(g.name)}" tabindex="0" title="${esc(g.name)}">
      ${g.running ? `<span class="dot"></span>` : ""}
      ${iconBox(g)}
      <span class="name">${esc(g.name)}</span>
      ${gameMeta(g)}
    </li>`).join("");
  loadIcons(shown);
}

view.addEventListener("click", (e) => {
  const row = e.target.closest(".tile");
  if (row) renderGame(row.dataset.game);
  if (e.target.closest("[data-act=add]")) openAddDialog();
});
view.addEventListener("keydown", (e) => {
  const row = e.target.closest(".tile");
  if (row && e.key === "Enter") renderGame(row.dataset.game);
});

// ---------- game detail ----------

async function renderGame(name, silent = false) {
  const detail = await (silent ? call("game", { game: name }).catch(() => null) : attempt(() => call("game", { game: name })));
  if (!detail) {
    if (silent) showList();
    return;
  }
  if (silent && route.name !== "game") return;
  route = { name: "game", game: name };
  gameSnapshot = snapshotOf(name);
  const g = detail.info;
  const problem = problemOf(g.name);
  const summary = g.last_backup
    ? `${g.local ? "" : t("game.not_installed")}${t("game.summary", { when: ago(g.last_backup), files: fileCount(g.files), size: bytes(g.size) })}`
    : t("game.no_backups_yet");
  const removeLabel = g.custom ? t("game.delete") : t("game.remove");

  const backups = detail.backups.length
    ? detail.backups.map((b) => `
        <li class="backup">
          <span class="text">
            <span class="when">${stamp(b.time)}</span>
            <span class="size">${esc(lookup("backup.reason.", b.reason, b.reason))} · ${fileCount(b.files)} · ${bytes(b.size)}${b.suspect ? ` · <span class="bad">${esc(t("backup.suspect", { why: lookup("backup.suspect.", b.suspect, b.suspect) }))}</span>` : ""}</span>
          </span>
          <button class="btn" data-restore="${esc(b.id)}" ${g.running ? `disabled title="${esc(t("backup.restore_disabled"))}"` : ""}>${t("backup.restore")}</button>
        </li>`).join("")
    : `<li class="none">${t("game.no_backups")}</li>`;

  view.innerHTML = `
    <div class="topbar">
      <button class="btn quiet" data-nav="list">${t("nav.back")}</button>
      <button class="btn icon quiet danger" data-game-act="remove" title="${esc(removeLabel)}" aria-label="${esc(removeLabel)}"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h18"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6M14 11v6"/><path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/></svg></button>
    </div>
    <div class="title-row">
      ${iconBox(g, true)}
      <h1>${esc(g.name)}</h1>
      ${g.running ? `<span class="pill">${t("game.playing")}</span>` : ""}
    </div>
    <p class="sub">${esc(summary)}</p>
    ${problem ? `<p class="game-error" title="${esc(problem.detail)}">${esc(problem.code === "suspect_save" ? t("game.suspect") : t("game.error", { text: problemText(problem) }))}</p>` : ""}
    <div class="row-actions">
      ${g.local ? `<button class="btn primary" data-game-act="backup">${t("game.backup_now")}</button>` : ""}
      <button class="btn${g.local ? "" : " primary"}" data-game-act="open-backups" title="${esc(t("game.backups_title"))}">${t("game.backups_btn")}</button>
      ${g.local ? `<button class="btn" data-game-act="open-saves" title="${esc(t("game.saves_title"))}">${t("game.saves_btn")}</button>` : ""}
    </div>
    <h3>${t("game.backups_heading")}</h3>
    <ul class="backups">${backups}</ul>
    ${g.local ? `<button class="fold" id="fold-locations" aria-expanded="${locationsOpen}">
      <span>${t("game.locations_heading")}</span>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6"/></svg>
    </button>
    <ul class="paths" id="locations" ${locationsOpen ? "" : "hidden"}>${detail.locations.map((l) => `<li>${esc(l)}</li>`).join("")}</ul>` : ""}`;
  $("#fold-locations")?.addEventListener("click", (e) => {
    locationsOpen = !locationsOpen;
    e.currentTarget.setAttribute("aria-expanded", locationsOpen);
    $("#locations").hidden = !locationsOpen;
  });

  loadIcons([g]);
  view.querySelector("[data-game-act=backup]")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    btn.disabled = true;
    btn.classList.add("busy");
    btn.textContent = t("game.backing_up");
    const res = await attempt(() => call("backup_now", { game: name }));
    if (res) toast(t(`backup.result.${res.code}`));
    await refresh();
    renderGame(name, true);
  });
  view.querySelector("[data-game-act=open-backups]").addEventListener("click", () => attempt(() => call("open_backups", { game: name })));
  view.querySelector("[data-game-act=open-saves]")?.addEventListener("click", () => attempt(() => call("open_saves", { game: name })));
  view.querySelector("[data-game-act=remove]").addEventListener("click", async () => {
    const ok = await confirmAction(
      g.custom ? t("remove.title_custom") : t("remove.title_list"),
      g.custom ? t("remove.text_custom") : t("remove.text_list"),
      t("remove.confirm"));
    if (!ok) return;
    if (await attempt(() => call("remove_game", { game: name })) !== undefined) {
      await refresh();
      showList();
    }
  });
  view.querySelectorAll("[data-restore]").forEach((btn) => btn.addEventListener("click", async () => {
    const backup = detail.backups.find((b) => b.id === btn.dataset.restore);
    // Tell the user what is about to be overwritten before anything is touched.
    const preview = await attempt(() => call("restore_preview", { game: name, backup: backup.id }));
    if (!preview) return;
    const lines = [t("restore.text", { when: stamp(backup.time) }), t("restore.files", { files: fileCount(preview.files) })];
    if (preview.newer > 0) lines.push({ text: tn("restore.newer", preview.newer), warn: true });
    if (!await confirmAction(t("restore.title"), lines, t("restore.confirm"))) return;
    btn.disabled = true;
    btn.classList.add("busy");
    btn.textContent = t("backup.restoring");
    if (await attempt(() => call("restore", { game: name, backup: backup.id })) !== undefined) toast(t("restore.done"));
    await refresh();
    renderGame(name, true);
  }));
}

// ---------- settings ----------

function renderSettings() {
  route = { name: "settings" };
  const s = status;
  const languages = [{ code: "auto", name: t("settings.lang_auto") }, ...LANGUAGES];
  view.innerHTML = `
    <div class="topbar">
      <button class="btn quiet" data-nav="list">${t("nav.back")}</button>
    </div>
    <div class="settings">
      <h1>${t("settings.title")}</h1>
      <div class="setting">
        <div class="setting-head">
          <div><b>${t("settings.backup_dir")}</b><span class="value">${esc(s.backup_dir)}</span></div>
          <button class="btn" id="change-dir">${t("settings.change")}</button>
        </div>
        <small>${t("settings.cloud_hint")}</small>
      </div>
      <div class="setting">
        <div class="setting-head">
          <b>${t("settings.keep")}</b>
          <input class="num" id="keep" type="number" min="1" max="1000" value="${s.keep}">
        </div>
        <small>${t("settings.keep_hint")}</small>
      </div>
      <div class="setting">
        <div class="setting-head">
          <b>${t("settings.autostart")}</b>
          <label class="switch"><input type="checkbox" id="autostart" ${s.autostart ? "checked" : ""}><span></span></label>
        </div>
      </div>
      <div class="setting">
        <div class="setting-head">
          <b>${t("settings.tray")}</b>
          <label class="switch"><input type="checkbox" id="show-tray" ${s.show_tray ? "checked" : ""}><span></span></label>
        </div>
      </div>
      <div class="setting">
        <div class="setting-head">
          <b>${t("settings.language")}</b>
          <select class="select" id="lang">${languages.map((l) => `<option value="${l.code}" ${l.code === s.lang ? "selected" : ""}>${esc(l.name)}</option>`).join("")}</select>
        </div>
      </div>
      ${s.hidden ? `<div class="setting">
        <div class="setting-head">
          <div><b>${t("settings.hidden")}</b><span class="value">${tn("settings.hidden_count", s.hidden)}</span></div>
          <button class="btn" id="unhide">${t("settings.unhide")}</button>
        </div>
      </div>` : ""}
      <div class="setting danger-zone">
        <div class="setting-head">
          <b>${t("settings.quit")}</b>
          <button class="btn danger" id="quit">${t("settings.quit_btn")}</button>
        </div>
      </div>
      <p class="version">Savepoint ${esc(s.version)}</p>
    </div>`;

  $("#change-dir").addEventListener("click", async () => {
    const dir = await attempt(() => invoke("pick_folder", { title: t("settings.backup_dir") }));
    if (!dir) return;
    if (await attempt(() => call("set_settings", { backup_dir: dir })) !== undefined) {
      toast(t("settings.backup_dir_changed"));
      await refresh();
      renderSettings();
    }
  });
  $("#keep").addEventListener("change", async (e) => {
    const keep = Math.max(1, Math.min(1000, parseInt(e.target.value, 10) || 10));
    e.target.value = keep;
    if (await attempt(() => call("set_settings", { keep })) !== undefined) toast(t("settings.saved"));
  });
  $("#autostart").addEventListener("change", async (e) => {
    if (await attempt(() => call("set_settings", { autostart: e.target.checked })) === undefined) e.target.checked = !e.target.checked;
  });
  $("#show-tray").addEventListener("change", async (e) => {
    if (await attempt(() => call("set_settings", { show_tray: e.target.checked })) === undefined) e.target.checked = !e.target.checked;
  });
  $("#lang").addEventListener("change", async (e) => {
    if (await attempt(() => call("set_settings", { lang: e.target.value })) === undefined) return;
    await refresh();
    toast(t("settings.saved"));
  });
  $("#quit").addEventListener("click", async () => {
    const ok = await confirmAction(t("quit.title"), t("quit.text"), t("settings.quit_btn"));
    if (ok) await call("quit").catch(() => {});
  });
  $("#unhide")?.addEventListener("click", renderHidden);
}

// ---------- hidden games ----------

function renderHidden() {
  route = { name: "hidden" };
  const games = [...status.hidden_games].sort((a, b) => a.localeCompare(b, i18n.code));
  view.innerHTML = `
    <div class="topbar">
      <button class="btn quiet" data-nav="settings">${t("nav.back_settings")}</button>
      ${games.length > 1 ? `<button class="btn" id="unhide-all">${t("hidden.restore_all")}</button>` : ""}
    </div>
    <div class="settings">
      <h1>${t("settings.hidden")}</h1>
      ${games.length ? `<ul class="backups">${games.map((name) => `
        <li class="backup">
          <span class="text"><span class="when">${esc(name)}</span></span>
          <button class="btn" data-unhide="${esc(name)}">${t("hidden.restore")}</button>
        </li>`).join("")}</ul>` : `<p class="sub">${t("hidden.empty")}</p>`}
    </div>`;
  $("#unhide-all")?.addEventListener("click", async () => {
    await attempt(() => call("reset_hidden"));
    await refresh();
    renderSettings();
  });
  view.querySelectorAll("[data-unhide]").forEach((btn) => btn.addEventListener("click", async () => {
    if (await attempt(() => call("unhide", { game: btn.dataset.unhide })) === undefined) return;
    toast(t("hidden.restored", { game: btn.dataset.unhide }));
    await refresh();
    renderHidden();
  }));
}

// ---------- add game ----------

const addDialog = $("#add-dialog");
const addForm = $("#add-form");

function openAddDialog() {
  addForm.reset();
  $(".form-error", addForm).hidden = true;
  addDialog.showModal();
}

addForm.addEventListener("click", async (e) => {
  if (e.target.closest("[data-close]")) addDialog.close();
  const pick = e.target.closest("[data-pick]");
  if (!pick) return;
  const isExe = pick.dataset.pick === "exe";
  const value = await attempt(() => invoke(isExe ? "pick_exe" : "pick_folder", { title: t(isExe ? "add.pick_exe" : "add.pick_folder") }));
  if (!value) return;
  addForm.elements[isExe ? "exe" : "path"].value = value;
  if (isExe && !addForm.elements.name.value) {
    addForm.elements.name.value = (value.split(/[\\/]/).pop() ?? "").replace(/\.exe$/i, "");
  }
  if (!isExe && !addForm.elements.name.value) {
    addForm.elements.name.value = value.split(/[\\/]/).filter(Boolean).pop() ?? "";
  }
});

addForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = addForm.elements;
  const error = $(".form-error", addForm);
  if (!f.path.value) {
    error.textContent = t("add.no_folder");
    error.hidden = false;
    return;
  }
  try {
    await call("add_game", { name: f.name.value, path: f.path.value, exe: f.exe.value || null });
  } catch (err) {
    error.textContent = errText(err);
    error.hidden = false;
    return;
  }
  addDialog.close();
  toast(t("add.added"));
  await refresh();
  renderGame(f.name.value.trim());
});

// ---------- navigation ----------

document.addEventListener("click", (e) => {
  const nav = e.target.closest("[data-nav]");
  if (!nav) return;
  if (nav.dataset.nav === "settings" && status) renderSettings();
  else showList();
});
$("#add-game").addEventListener("click", openAddDialog);
$("#close-app").addEventListener("click", () => invoke("close_window"));
$("#open-root").addEventListener("click", () => attempt(() => call("open_backups", { game: null })));
$("#alert-open").addEventListener("click", () => {
  const game = $("#alert").dataset.game;
  if (game) renderGame(game);
});
$("#alert-close").addEventListener("click", async () => {
  await call("clear_errors", { game: null }).catch(() => {});
  await refresh();
});
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || document.querySelector("dialog[open]")) return;
  if (route.name !== "list") showList();
  else invoke("close_window");
});
document.addEventListener("contextmenu", (e) => {
  if (!e.target.closest("input")) e.preventDefault();
});

// ---------- start ----------

(async function start() {
  // Paint in the last used language right away; the service has the final say.
  try { langPref = localStorage.getItem(LANG_KEY) || "auto"; } catch { /* no cache */ }
  await setLanguage(langPref);
  showList();
  await call("ui_opened").catch(() => {});
  await refresh();
  schedule();
})();
