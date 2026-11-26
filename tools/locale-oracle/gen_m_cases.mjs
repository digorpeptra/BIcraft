// Generates crates/locale/tests/oracle/m_cases.json: Power Query (M) text ↔ number/date
// conversions per culture, for the Power BI Desktop recording (maintainers' development
// oracles) and the bicraft-data test that compares its M evaluator with Desktop's results.
//   node tools/locale-oracle/gen_m_cases.mjs > crates/locale/tests/oracle/m_cases.json
//
// Each probe is a function of the culture name `c` returning text; numbers are written back
// with Number.ToText(x, "R", "en-US") and dates as yyyy-MM-dd, so only the conversion under
// test depends on the culture.
import fs from "node:fs";
const n = (e) => `Number.ToText(${e}, "R", "en-US")`;
const d = (e) => `Date.ToText(${e}, "yyyy-MM-dd", "en-US")`;
const D = "#date(2024,3,5)";
const DT = "#datetime(2024,3,5,14,7,9)";
const T = "#time(9,5,3)";
const probes = [
  ["n2", 'Number.ToText(1234567.891, "N2", c)'],
  ["n0neg", 'Number.ToText(-1234.5, "N0", c)'],
  ["f1", 'Number.ToText(1234.56, "F1", c)'],
  ["p1", 'Number.ToText(0.256, "P1", c)'],
  ["c2", 'Number.ToText(1234.5, "C", c)'],
  ["cneg", 'Number.ToText(-1234.5, "C", c)'],
  ["g", 'Number.ToText(1234.5, "G", c)'],
  ["e2", 'Number.ToText(12345.678, "E2", c)'],
  ["custom", 'Number.ToText(1234567.891, "#,##0.00", c)'],
  ["textnum", "Text.From(1234.5, c)"],
  ["textdate", `Text.From(${D}, c)`],
  ["textdt", `Text.From(${DT}, c)`],
  ["texttime", `Text.From(${T}, c)`],
  ["d_long", `Date.ToText(${D}, "D", c)`],
  ["d_short", `Date.ToText(${D}, "d", c)`],
  ["d_monthday", `Date.ToText(${D}, "M", c)`],
  ["d_yearmonth", `Date.ToText(${D}, "Y", c)`],
  ["dMMM", `Date.ToText(${D}, "MMM", c)`],
  ["dMMMM", `Date.ToText(${D}, "MMMM", c)`],
  ["ddMMMM", `Date.ToText(${D}, "d MMMM", c)`],
  ["dddd", `Date.ToText(${D}, "dddd", c)`],
  ["dddabbr", `Date.ToText(${D}, "ddd", c)`],
  ["dcustom", `Date.ToText(${D}, "dddd d MMMM yyyy", c)`],
  ["dt_general", `DateTime.ToText(${DT}, "G", c)`],
  ["dt_short", `DateTime.ToText(${DT}, "g", c)`],
  ["dt_full", `DateTime.ToText(${DT}, "f", c)`],
  ["t_long", `Time.ToText(${T}, "T", c)`],
  ["t_short", `Time.ToText(${T}, "t", c)`],
  ["tpm", 'Time.ToText(#time(14,7,9), "T", c)'],
  ["monthname", `Date.MonthName(${D}, c)`],
  ["dayname", `Date.DayOfWeekName(${D}, c)`],
  ["p_space", n('Number.FromText("1 234,5", c)')],
  ["p_nnbsp", n('Number.FromText("1 234,5", c)')],
  ["p_nbsp", n('Number.FromText("1 234,5", c)')],
  ["p_dotcomma", n('Number.FromText("1.234,5", c)')],
  ["p_commadot", n('Number.FromText("1,234.5", c)')],
  ["p_dollar", n('Number.FromText("$2.675", c)')],
  ["p_dollar2", n('Number.FromText("$1,234.50", c)')],
  ["p_euro", n('Number.FromText("2,675 €", c)')],
  ["p_euro2", n('Number.FromText("1.234,50 €", c)')],
  ["p_euro3", n('Number.FromText("€1,234.50", c)')],
  ["p_pound", n('Number.FromText("£1,234.50", c)')],
  ["p_real", n('Number.FromText("R$ 1.234,50", c)')],
  ["p_yen", n('Number.FromText("¥1,235", c)')],
  ["p_pct", n('Number.FromText("12,5 %", c)')],
  ["p_neg", n('Number.FromText("(5)", c)')],
  ["p_type", n('Value.FromText("2,5", c)')],
  ["p_cur", n('Currency.From("$2.675", c)')],
  ["p_cur2", n('Currency.From("2,675 €", c)')],
  ["p_d1", d('Date.FromText("05/03/2024", c)')],
  ["p_d2", d('Date.FromText("5.3.2024", c)')],
  ["p_d3", d('Date.FromText("5 mars 2024", c)')],
  ["p_d4", d('Date.FromText("March 5, 2024", c)')],
  ["p_d5", d('Date.FromText("5 März 2024", c)')],
  ["p_d6", d('Date.FromText("2024-03-05", c)')],
  ["p_d7", d('Date.FromText("5-3-2024", c)')],
  ["p_d8", d('Date.FromText("2024/03/05", c)')],
];
// Every month and weekday name, plain and after a day number (genitive).
for (let m = 1; m <= 12; m++) {
  const day = `#date(2024,${m},7)`;
  probes.push([`m${m}_full`, `Date.ToText(${day}, "MMMM", c)`], [`m${m}_abbr`, `Date.ToText(${day}, "MMM", c)`],
    [`m${m}_gen`, `Date.ToText(${day}, "d MMMM", c)`], [`m${m}_genabbr`, `Date.ToText(${day}, "d MMM", c)`]);
}
for (let w = 0; w < 7; w++) {
  const day = `#date(2024,3,${3 + w})`;
  probes.push([`w${w}_full`, `Date.ToText(${day}, "dddd", c)`], [`w${w}_abbr`, `Date.ToText(${day}, "ddd", c)`]);
}
// The cultures of the FORMAT corpus.
const cultures = JSON.parse(fs.readFileSync(new URL("../../crates/locale/tests/oracle/cases.json", import.meta.url), "utf8")).cultures;
// Conversions without a culture argument follow the model culture (fr-FR in the harness).
const defaults = [
  ["def_textnum", "Text.From(1234.5)"],
  ["def_numtext", 'Number.ToText(1234.5, "N2")'],
  ["def_textdate", `Text.From(${D})`],
  ["def_fromtext", n('Number.FromText("1 234,5")')],
  ["def_datefrom", d('Date.FromText("05/03/2024")')],
];
console.log(JSON.stringify({ model_culture: "fr-FR", cultures, probes, defaults }, null, 1));
