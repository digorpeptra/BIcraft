// Generates crates/locale/tests/oracle/cases.json, the DAX FORMAT corpus the engine results
// (expected.json) were recorded for with the maintainers' development oracles.
//   node tools/locale-oracle/gen_cases.mjs > crates/locale/tests/oracle/cases.json
const BS = "\\";
const nums = {
  n1: "1234567.891", n2: "-1234.5", n3: "0.256", n4: "0", n5: "1234.5", n6: "12345678901",
  n7: "0.000123", n8: "-0.5", n9: "3", n10: "-0.004", n11: "1E+20", n12: "123456789012345678",
};
const dates = {
  d1: "DATE(2024,3,5)+TIME(14,7,9)", d2: "DATE(2024,11,25)+TIME(9,5,3)", d3: "DATE(2024,3,5)",
  d4: "TIME(9,5,3)", d5: "DATE(2023,1,1)+TIME(0,30,0)",
};
const numFormats = ["", "General Number", "Currency", "Fixed", "Standard", "Percent", "Scientific",
  "#,0", "#,0.00", "0.00", "0.0%", "#,0.00 \"€\"", "$#,0.00;($#,0.00)", "#,0,", "#,0,,\"M\"", "0.00E+00",
  BS + "$#,0.00", "#,##0.###", "0", "00.0", "#.##", "€#,0.00", "#,0.00 €", "0 %",
  "#,0.00;-#,0.00;\"zero\"", "0.00" + BS + "%", "#,0.0,,", "\"Total: \"#,0", "# ##0,00", "0.###E-0", "Yes/No", "True/False", "On/Off"];
const dateFormats = ["", "General Date", "Long Date", "Medium Date", "Short Date", "Long Time", "Medium Time", "Short Time",
  "dddd d mmmm yyyy", "ddd dd mmm yy", "mmmm", "mmm", "dd/mm/yyyy", "hh:nn:ss", "h:nn AM/PM", "yyyy-mm-dd", "c",
  "ddddd", "dddddd", "ttttt", "mmmm yyyy", "d mmm", "hh:nn", "AM/PM", "am/pm", "a/p", "q", "yyyy" + BS + "Qq",
  "dd.mm.yy", "dddd", "ddd", "\"Q\"q yyyy", "d/m/yy", "h:n:s", "hh:nn AMPM", "w", "ww", "y", "yy", "m/d", "mmmmm",
  "dd mmmm", "d" + BS + "/m", "hh" + BS + ":nn"];
const cases = [];
for (const [k, v] of Object.entries(nums)) numFormats.forEach((f, i) => cases.push({ id: `${k}_f${i}`, value: v, format: f }));
for (const [k, v] of Object.entries(dates)) dateFormats.forEach((f, i) => cases.push({ id: `${k}_g${i}`, value: v, format: f }));
// Every month and weekday name, and the long/medium dates of a one-digit and two-digit day.
for (let m = 1; m <= 12; m++) {
  for (const [s, f] of [["w", "mmmm"], ["a", "mmm"], ["l", "Long Date"], ["m", "Medium Date"], ["d", "d mmmm"]]) {
    cases.push({ id: `m${m}_${s}`, value: `DATE(2024,${m},${m === 2 ? 29 : 7})`, format: f });
  }
}
for (let d = 0; d < 7; d++) {
  for (const [s, f] of [["w", "dddd"], ["a", "ddd"], ["l", "Long Date"]]) {
    cases.push({ id: `w${d}_${s}`, value: `DATE(2024,3,${3 + d})`, format: f });
  }
}
// AM and PM forms of the time formats.
for (const [k, v] of [["t0", "TIME(0,5,0)"], ["t12", "TIME(12,0,0)"], ["t23", "TIME(23,59,59)"]]) {
  ["Long Time", "Medium Time", "Short Time", "AM/PM", "am/pm", "General Date"].forEach((f, i) => cases.push({ id: `${k}_${i}`, value: v, format: f }));
}
// Week numbers around the new year (week rules), and rounding of halves.
for (const [k, v] of [["wk1", "DATE(2021,1,1)"], ["wk2", "DATE(2020,1,1)"], ["wk3", "DATE(2024,12,30)"], ["wk4", "DATE(2025,12,31)"], ["wk5", "DATE(2027,1,2)"]]) {
  cases.push({ id: `${k}_w`, value: v, format: "ww" }, { id: `${k}_d`, value: v, format: "w" });
}
for (const [k, v, f] of [["r1", "2.675", "0.00"], ["r2", "1.005", "0.00"], ["r3", "0.125", "0.00"], ["r4", "-2.5", "0"],
  ["r5", "0.5", "0"], ["r6", "1.45", "0.0"], ["r7", "8.345", "#,0.00"], ["r8", "1234.5", "Currency"], ["r9", "0.285", "0.0%"]]) {
  cases.push({ id: k, value: v, format: f });
}
const parse = [
  "1234.5", "1,234.5", "1.234,5", "1 234,5", "1 234,5", "1 234,5", "$1,234.50", "1.234,50 €", "€1,234.50",
  "1 234,50 €", "12%", "12 %", "(5)", "-5", "5-", "1'234.5", "R$ 1.234,50", "¥1,235", "£1,234.50", "1,5", "1.5",
  "2.675", "$2.675", "2,675", "1E3", "  7  ",
];
const dparse = ["05/03/2024", "03/05/2024", "2024-03-05", "5.3.2024", "05.03.2024", "5 mars 2024", "5 March 2024",
  "March 5, 2024", "5-Mar-2024", "5 Mär 2024", "5 de marzo de 2024", "2024/03/05", "5/3/24", "05-03-2024", "14:07",
  "2:07 PM", "05/03/2024 14:07:09"];
