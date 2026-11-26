// Highlight probe (docs/interaction-oracle.md, "Highlight probe"): which built-in visual types
// Power BI Desktop highlights, rather than filters, when a data point of another visual is
// selected. Windows only, development aid.
//   node highlight_probe.mjs gen DIR MODEL.bim   -> DIR\{model.bim, cmds1.json, cmds2.json, plan.json}
//   bicraft run --file DIR\cmds1.json; bicraft run --file DIR\cmds2.json   -> DIR\interactions.pbix
//   powershell -File xtask\interaction-oracle\record.ps1 -Dir DIR -Settle 10000
//   node highlight_probe.mjs verdicts DIR > verdicts.json
// One page holds a source bar chart (Product[Category]) and one target per type, each with a
// measure of its own (`HL_<type>` = [Total Amount]) so that its query is recognisable in the
// engine trace. After a click on Toys in the source, a target whose query computes its measure a
// second time under KEEPFILTERS of the selection's filter table is highlighted; one whose query
// filters on the selection (TREATAS) is filtered; a target that sends no query is unknown.
// MODEL.bim is the interaction oracle's model (testsupport/interaction_oracle/model.bim).
import { readFileSync, writeFileSync, copyFileSync } from 'node:fs';
import { join } from 'node:path';

const [mode, dir, model] = process.argv.slice(2);
const ST = 'Store[Store]', COL = 'Product[Color]', MON = 'Date[Month]';
const TARGETS = [
  ['clusteredColumnChart', m => [['Category', ST], ['Y', m]]],
  ['columnChart', m => [['Category', ST], ['Y', m]]],
  ['hundredPercentStackedColumnChart', m => [['Category', ST], ['Y', m]]],
  ['clusteredBarChart', m => [['Category', ST], ['Y', m]]],
  ['barChart', m => [['Category', ST], ['Y', m]]],
  ['hundredPercentStackedBarChart', m => [['Category', ST], ['Y', m]]],
  ['lineChart', m => [['Category', ST], ['Y', m]]],
  ['areaChart', m => [['Category', ST], ['Y', m]]],
  ['stackedAreaChart', m => [['Category', ST], ['Y', m]]],
  ['hundredPercentStackedAreaChart', m => [['Category', ST], ['Y', m]]],
  ['lineClusteredColumnComboChart', m => [['Category', ST], ['Y', m], ['Y2', '[Units]']]],
  ['lineStackedColumnComboChart', m => [['Category', ST], ['Y', m], ['Y2', '[Units]']]],
  ['ribbonChart', m => [['Category', ST], ['Y', m]]],
  ['waterfallChart', m => [['Category', ST], ['Y', m]]],
  ['funnel', m => [['Category', ST], ['Y', m]]],
  ['pieChart', m => [['Category', COL], ['Y', m]]],
  ['donutChart', m => [['Category', COL], ['Y', m]]],
  ['treemap', m => [['Group', ST], ['Values', m]]],
  ['scatterChart', m => [['Category', ST], ['X', '[Units]'], ['Y', m]]],
  ['card', m => [['Values', m]]],
  ['multiRowCard', m => [['Values', m]]],
  ['cardVisual', m => [['Data', m]]],
  ['kpi', m => [['Indicator', m], ['TrendLine', MON]]],
  ['gauge', m => [['Y', m]]],
  ['tableEx', m => [['Values', ST], ['Values', m]]],
  ['pivotTable', m => [['Rows', ST], ['Values', m]]],
  ['decompositionTreeVisual', m => [['Analyze', m], ['ExplainBy', ST]]],
  ['map', m => [['Category', ST], ['Size', m]]],
  ['filledMap', m => [['Category', ST], ['Y', m]]],
];

