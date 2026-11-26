// Generates crates/locale/src/data.rs, the culture table of bicraft-locale.
//
//   node tools/locale-oracle/gen_data.mjs            (writes the file, prints a report)
//
// Three sources, none of them a Microsoft file:
// - Unicode CLDR, as shipped in the ICU data of the Node.js that runs this script (read through
//   `Intl`; the CLDR and ICU versions are written into the generated file). CLDR gives the month
//   and day names Power Query uses (.NET), the separators and percent pattern Desktop's visuals
//   use, the first day of the week, and the display-unit suffixes of cultures Power BI does not
//   abbreviate its own way.
// - What the Analysis Services engine of Power BI Desktop returned for our own FORMAT corpus
//   (crates/locale/tests/oracle/expected.json, recorded with the development oracles): the
//   number separators, currency layout, date/time patterns, AM/PM designators and the names
//   DAX FORMAT prints. Where the engine differs from CLDR (e.g. fr-FR "janv" vs "janv.", es-ES
//   grouping of 4-digit numbers), the engine wins for DAX and Power Query, and the report lists
//   the difference.
// - What Power Query in Desktop returned for our M corpus (tests/oracle/m_expected.json,
//   recorded with the development oracles): the month and day names, month-day and
//   year-month patterns and percent layout .NET formats with in Power Query.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const exp = JSON.parse(fs.readFileSync(path.join(repo, "crates/locale/tests/oracle/expected.json"), "utf8"));
const cultures = exp.cultures;
const report = [];
const warn = (c, m) => report.push(`${c}: ${m}`);

// ---------------------------------------------------------------------------------------------
// CLDR through Intl.

const UTC = { timeZone: "UTC" };
const day = (y, m, d) => new Date(Date.UTC(y, m - 1, d));
function part(loc, opts, date, type) {
  const p = new Intl.DateTimeFormat(loc, { ...UTC, ...opts }).formatToParts(date).find((x) => x.type === type);
  return p ? p.value : "";
}
function cldr(loc) {
  const months = [], monthsGen = [], monthsAbbr = [], monthsAbbrGen = [], days = [], daysAbbr = [];
  for (let m = 1; m <= 12; m++) {
    const d = day(2024, m, 7);
    months.push(part(loc, { month: "long" }, d, "month"));
    monthsGen.push(part(loc, { day: "numeric", month: "long" }, d, "month"));
    // Some cultures write the month as a number next to a day: take the stand-alone form then.
    let abbr = part(loc, { day: "numeric", month: "short" }, d, "month");
    if (/^[0-9]+$/.test(abbr)) abbr = part(loc, { month: "short" }, d, "month");
    monthsAbbr.push(abbr);
    monthsAbbrGen.push(abbr);
  }
  for (let w = 0; w < 7; w++) {
    const d = day(2024, 3, 3 + w); // 2024-03-03 is a Sunday
    days.push(part(loc, { weekday: "long" }, d, "weekday"));
    daysAbbr.push(part(loc, { weekday: "short" }, d, "weekday"));
  }
  const nf = new Intl.NumberFormat(loc);
  const p5 = nf.formatToParts(12345.5);
  const group = (p5.find((x) => x.type === "group") || { value: "" }).value;
  const decimal = (p5.find((x) => x.type === "decimal") || { value: "." }).value;
  const minGrouping = nf.formatToParts(1234).some((x) => x.type === "group") ? 1 : 2;
  const pp = new Intl.NumberFormat(loc, { style: "percent" }).formatToParts(0.5);
  const pi = pp.findIndex((x) => x.type === "integer");
  const percent = {
    prefix: pp.slice(0, pi).map((x) => x.value).join(""),
    suffix: pp.slice(pi + 1).map((x) => x.value).join(""),
  };
  const units = [1e3, 1e6, 1e9, 1e12].map((x) =>
    new Intl.NumberFormat(loc, { notation: "compact" })
      .formatToParts(x * 1.5)
      .filter((p) => !["integer", "decimal", "fraction", "minusSign"].includes(p.type))
      .map((p) => p.value)
      .join(""),
  );
  let firstDay = 0;
  try {
    const fd = new Intl.Locale(loc).getWeekInfo().firstDay; // 1 = Monday … 7 = Sunday
    firstDay = fd % 7;
  } catch { /* old ICU */ }
  return { months, monthsGen, monthsAbbr, monthsAbbrGen, days, daysAbbr, group, decimal, minGrouping, percent, units, firstDay };
}

