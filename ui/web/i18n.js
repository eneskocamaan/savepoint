"use strict";

// Translations live in lang/<code>.json (flat "key": "text" files).
//
// To add a language: copy lang/en.json to lang/<code>.json and translate it,
// then add its name to LANGUAGES below and its tray strings to
// crates/savepointd/src/i18n.rs. Missing keys fall back to English.
// Plurals use "<key>.one" / "<key>.other" (see tn); a language without
// plural forms only needs "<key>.other".

const LANGUAGES = [
  { code: "en", name: "English" },
  { code: "tr", name: "Türkçe" },
  { code: "es", name: "Español" },
];

const i18n = { code: "en", strings: {}, fallback: null, plural: new Intl.PluralRules("en") };

const loaded = new Map();
async function loadFile(code) {
  if (!loaded.has(code)) {
    loaded.set(code, fetch(`lang/${code}.json`).then((r) => r.json()).catch(() => ({})));
  }
  return loaded.get(code);
}

/** "auto" follows the system language; anything unsupported falls back to English. */
function resolveLang(pref) {
  const supported = LANGUAGES.map((l) => l.code);
  if (supported.includes(pref)) return pref;
  for (const tag of navigator.languages ?? [navigator.language]) {
    const code = String(tag).toLowerCase().split("-")[0];
    if (supported.includes(code)) return code;
  }
  return "en";
}

async function setLanguage(pref) {
  const code = resolveLang(pref);
  i18n.fallback = await loadFile("en");
  i18n.strings = code === "en" ? i18n.fallback : await loadFile(code);
  i18n.code = code;
  i18n.plural = new Intl.PluralRules(code);
  document.documentElement.lang = code;
  applyStatic();
}

function t(key, vars) {
  const text = i18n.strings[key] ?? i18n.fallback?.[key] ?? key;
  return vars ? text.replace(/\{(\w+)\}/g, (m, name) => (name in vars ? vars[name] : m)) : text;
}

/** Text for a count, e.g. tn("list.count", 3) -> "3 games". */
function tn(key, n, vars = {}) {
  const form = i18n.plural.select(n);
  const pick = [`${key}.${form}`, `${key}.other`].find((k) => k in i18n.strings || k in (i18n.fallback ?? {}));
  return t(pick ?? key, { n, ...vars });
}

/** Translates the fixed text in index.html. */
function applyStatic() {
  for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = t(el.dataset.i18n);
  for (const el of document.querySelectorAll("[data-i18n-title]")) el.title = t(el.dataset.i18nTitle);
  for (const el of document.querySelectorAll("[data-i18n-aria]")) el.setAttribute("aria-label", t(el.dataset.i18nAria));
  for (const el of document.querySelectorAll("[data-i18n-placeholder]")) el.placeholder = t(el.dataset.i18nPlaceholder);
}