function gen() {
  copyFileSync(model, join(dir, 'model.bim'));
  const cmds = [{ command: 'file.open', params: { path: join(dir, 'model.bim') } }];
  const visuals = [];
  const W = 260, H = 220, X0 = 330, COLS = 6;
  TARGETS.forEach(([ty, f], i) => {
    const name = `HL_${ty}`;
    cmds.push({ command: 'measure.add', params: { table: 'Sales', name, expression: '[Total Amount]' } });
    visuals.push({ name: `t_${i}`, type: ty, x: X0 + (i % COLS) * (W + 10), y: 10 + Math.floor(i / COLS) * (H + 10), width: W, height: H,
      fields: f(`[${name}]`).map(([role, field]) => ({ role, field })) });
  });
  visuals.push({ name: 'src', type: 'clusteredBarChart', x: 10, y: 10, width: 310, height: 300,
    fields: [{ role: 'Category', field: 'Product[Category]' }, { role: 'Y', field: '[Total Amount]' }] });
  const height = 10 + Math.ceil(TARGETS.length / COLS) * (H + 10);
  cmds.push({ command: 'report.new', params: {} });
  cmds.push({ command: 'report.apply', params: { spec: { pages: [
    { key: 'P0', display_name: 'P0', width: 1920, height, visuals: [] },
    { key: 'H', display_name: 'H', width: 1920, height, visuals },
  ] } } });
  cmds.push({ command: 'project.new', params: { dir: join(dir, 'proj'), name: 'Interactions' } });
  writeFileSync(join(dir, 'cmds1.json'), JSON.stringify(cmds, null, 1));
  writeFileSync(join(dir, 'cmds2.json'), JSON.stringify([
    { command: 'file.open', params: { path: join(dir, 'proj', 'Interactions.pbip') } },
    { command: 'file.save_as', params: { path: join(dir, 'interactions.pbix'), overwrite: true } },
  ], null, 1));
  const pv = visuals.map(v => ({ name: v.name, key: v.name, type: v.type, x: v.x, y: v.y, width: v.width, height: v.height, fields: v.fields.map(f => [f.role, f.field]) }));
  writeFileSync(join(dir, 'plan.json'), JSON.stringify({ version: 1, pbix: 'interactions.pbix', width: 1920, height, scenarios: [
    { page: 'P0', title: 'start', hidden: false, visuals: [], steps: [] },
    { page: 'H', title: 'highlight probe', hidden: false, visuals: pv, steps: [
      { id: 'H.file', note: 'as opened', actions: [] },
      { id: 'H.toys', note: 'click Toys in the source bar chart', actions: [{ do: 'click', visual: 'src', point: 'Toys' }] },
    ] },
  ] }, null, 1));
}

function verdicts() {
  const raw = join(dir, 'raw');
  const plan = JSON.parse(readFileSync(join(dir, 'plan.json'), 'utf8'));
  const rd = JSON.parse(readFileSync(join(raw, 'readings.json'), 'utf8'));
  const inst = JSON.parse(readFileSync(join(raw, 'instance.json'), 'utf8').replace(/^﻿/, ''));
  const trace = readFileSync(join(raw, 'trace.jsonl'), 'utf8').split('\n').filter(Boolean).map(l => JSON.parse(l))
    .filter(e => e.event === 'QueryBegin' && e.subclass === 'DAXQuery');
  const step = rd.steps.find(s => s.id === 'H.toys');
  const t0 = Date.parse(step.actions[0].at), t1 = Date.parse(step.end);
  const types = {};
  for (const v of plan.scenarios.find(s => s.page === 'H').visuals.filter(v => v.name !== 'src')) {
    const name = v.fields.map(f => f[1]).find(f => f.startsWith('[HL_')).slice(1, -1);
    const qs = trace.filter(e => { const t = Date.parse(e.start); return t >= t0 && t <= t1 && e.text.includes(`[${name}]`); });
    const hl = qs.some(q => new RegExp(`CALCULATE\\(\\s*'Sales'\\[${name}\\]\\s*,\\s*KEEPFILTERS\\(__\\w+\\)`).test(q.text));
    const filt = qs.some(q => /TREATAS\(\{"Toys"\}, 'Product'\[Category\]\)/.test(q.text));
    types[v.type] = qs.length === 0 ? 'unknown' : hl ? 'highlight' : filt ? 'filter' : 'unknown';
  }
  console.log(JSON.stringify({ desktop: inst.desktop, ui_culture: inst.ui_culture, recorded: inst.recorded, click_ok: step.actions[0].ok, types }, null, 1));
}

if (mode === 'gen' && dir && model) gen();
else if (mode === 'verdicts' && dir) verdicts();
else { console.error('usage: node highlight_probe.mjs gen DIR MODEL.bim | verdicts DIR'); process.exit(2); }