// ---------------------------------------------------------------------------------------------
// The engine's results.

const E = (id, i) => {
  const v = exp.format[id];
  if (!v) throw new Error(`missing case ${id}`);
  return v[i];
};
const esc = (s) => s.replace(/'/g, "''");

// Splits `s` into tokens: the longest of `words` (text → pattern token) at each position, digit
// runs through `numbers` (digits → token), other text as literals.
function tokenize(s, words, numbers) {
  const out = [];
  let lit = "";
  let i = 0;
  const flush = () => { if (lit) { out.push({ lit }); lit = ""; } };
  outer: while (i < s.length) {
    if (/[0-9]/.test(s[i])) {
      let j = i;
      while (j < s.length && /[0-9]/.test(s[j])) j++;
      const run = s.slice(i, j);
      if (numbers[run] !== undefined) { flush(); out.push({ tok: numbers[run] }); i = j; continue; }
      lit += run; i = j; continue;
    }
    let best = null;
    for (const [w, t] of words) {
      if (w && s.startsWith(w, i) && (!best || w.length > best[0].length)) best = [w, t];
    }
    if (best) {
      // a name must not be the start of a longer word ("mar" in "mars")
      const next = s[i + best[0].length];
      if (!(next && /[\p{Lu}\p{Ll}]/u.test(next) && /[\p{Lu}\p{Ll}]$/u.test(best[0]))) {
        flush(); out.push({ tok: best[1] }); i += best[0].length; continue outer;
      }
    }
    lit += s[i]; i++;
  }
  flush();
  return out;
}
// A .NET-style pattern: separators equal to the culture's are `/` and `:`, other literals quoted.
function pattern(tokens, dateSep, timeSep) {
  return tokens.map((t) => {
    if (t.tok) return t.tok;
    let out = "";
    let pending = "";
    const flush = () => { if (pending) { out += `'${esc(pending)}'`; pending = ""; } };
    for (let k = 0; k < t.lit.length; ) {
      if (dateSep && t.lit.startsWith(dateSep, k) && !/^[ ]+$/.test(dateSep)) { flush(); out += "/"; k += dateSep.length; continue; }
      if (timeSep && t.lit.startsWith(timeSep, k)) { flush(); out += ":"; k += timeSep.length; continue; }
      pending += t.lit[k]; k++;
    }
    flush();
    return out;
  }).join("");
}

// Renders a pattern for the checks below (the subset of .NET tokens the derived patterns use).
function render(p, v, names) {
  let out = "";
  for (let i = 0; i < p.length; ) {
    const c = p[i];
    if (c === "'") {
      let j = i + 1;
      while (j < p.length && !(p[j] === "'" && p[j + 1] !== "'")) { if (p[j] === "'") j++; out += p[j]; j++; }
      i = j + 1; continue;
    }
    let n = 1;
    while (p[i + n] === c) n++;
    const pad = (x, w) => String(x).padStart(w, "0");
    const h12 = v.H % 12 === 0 ? 12 : v.H % 12;
    switch (c) {
      case "d": out += n === 1 ? v.d : n === 2 ? pad(v.d, 2) : n === 3 ? names.daysAbbr[v.dow] : names.days[v.dow]; break;
      case "M": out += n === 1 ? v.M : n === 2 ? pad(v.M, 2) : n === 3 ? names.monthsAbbr[v.M - 1] : names.months[v.M - 1]; break;
      case "y": out += n <= 2 ? pad(v.y % 100, 2) : pad(v.y, 4); break;
      case "H": out += n === 1 ? v.H : pad(v.H, 2); break;
      case "h": out += n === 1 ? h12 : pad(h12, 2); break;
      case "m": out += n === 1 ? v.m : pad(v.m, 2); break;
      case "s": out += n === 1 ? v.s : pad(v.s, 2); break;
      case "t": out += v.H < 12 ? names.am : names.pm; break;
      case "/": out += names.dateSep; break;
      case ":": out += names.timeSep; break;
      default: out += c.repeat(n);
    }
    i += n;
  }
  return out;
}

function engineProfile(c, i) {
  const r = {};
  // Numbers.
  const g = E("n5_f7", i); // 1234.5 as "#,0" → "1<group>235"
  r.group = g.length === 5 ? g[1] : "";
  const dec = E("n5_f9", i); // "1234<dec>50"
  r.decimal = dec[dec.length - 3];
  const groupDigits = (s) => s.replace(/\B(?=(\d{3})+(?!\d))/g, r.group);
  const cur0 = E("n9_f2", i); // 3 → "3,00 €"
  const m = cur0.match(/3(?:\D(\d+))?/);
  r.currencyDigits = m && m[1] ? m[1].length : 0;
  const amount = (x) => {
    const [a, b] = x.toFixed(r.currencyDigits).split(".");
    return groupDigits(a) + (b ? r.decimal + b : "");
  };
  const split = (s, digits) => {
    const k = s.indexOf(digits);
    if (k < 0) { warn(c, `currency layout not found in ${JSON.stringify(s)}`); return { prefix: "", suffix: "" }; }
    return { prefix: s.slice(0, k), suffix: s.slice(k + digits.length) };
  };
  r.currencyPositive = split(E("n1_f2", i), amount(1234567.891));
  r.currencyNegative = split(E("n2_f2", i), amount(1234.5));
  r.currencySymbol = (r.currencyPositive.prefix + r.currencyPositive.suffix).replace(/[\s\u00a0\u202f()-]/g, "");
  // Words of Yes/No, True/False, On/Off.
  r.words = [E("n1_f30", i), E("n4_f30", i), E("n1_f31", i), E("n4_f31", i), E("n1_f32", i), E("n4_f32", i)];
  // Separators.
  const ds = E("d1_g12", i); // "05<sep>03<sep>2024"
  r.dateSep = ds.slice(2, ds.indexOf("03", 2));
  const ts = E("d1_g13", i); // "14<sep>07<sep>09"
  r.timeSep = ts.slice(2, ts.indexOf("07", 2));
  // Names as FORMAT prints them.
  r.months = [], r.monthsAbbr = [], r.monthsGen = [], r.days = [], r.daysAbbr = [];
  for (let k = 1; k <= 12; k++) {
    r.months.push(E(`m${k}_w`, i));
    r.monthsAbbr.push(E(`m${k}_a`, i));
    r.monthsGen.push(E(`m${k}_d`, i).replace(/^\d+ /, ""));
  }
  for (let k = 0; k < 7; k++) {
    r.days.push(E(`w${k}_w`, i));
    r.daysAbbr.push(E(`w${k}_a`, i));
  }
  // AM/PM designators: Medium Time of 09:05 and 14:07 without the digits.
  const designator = (s, digits) => s.replace(digits, "").trim();
  r.am = designator(E("d4_g6", i), `09${r.timeSep}05`);
  r.pm = designator(E("d1_g6", i), `02${r.timeSep}07`);
  // First day of the week from FORMAT(Tuesday, "w").
  r.firstDay = (((2 - (Number(E("d1_g35", i)) - 1)) % 7) + 7) % 7;
  // Week 1 of "ww", from 1 January 2021 (a Friday): in week 1 → the week of 1 January; in
  // week 53 → the first week with four days; else the first full week.
  const wk1 = E("wk1_w", i);
  r.firstWeek = wk1 === "1" ? 0 : wk1 === "53" ? 2 : 1;
  return r;
}

const DATE_NUM = { "2024": "yyyy", "24": "yy", "05": "dd", "5": "d", "03": "MM", "3": "M" };

function derive(c, i, names, natNames) {
  const r = names;
  const nameWords = (src) => [
    ...src.days.map((w) => [w, "dddd"]),
    ...src.months.map((w) => [w, "MMMM"]),
    ...src.monthsGen.map((w) => [w, "MMMM"]),
  ];
  // Month names of Medium Date (ru "май" where "mmm" alone gives "Май"; zh-TW "三月"): the part
  // that varies between the twelve months.
  r.monthsAbbrGen = (r.englishNames ? natNames.monthsAbbr : r.monthsAbbr).slice();
  {
    const all = [];
    for (let k = 1; k <= 12; k++) all.push(E(`m${k}_m`, i).replace(k === 2 ? "29" : "\u0000", "07"));
    let pre = 0;
    while (all.every((x) => x[pre] !== undefined && x[pre] === all[0][pre])) pre++;
    let suf = 0;
    while (all.every((x) => x.length - suf - 1 >= pre && x[x.length - suf - 1] === all[0][all[0].length - suf - 1])) suf++;
    const mid = all.map((x) => x.slice(pre, x.length - suf));
    if (mid.every((x) => x && !/[0-9]/.test(x))) {
      // a shared trailing letter or dot belongs to the names ("janv", "févr" … all end differently)
      r.monthsAbbrGen = mid;
    }
  }
  const longSrc = r.englishNames ? natNames : r;
  // Patterns are derived from 5 March 2024 (a Tuesday), 14:07:09 and 09:05:03.
  r.shortDate = pattern(tokenize(E("d1_g4", i), [], DATE_NUM), r.dateSep, null);
  r.longDate = pattern(tokenize(E("d1_g2", i), nameWords(longSrc), DATE_NUM), null, null);
  r.mediumDate = pattern(
    tokenize(E("d1_g3", i), [...r.monthsAbbrGen.map((w) => [w, "MMM"]), ...nameWords(longSrc)], DATE_NUM),
    null, null,
  );
  const twelve = !/14/.test(E("d1_g5", i));
  const timeNum = (h) => ({ [twelve ? "09" : "09"]: twelve ? "hh" : "HH", "9": twelve ? "h" : "H", "05": "mm", "03": "ss", ...(h || {}) });
  const des = r.am ? [[r.am, "tt"]] : [];
  r.longTime = pattern(tokenize(E("d4_g5", i), des, timeNum()), null, r.timeSep);
  r.mediumTime = pattern(tokenize(E("d4_g6", i), des, { "09": "hh", "9": "h", "05": "mm" }), null, r.timeSep);
  r.shortTime = r.longTime.replace(/:ss/, "");
  // Checks: the patterns must give back every recorded long/medium/short date and time.
  const v = (y, M, d, H, m, s) => ({ y, M, d, H, m, s, dow: day(y, M, d).getUTCDay() });
  const rn = { ...longSrc, am: r.am, pm: r.pm, dateSep: r.dateSep, timeSep: r.timeSep };
  const rm = { ...rn, monthsAbbr: r.monthsAbbrGen };
  const checks = [
    ["d1_g4", r.shortDate, v(2024, 3, 5, 14, 7, 9)],
    ["d2_g4", r.shortDate, v(2024, 11, 25, 9, 5, 3)],
    ["d1_g2", r.longDate, v(2024, 3, 5, 14, 7, 9)],
    ["d2_g2", r.longDate, v(2024, 11, 25, 9, 5, 3)],
    ["d1_g3", r.mediumDate, v(2024, 3, 5, 14, 7, 9)],
    ["d1_g5", r.longTime, v(2024, 3, 5, 14, 7, 9)],
    ["t0_0", r.longTime, v(1899, 12, 30, 0, 5, 0)],
    ["t12_0", r.longTime, v(1899, 12, 30, 12, 0, 0)],
    ["d1_g6", r.mediumTime, v(2024, 3, 5, 14, 7, 9)],
    ["t0_1", r.mediumTime, v(1899, 12, 30, 0, 5, 0)],
  ];
  for (let k = 1; k <= 12; k++) checks.push([`m${k}_l`, r.longDate, v(2024, k, k === 2 ? 29 : 7, 0, 0, 0)], [`m${k}_m`, r.mediumDate, v(2024, k, k === 2 ? 29 : 7, 0, 0, 0)]);
  for (let k = 0; k < 7; k++) checks.push([`w${k}_l`, r.longDate, v(2024, 3, 3 + k, 0, 0, 0)]);
  for (const [id, p, val] of checks) {
    const got = render(p, val, id.endsWith("_m") || id === "d1_g3" ? rm : rn);
    if (got !== E(id, i)) warn(c, `${id}: pattern ${JSON.stringify(p)} gives ${JSON.stringify(got)}, engine ${JSON.stringify(E(id, i))}`);
  }
}

// Power BI's own display-unit suffixes (what Desktop's visuals print: "0,3 M" in fr-FR, "1bn" in
// en-US), as the viewer and renderer show them; other languages take CLDR's short compact forms
// when CLDR abbreviates by thousands, else the English ones.
const NB = "\u00a0";
const PBI_UNITS = {
  en: ["K", "M", "bn", "T"],
  fr: [NB + "k", NB + "M", NB + "Md", NB + "Bn"],
  de: [NB + "Tsd.", NB + "Mio.", NB + "Mrd.", NB + "Bio."],
  es: [NB + "mil", NB + "M", NB + "mil" + NB + "M", NB + "B"],
  it: [NB + "k", NB + "Mln", NB + "Mld", NB + "Bln"],
  nl: ["K", NB + "mln.", NB + "mld.", NB + "bln."],
  pt: [NB + "mil", NB + "mi", NB + "bi", NB + "tri"],
};

// ---------------------------------------------------------------------------------------------
// What Power Query in Desktop returned (tests/oracle/m_expected.json):
// the names and patterns .NET formats with.

const mexp = JSON.parse(fs.readFileSync(path.join(repo, "crates/locale/tests/oracle/m_expected.json"), "utf8"));
function mProfile(c, cl, r) {
  const k = mexp.cultures.indexOf(c);
  const M = (id) => (k >= 0 && mexp.results[id] ? mexp.results[id][k] : null);
  const ok = (v) => typeof v === "string" && v !== "#ERR" && v !== "";
  const list = (n, f, fallback) => {
    const out = [];
    for (let j = 0; j < n; j++) {
      const v = f(j);
      out.push(ok(v) ? v : fallback[j]);
    }
    return out;
  };
  const strip = (v) => (ok(v) ? v.replace(/^7 /, "") : null);
  const m = {
    months: list(12, (j) => M(`m${j + 1}_full`), cl.months),
    monthsGen: list(12, (j) => strip(M(`m${j + 1}_gen`)), cl.monthsGen),
    monthsAbbr: list(12, (j) => M(`m${j + 1}_abbr`), cl.monthsAbbr),
    monthsAbbrGen: list(12, (j) => strip(M(`m${j + 1}_genabbr`)), cl.monthsAbbr),
    days: list(7, (j) => M(`w${j}_full`), cl.days),
    daysAbbr: list(7, (j) => M(`w${j}_abbr`), cl.daysAbbr),
  };
  const words = [...m.monthsGen.map((w) => [w, "MMMM"]), ...m.months.map((w) => [w, "MMMM"])];
  const md = M("d_monthday");
  m.monthDay = ok(md) ? pattern(tokenize(md, words, { "05": "dd", "5": "d", "03": "MM", "3": "M" }), null, null) : null;
  // Without a day the names are nominative (ca "de març de 2024": "de " is literal).
  const ym = M("d_yearmonth");
  const nominative = m.months.map((w) => [w, "MMMM"]);
  m.yearMonth = ok(ym) ? pattern(tokenize(ym, nominative, { "2024": "yyyy", "03": "MM", "3": "M" }), null, null) : null;
  // "C" of 1234.5 and -1234.5: the layout around "1<group>234<decimal><digits>".
  const cur = (s) => {
    const x = ok(s) ? s.match(/1([^0-9]?)23[45](?:([^0-9])([0-9]+))?/) : null;
    if (!x) return null;
    return { prefix: s.slice(0, x.index), suffix: s.slice(x.index + x[0].length), group: x[1] || r.group || " ", decimal: x[2] || r.decimal, digits: x[3] ? x[3].length : 0 };
  };
  const cp = cur(M("c2")), cn = cur(M("cneg"));
  m.currency = cp && cn
    ? { symbol: (cp.prefix + cp.suffix).replace(/[\s\u00a0\u202f()-]/g, ""), positive: { prefix: cp.prefix, suffix: cp.suffix }, negative: { prefix: cn.prefix, suffix: cn.suffix }, digits: cp.digits, decimal: cp.decimal, group: cp.group }
    : { symbol: r.currencySymbol, positive: r.currencyPositive, negative: r.currencyNegative, digits: r.currencyDigits, decimal: r.decimal, group: r.group || " " };
  // "N2" of 1234567.891: Indian grouping is "12<g>34<g>567".
  const n2 = M("n2");
  m.indian = ok(n2) && /^12[^0-9]34[^0-9]567/.test(n2);
  // "P1" of 0.256: the text around "25<decimal>6".
  const p1 = M("p1");
  const num = `25${r.decimal}6`;
  m.percent = ok(p1) && p1.includes(num)
    ? { prefix: p1.slice(0, p1.indexOf(num)), suffix: p1.slice(p1.indexOf(num) + num.length) }
    : { prefix: "", suffix: r.decimal === "," ? " %" : "%" };
  if (k < 0) warn(c, "no Power Query results: CLDR names");
  return m;
}

const rows = [];
for (const [i, c] of cultures.entries()) {
  const cl = cldr(c);
  const en = cldr("en-US");
  const r = engineProfile(c, i);
  // FORMAT prints English names for custom tokens in some cultures (ja, zh, ko).
  r.englishNames = !c.startsWith("en") && r.months[2] === "March";
  derive(c, i, r, { months: cl.months, monthsGen: cl.monthsGen, monthsAbbr: cl.monthsAbbr, days: cl.days, daysAbbr: cl.daysAbbr });
  if (r.englishNames && (JSON.stringify(r.months) !== JSON.stringify(en.months))) warn(c, "English custom names differ from CLDR en");
  // Differences with CLDR, for the report.
  const diff = (what, a, b) => { if (JSON.stringify(a) !== JSON.stringify(b)) warn(c, `engine ${what} ${JSON.stringify(a)} vs CLDR ${JSON.stringify(b)}`); };
  diff("group separator", r.group, cl.group);
  diff("decimal separator", r.decimal, cl.decimal);
  if (!r.englishNames) {
    diff("month names", r.months, cl.months);
    diff("abbreviated month names", r.monthsAbbr, cl.monthsAbbr);
    diff("day names", r.days, cl.days);
    diff("abbreviated day names", r.daysAbbr, cl.daysAbbr);
  }
  diff("first day of week", r.firstDay, cl.firstDay);
  const lang = c.split("-")[0];
  let units = PBI_UNITS[lang];
  if (!units) {
    units = cl.units.every((u) => u.trim() !== "") && !["ja", "zh", "ko"].includes(lang) ? cl.units.map((u) => u.replace(/^ /, NB)) : PBI_UNITS.en;
  }
  const mp = mProfile(c, cl, r);
  diff("Power Query month names", mp.months, cl.months);
  diff("Power Query abbreviated month names", mp.monthsAbbr, cl.monthsAbbr);
  rows.push({ c, cl, r, units, mp });
}

// ---------------------------------------------------------------------------------------------
// Output.

const ch = (s) => {
  if (s.length !== 1) throw new Error(`not one char: ${JSON.stringify(s)}`);
  const cp = s.codePointAt(0);
  if (cp === 0x27) return "'\\''";
  if (cp < 0x20 || cp === 0xa0 || cp === 0x202f || cp === 0x2019) return `'\\u{${cp.toString(16)}}'`;
  return `'${s}'`;
};
const str = (s) => {
  let out = '"';
  for (const c of s) {
    const cp = c.codePointAt(0);
    if (c === '"' || c === "\\") out += "\\" + c;
    else if (cp === 0xa0 || cp === 0x202f || cp < 0x20) out += `\\u{${cp.toString(16)}}`;
    else out += c;
  }
  return out + '"';
};
const arr = (a) => `[${a.map(str).join(", ")}]`;
const affix = (a) => `Affix { prefix: ${str(a.prefix)}, suffix: ${str(a.suffix)} }`;
const currency = (c) =>
  `Currency { symbol: ${str(c.symbol)}, positive: ${affix(c.positive)}, negative: ${affix(c.negative)}, digits: ${c.digits}, decimal: ${ch(c.decimal)}, group: ${ch(c.group)} }`;

let src = `// @generated by tools/locale-oracle/gen_data.mjs; do not edit by hand.
//
// Sources: Unicode CLDR ${process.versions.cldr} (ICU ${process.versions.icu}, Unicode License v3, see SOURCE.txt) and
// the results of Power BI Desktop ${exp.engine.version}'s engine for our FORMAT corpus
// (tests/oracle/expected.json).

use crate::{Affix, Currency, Display, Locale, PowerQuery, Vb};

pub(crate) static LOCALES: &[Locale] = &[\n`;
for (const { c, cl, r, units, mp } of rows) {
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  src += `    Locale {
        name: ${str(c)},
        decimal: ${ch(r.decimal)},
        group: ${ch(r.group || " ")},
        currency: ${currency({ symbol: r.currencySymbol, positive: r.currencyPositive, negative: r.currencyNegative, digits: r.currencyDigits, decimal: r.decimal, group: r.group || " " })},
        pq: PowerQuery {
            currency: ${currency(mp.currency)},
            indian_grouping: ${mp.indian},
        },
        date_separator: ${str(r.dateSep)},
        time_separator: ${str(r.timeSep)},
        short_date: ${str(r.shortDate)},
        long_date: ${str(r.longDate)},
        long_time: ${str(r.longTime)},
        short_time: ${str(r.shortTime)},
        am: ${str(r.am)},
        pm: ${str(r.pm)},
        month_day: ${str(mp.monthDay || "MMMM d")},
        year_month: ${str(mp.yearMonth || "MMMM yyyy")},
        percent: ${affix(mp.percent)},
        months: ${arr(mp.months)},
        months_genitive: ${arr(mp.monthsGen)},
        months_abbr: ${arr(mp.monthsAbbr)},
        months_abbr_genitive: ${arr(mp.monthsAbbrGen)},
        days: ${arr(mp.days)},
        days_abbr: ${arr(mp.daysAbbr)},
        first_day: ${cl.firstDay},
        vb: Vb {
            medium_date: ${str(r.mediumDate)},
            medium_time: ${str(r.mediumTime)},
            english_names: ${r.englishNames},
            months: ${arr(r.englishNames ? cl.months : r.months)},
            months_genitive: ${arr(r.englishNames ? cl.monthsGen : r.monthsGen)},
            months_abbr: ${arr(r.englishNames ? cl.monthsAbbr : r.monthsAbbr)},
            months_abbr_genitive: ${arr(r.monthsAbbrGen)},
            days: ${arr(r.englishNames ? cl.days : r.days)},
            days_abbr: ${arr(r.englishNames ? cl.daysAbbr : r.daysAbbr)},
            first_day: ${r.firstDay},
            first_week: ${r.firstWeek},
            words: ${arr(r.words)},
        },
        display: Display {
            decimal: ${ch(cl.decimal)},
            group: ${ch(cl.group || " ")},
            min_grouping: ${cl.minGrouping},
            percent: ${affix(cl.percent)},
            units: ${arr(units)},
        },
    },\n`;
}
src += "];\n";
fs.writeFileSync(path.join(repo, "crates/locale/src/data.rs"), src);
console.log(report.join("\n"));
console.log(`\n${rows.length} cultures written (CLDR ${process.versions.cldr}, ICU ${process.versions.icu}); ${report.length} notes`);