const words = ["a", "A", "á", "à", "â", "ä", "å", "æ", "b", "c", "ç", "ch", "cz", "d", "e", "é", "è", "f", "h", "i",
  "ı", "İ", "ij", "l", "ll", "lz", "n", "ñ", "nz", "o", "ö", "ø", "œ", "oz", "s", "ß", "ss", "sz", "t", "u", "ü", "v",
  "w", "x", "y", "z", "zz", "þ", "co-op", "coop", "côte", "coté", "cote", "côté", "Ångström", "Zebra", "über", "uber",
  "Öl", "Ol", "zz2", "aa", "ab", "ae", "ad", "oe", "ue", "ot", "ys", "č", "cx", "ř", "rz", "š", "ž", "ł", "lx", "ő",
  "ğ", "gz", "ş", "dž", "dz", "hx", "chz", "Ä", "Å", "Æ", "Ø", "Ö", "Ü", "Ñ"];
// Letters some cultures sort as their own letter (tailorings), with neighbours.
for (const w of ("ą ąb ać b c ć ćb cz d e ę ęb ez l ł łb lz m n ń ńb nz o ó ób oz p r ř řb rz s ś śb sz š šb t z ź ź ż " +
  "żb zz ž žb h ch chb hz i ı ıb I İ ib iz g ğ ğb gz ö öb oz ş şb ü üb uz y yb å åb ä äb æ æb ø øb aa aab az ő ű v w " +
  "wa va x ss ß dz dzs dż ly ny gy cs zs sz").split(" ")) {
  if (!words.includes(w)) words.push(w);
}
const cultures = ["en-US", "en-GB", "fr-FR", "de-DE", "es-ES", "it-IT", "nl-NL", "pt-BR", "ja-JP", "pt-PT", "fr-CA",
  "fr-BE", "fr-CH", "de-CH", "de-AT", "en-CA", "en-AU", "en-IN", "es-MX", "sv-SE", "da-DK", "nb-NO", "fi-FI",
  "pl-PL", "cs-CZ", "ru-RU", "tr-TR", "zh-CN", "ko-KR", "nl-BE", "el-GR", "ro-RO", "hu-HU", "sk-SK", "uk-UA",
  "ca-ES", "id-ID", "vi-VN", "lt-LT", "lv-LV", "et-EE", "hr-HR", "sl-SI", "bg-BG", "zh-TW", "en-NZ", "en-ZA",
  "en-IE", "es-AR", "es-CO", "it-CH", "de-LU", "fr-LU", "nn-NO", "is-IS", "sr-Latn-RS", "ms-MY", "en-SG", "hi-IN"];
console.log(JSON.stringify({ cultures, format: cases, value: parse, datevalue: dparse, sort: words }, null, 1));
