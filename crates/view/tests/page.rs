//! Static checks of the embedded viewer page (crates/view/src/page.html): it must stay one
//! self-contained, offline, XSS-safe file. The page's behaviour is checked in a browser.

const PAGE: &str = include_str!("../src/page.html");

fn script() -> &'static str {
    let start = PAGE.find("<script>").expect("inline script") + "<script>".len();
    let end = PAGE.rfind("</script>").expect("script end");
    &PAGE[start..end]
}

#[test]
fn no_html_injection_sinks() {
    for sink in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
        "new Function",
        "setTimeout('",
        "setTimeout(\"",
    ] {
        assert!(!PAGE.contains(sink), "page uses {sink}");
    }
}

#[test]
fn works_offline() {
    // The only absolute URL is the SVG namespace; no CDN, font or image is fetched. (The web
    // protocols a web-URL conditional format may link to are named once, without an address.)
    let mut rest = PAGE;
    while let Some(i) = rest.find("http") {
        let tail = &rest[i..];
        assert!(
            tail.starts_with("http://www.w3.org/2000/svg")
                || tail.starts_with("http:', 'https:']")
                || tail.starts_with("https:']"),
            "external URL in page: {}",
            &tail[..tail.len().min(60)]
        );
        rest = &tail[4..];
    }
    assert!(!PAGE.contains("<link rel=\"stylesheet\""));
    assert!(!PAGE.contains("@import"));
    assert!(!PAGE.contains("<script src"));
}

#[test]
fn storage_is_wrapped() {
    // localStorage is only touched inside the try/catch `store` helper.
    let js = script();
    let uses = js.matches("localStorage").count();
    let guarded = js.matches("try { const v = localStorage.getItem(").count()
        + js.matches("try { localStorage.setItem(").count();
    assert_eq!(uses, guarded, "unguarded localStorage access");
}

#[test]
fn has_the_rendering_mode() {
    let js = script();
    // render / wireframe toggle, remembered per viewer
    for needle in ["Rendu", "Maquette", "store.set('mode'", "aperçu sans données"] {
        assert!(js.contains(needle), "missing {needle}");
    }
    // the visual types the M8.2 rendering covers
    for ty in [
        "cardvisual",
        "multirowcard",
        "clusteredcolumnchart",
        "hundredpercentstackedbarchart",
        "linechart",
        "areachart",
        "lineclusteredcolumncombochart",
        "piechart",
        "donutchart",
        "tableex",
        "pivottable",
        "slicer",
        "textbox",
        "actionbutton",
        "gauge",
        "kpi",
    ] {
        assert!(js.contains(&format!("{ty}:")), "no renderer for {ty}");
    }
}

#[test]
fn colours_and_fonts_are_sanitised() {
    let js = script();
    // Snapshot colours go through hexColor, fonts through safeFont, before reaching a style.
    assert!(js.contains("function hexColor("));
    assert!(js.contains("function safeFont("));
    assert!(!js.contains("style.cssText"));
    assert!(!js.contains("setAttribute('style', str("));
}

#[test]
fn images_come_from_the_asset_route_only() {
    let js = script();
    // Image URLs are built in one place, from an id checked against a strict pattern.
    assert!(js.contains("const IMG_ID = /^[0-9a-f]{16,64}$/;"));
    assert!(js.contains("if (!IMG_ID.test(id)) return null;"));
    assert_eq!(js.matches("'api/asset/' + id + VQ").count(), 1);
    assert_eq!(js.matches("src: img.src").count(), 1);
    // background images are set in one helper only, from that URL
    assert_eq!(js.matches("backgroundImage").count(), 1);
    assert!(js.contains("backgroundImage: 'url(\"' + img.src + '\")'"));
    assert!(!js.contains("url(' + str("));
    // wallpaper, page background, image visuals and visual backgrounds are drawn
    for needle in [
        "function wallpaperOf(",
        "function pageImageLayer(",
        "function imageBody(",
        "imgOf(b.image)",
        "page.background_transparency",
    ] {
        assert!(js.contains(needle), "missing {needle}");
    }
}

#[test]
fn renders_the_visual_details() {
    let js = script();
    // filters, conditional formatting and the visual details of the snapshot (M8.2 fidelity)
    for needle in [
        "function smallMultiples(",
        "function buttonBody(",
        "function refValues(",
        "function valueScale(",
        "d.cell_styles",
        "f.series_colors",
        "f.column_styles",
        "f.reference_lines",
        "f.small_multiples",
        "obj(f.subtitle)",
        "v.slicer",
        "t.paragraphs",
        "d.filters_ignored_reason",
    ] {
        assert!(js.contains(needle), "missing {needle}");
    }
    // cell styles: colours through hexColor, bar extents clamped numbers
    assert!(js.contains("bg: hexColor(o.bg, null), fg: hexColor(o.fg, null), fill: hexColor(o.fill, null)"));
    assert!(js.contains("from: clamp(num(bar.from, 0), 0, 1), to: clamp(num(bar.to, 0), 0, 1)"));
    // text runs: sanitised colour and font
    assert!(js.contains("color: hexColor(r.color, null), fontFamily: fam ? safeFont(fam) : null"));
    // icons, web URLs, matrix conditional formats, trend lines
    for needle in [
        "function iconKids(",
        "function webUrl(",
        "function trendFits(",
        "function drawTrends(",
        "url: webUrl(o.url)",
        "cellsOf([], false)",
    ] {
        assert!(js.contains(needle), "missing {needle}");
    }
    // a link only ever comes from webUrl (an absolute web address), never opens with an opener
    assert_eq!(js.matches("href: ").count(), 1);
    assert!(js.contains(
        "h('a', { href: c.url, target: '_blank', rel: 'noopener noreferrer', referrerpolicy: 'no-referrer'"
    ));
    assert!(js.contains("return WEB_PROTOCOLS.includes(u.protocol) ? u.href : null;"));
    // icon names are checked before they reach an attribute
    assert!(js.contains("/^[A-Za-z0-9_-]{1,100}$/.test(str(ic.name))"));
}

#[test]
fn renders_subtitle_fonts_dividers_and_slicer_search() {
    let js = script();
    // title and subtitle fonts go through safeFont; divider colour through hexColor, its style
    // from a fixed list and its width clamped
    assert!(js.contains("function fontOf(o, title) {"));
    assert!(js.contains("font: fontOf(sub, false)"));
    assert!(js.contains("divider: { show: dv.show === true, color: hexColor(dv.color, ctx.grid), width: clamp(num(dv.width, 1), 1, 12)"));
    assert!(js.contains("['dashed', 'dotted'].includes(str(dv.style).toLowerCase())"));
    // a text slicer shows its typed text (a text node, clipped)
    assert!(js.contains("search: Array.from(str(sl.search)).slice(0, 200).join('')"));
}

#[test]
fn slicers_use_the_snapshot_settings() {
    let js = script();
    // header switch, search box, slider ends from the column's domain, relative-date inputs
    assert!(js.contains("(st.headerShow != null ? st.headerShow : !F.titled) && header"));
    assert!(js.contains("if (st.searchBox || st.search) {"));
    assert!(js.contains("own(dom, 'min') && dom.min != null ? dom.min : sorted.length ? sorted[0] : null"));
    assert!(js.contains("isObj(st.range) && own(rg, k) && rg.filter_only !== true"));
    assert!(
        js.contains("Number.isFinite(ro.duration) ? String(Math.round(clamp(ro.duration, -1e9, 1e9))) : '1'")
    );
}

#[test]
fn slicers_and_buttons_follow_the_renderer() {
    // crates/render/src/slicers.rs and bodies.rs, drawn by hand at the renderer's positions
    let js = script();
    let body = function(js, "slicerBody");
    for needle in [
        "isObj(st.range) || (numeric && vals.length > 1 && b.h >= 60) ? 'between' : b.h < 50 ? 'dropdown' : 'list'",
        "slText(P, 9, 13.7, fitText(header, Math.max(b.w - 9, 0), hpx), hpx, p.header);",
        "if (!rc.fmt && /double|decimal|currency/i.test(rc.type)) rc.fmt = '#,0.00';",
        "lower.endsWith(' date') || lower.endsWith(' time') || isDateFmt(rc.fmt)",
        "top = 16.3;",
    ] {
        assert!(body.contains(needle), "slicerBody lacks {needle}");
    }
    let list = function(js, "slList");
    for needle in [
        "bx = 7.7, bs = 13.3, pitch = 19",
        "let y = top + 5.3;",
        "hierarchy ? bx + 43.3 : bx + 22.7",
        "on === 0 ? 0 : on >= vals.length ? 1 : 2",
    ] {
        assert!(list.contains(needle), "slList lacks {needle}");
    }
    assert!(function(js, "slTiles").contains("Math.max(Math.floor((x1 - x0) / 60), 1)"));
    assert!(
        function(js, "slDropdown")
            .contains("x0 = 5.7, y0 = top + slInset(top), bw = Math.max(b.w - 10.7, 20), bh = 26.6")
    );
    let range = function(js, "slRange");
    for needle in [
        "Math.max(need, 72.7)",
        "const ty = top + (top > 0 ? 48 : 46.3);",
        "pRR(P, ha - 2.25, ty - 7.5, 4.5, 15, 2.2, SLC.item)",
    ] {
        assert!(range.contains(needle), "slRange lacks {needle}");
    }
    let rel = function(js, "slRelative");
    assert!(rel.contains("[x0 + total * 0.51, total * 0.49, period, true]"));
    assert!(rel.contains("if (y1 + 12 <= b.h && (!set || covered))"));
    assert!(function(js, "periodWord").contains("SL_PERIOD_ONE"));
    assert!(function(js, "newListSlicer").contains("radio = st.single !== false, pitch = 34, d = 16"));
    assert!(function(js, "buttonSlicer").contains("Math.max(Math.floor((b.w + gap) / 100), 1)"));
    assert!(function(js, "textSlicer").contains("w.enter + ' ' + name"));
    // words in the UI language: the snapshot's strings first, then fr / en
    assert!(function(js, "slWords").contains("typeof st[k] === 'string' && st[k] ? clip(st[k], 200)"));
    // buttons: the format's fill and text, icon beside the text; Desktop's blank button otherwise
    let b = function(js, "buttonBody");
    for needle in [
        "obj(fo.button_fill)",
        "Math.min(box.h * 0.6, Math.max(px * 1.2, size * 1.1))",
        "size * 0.4",
        "str(b.icon_placement) === 'right'",
    ] {
        assert!(b.contains(needle), "buttonBody lacks {needle}");
    }
    assert!(function(js, "buttonFallback").contains("'#666666', 2"));
    assert!(js.contains("body = isObj(v.button) ? buttonBody(v, box, ctx) : buttonFallback(v, box, ctx);"));
    // the empty-visual prompt sits where the renderer puts it in the visual
    assert!(function(js, "blankVisual").contains("tx(34 - ox, 22 - oy,"));
}

#[test]
fn column_formats_follow_the_renderer() {
    // crates/render/src/data.rs and fmt.rs: column format strings on columns and their implicit
    // measures (not counts), two decimals for decimal columns, named date formats
    let js = script();
    assert!(js.contains("if (str(o.format_string)) colFmts.set("));
    let fc = function(js, "fieldCol");
    assert!(fc.contains(
        "kind === 'column' || (kind === 'aggregation' && !/count/i.test(fn)) ? str(ctx.colFmts.get("
    ));
    assert!(function(js, "dataCol").contains("engineAggName(x) === name"));
    assert!(function(js, "fmtValU").contains("/double/i.test(col.type)) s = numText(ctx, n, 2, 2, true);"));
    let nd = function(js, "namedDate");
    assert!(nd.contains("netDate(dt, p[0], ctx) + ' ' + netDate(dt, p[2], ctx)"));
    assert!(function(js, "fmtCell").contains("namedDate(d, col.fmt, ctx)"));
    // the culture patterns come from bicraft-locale's data (en-US and fr-FR checked here)
    assert!(js.contains(r#"'en-US': ["M/d/yyyy", "dddd', 'MMMM' 'd', 'yyyy", "h:mm:ss' 'tt""#));
    assert!(js.contains(r#"'fr-FR': ["dd/MM/yyyy", "dddd' 'd' 'MMMM' 'yyyy", "HH:mm:ss""#));
    // the KPI goal keeps its own column's format
    assert!(function(js, "kpiChart").contains("fmtVal(goal, gcol, ctx, !gcol.fmt)"));
}

#[test]
fn numbers_and_chart_text_follow_desktop() {
    let js = script();
    // Desktop 2.158 (and the PNG renderer's fmt.rs): the culture gives only the separators;
    // display units stay K/M/bn/T, `%` sits where the format string puts it, no Intl percent
    assert!(js.contains("const UNITS = ['K', 'M', 'bn', 'T'];"));
    assert!(!js.contains("style: 'percent', maximumFractionDigits"));
    assert!(
        js.contains("numText(ctx, n * 100, min, max, p ? p.group : true) + (p && p.pctSp ? NB : '') + '%'")
    );
    assert!(js.contains(r".replace(/[\u202f\u2009]/g, NB)"));
    assert!(js.contains("return n > 0 && s && p.plus ? '+' + s : s;"));
    // auto axis units from 10 000 (K) and 100 000 (M)
    assert!(js.contains(": m >= 1e11 ? 4 : m >= 1e8 ? 3 : m >= 1e5 ? 2 : m >= 1e4 ? 1 : 0;"));
    // date values of a measure with a date format string
    assert!(js.contains("function isDateFmt(") && js.contains("function vbDate("));
    assert!(js.contains("const t = ok && isDateFmt(col.fmt) ? vbDate(d, str(col.fmt), ctx) : null;"));
    // chart text at 9 pt (8 pt classic) whatever the theme says; tables keep the label size
    assert!(js.contains("chartPx: (classic ? 8 : 9) * PT,"));
    assert!(js.contains("'font-size': rd(ctx.chartPx)"));
    assert!(js.contains("const px = lay.fontPt != null ? lay.fontPt * PT : ctx.px"));
    // category labels: gap of 0.4 font sizes, 90° below 1.9 font sizes (continuous axes label
    // their ticks instead)
    assert!(js.contains("fitW = step - px * 0.4;"));
    assert!(!js.contains("mode: 'skip'"));
    assert!(js.contains("const deg = w < px * 1.9 ? 90 : 35"));
}

/// The body of the page's function `name` (from its declaration to the next top-level `}`).
fn function(js: &str, name: &str) -> String {
    let start = js
        .find(&format!("\nfunction {name}("))
        .unwrap_or_else(|| panic!("no function {name}"));
    let body = &js[start + 1..];
    let end = body.find("\n}\n").unwrap_or_else(|| panic!("{name} has no end"));
    body[..end + 2].to_string()
}

#[test]
fn value_axes_follow_the_renderer() {
    // crates/render/src/axes.rs `scale`: d3 nice domain with 3, 5 or 8 intervals by axis length,
    // d3 v4 increments for 2, 3 or 5 intervals, the axis' own start and end, log axes apart
    let js = script();
    let scale = function(js, "axisScale");
    for needle in [
        "if (ax && ax.log) return valueScale(lo, hi, 4, ax);",
        "const m = vertical ? (len < 150 ? 3 : len < 300 ? 5 : 8) : (len < 300 ? 3 : len < 500 ? 5 : 8);",
        "const sd = d3Step(hi - lo, m);",
        "const a = ax && ax.min != null ? lo : Math.floor(lo / sd + 1e-9) * sd;",
        "let b = ax && ax.max != null ? hi : Math.ceil(hi / sd - 1e-9) * sd;",
        "const st = Math.max(d3Increment(b - a, m < 4 ? 2 : m < 6 ? 3 : 5), sd);",
    ] {
        assert!(scale.contains(needle), "axisScale lacks {needle}");
    }
    let step = function(js, "d3Step");
    assert!(step.contains(
        "if (err <= 0.15) step *= 10; else if (err <= 0.35) step *= 5; else if (err <= 0.75) step *= 2;"
    ));
    let inc = function(js, "d3Increment");
    assert!(inc.contains("e >= Math.sqrt(50) ? 10 : e >= Math.sqrt(10) ? 5 : e >= Math.SQRT2 ? 2 : 1"));
    // every cartesian chart takes its ticks from the plot length, none from a tick count
    for chart in ["barChart", "lineChart", "waterfallChart"] {
        let f = function(js, chart);
        assert!(f.contains("axisScale("), "{chart} does not use axisScale");
        assert!(
            !f.contains("valueScale(") && !f.contains("niceTicks("),
            "{chart} uses its own ticks"
        );
    }
    // bands: d3 rangeBands with a 0.4 outer padding, Desktop's minimum category width and scroll bar
    assert!(js.contains("const OUTER_PAD = 0.4, INNER_PAD = 0.2, BAR_SLOT = 0.966, MIN_CATEGORY = 20, MAX_MARGIN = 0.25, SCROLL = 9;"));
    assert!(function(js, "mkBands").contains("Math.floor(len / minW - pad) - 1"));
}

#[test]
fn data_labels_follow_the_renderer() {
    // crates/render/src/axes.rs `label_text` / fmt.rs `fmt_label`: auto labels take the value
    // axis' unit with two decimals; `labels.display_units` / `precision` and per-series
    // `series_labels` win; 100 % charts write shares with two decimals ("19,60%")
    let js = script();
    let text = function(js, "labelText");
    assert!(
        text.contains("return unit > 1 ? fmtLabel(x, col, ctx, unit, 2) : fmtLabel(x, col, ctx, 1, null);")
    );
    assert!(function(js, "labelStyles").contains("e.column === se.j"));
    let bar = function(js, "barChart");
    assert!(bar.contains(
        "const p = st.precision != null ? st.precision : 2; return pctText(ctx, val(j, i) || 0, p, p);"
    ));
    // positions: outside end by default (inside centre when stacked), inside, base, centre
    assert!(bar.contains("autoPos = o.stacked ? 'inCenter' : 'out'"));
    for pos in [
        "insideend: 'inEnd'",
        "insidecenter: 'inCenter'",
        "insidebase: 'inBase'",
        "outsideend: 'out'",
    ] {
        assert!(js.contains(pos), "label position {pos}");
    }
    // inside labels stay legible; backgrounds are drawn behind the text
    assert!(js.contains(
        "const insideColor = (set, bar) => set && Math.abs(lum(set) - lum(bar)) >= 0.35 ? set : onColor(bar);"
    ));
    assert!(function(js, "drawLabels").contains("rx: 3, fill: l.bg.c"));
    // line labels sit 9 px over their point (plus half the marker)
    assert!(function(js, "lineChart").contains("y: Y(x) - 9 - (ls.marker || 0) * 0.5"));
}

#[test]
fn date_axes_read_as_months() {
    // first-of-month dates of a categorical date axis read "Jan 2025" (the culture's month names)
    let js = script();
    let cd = function(js, "chartData");
    assert!(cd.contains("const monthCat = ci >= 0 && /date|time/i.test(cols[ci].type);"));
    assert!(cd.contains("mo && mo[3] === '01' ? monthName(clamp(+mo[2] - 1, 0, 11), ctx) + ' ' + mo[1]"));
}

#[test]
fn continuous_axes_follow_the_renderer() {
    // crates/render/src/timeaxis.rs: a single date or number category column is continuous
    // unless set Categorical; a number level of a drill hierarchy stays categorical
    let js = script();
    let vals = function(js, "contValues");
    for needle in [
        "if (F.x.cont === false || cd.ci < 0 || cd.raw.length < 2 || cols.filter(c => c.rc === 'cat').length > 1) return null;",
        "if (drill || col.kind !== 'column' || !NUM_T.test(col.type)) return null;",
    ] {
        assert!(vals.contains(needle), "contValues lacks {needle}");
    }
    // start / end widen the axis and drop the categories outside it
    let ax = function(js, "contAxis");
    assert!(ax.contains(
        "const keep = v.t.map(x => (a == null || x >= a - 1e-9) && (b == null || x <= b + 1e-9));"
    ));
    assert!(
        ax.contains("const lo = a != null ? Math.min(a, d0) : d0, hi = b != null ? Math.max(b, d1) : d1;")
    );
    assert!(js.contains("minDate: dateBound(a.min), maxDate: dateBound(a.max) };"));
    // columns as wide as the smallest gap less the inner padding, each at its value
    assert!(function(js, "contScale").contains("(ca.hi - ca.lo + k * ca.gap)"));
    // ticks: the finest round step (dates: minutes to years; numbers: 1, 2, 5 × 10^n, whole
    // numbers at least 1) whose labels keep their room
    let ticks = function(js, "contTicks");
    assert!(ticks.contains("for (const m of [1, 2, 5])"));
    assert!(ticks.contains(
        "if ((ca.int && step < 1) || span / step > MAX_TICKS || !Number.isFinite(step)) continue;"
    ));
    assert!(ticks.contains("if (dd < ca.gap * 0.9 || (tb - ta) / dd > MAX_TICKS) continue;"));
    // the charts place categories at their values; lines join them in value order
    let line = function(js, "lineChart");
    assert!(
        line.contains("const X = i => cont ? l + tm[0] + (ca.at(i) - ca.lo) * tm[1] : l + bands.mid(i);")
    );
    assert!(line.contains("for (const i of order) {"));
    let bar = function(js, "barChart");
    assert!(bar.contains("bands = ca ? contBands(ca, r - l, ip) : mkBands(n, r - l, ip, minw);"));
    assert!(bar.contains("const bands = ca ? contBands(ca, bb - t0, ip) : mkBands(n, bb - t0, ip, minw)"));
    for chart in ["barChart", "lineChart"] {
        assert!(
            function(js, chart).contains("contAxis(cd, ds.cols, F, v && v.drill)"),
            "{chart}"
        );
    }
    assert!(!js.contains("function dateBands("));
}

#[test]
fn visual_font_sizes_reach_every_cartesian_chart() {
    let js = script();
    // horizontal bars: categories in the x axis' size, cut to a quarter of the width, never
    // wrapped nor turned; values in the y axis' size
    let bar = function(js, "barChart");
    assert!(bar.contains("const px = ctx.chartPx, cpx = F.x.px || px, vpx = F.y.px || px;"));
    assert!(bar.contains("box.w * (F.x.maxMargin != null ? F.x.maxMargin : MAX_MARGIN)"));
    assert!(bar.contains("fitText(cd.cats[i] || '', Math.max(cw - 4, cpx), cpx)"));
    let wf = function(js, "waterfallChart");
    assert!(wf.contains("const vpx = F.y.px || px, cpx = F.x.px || px"));
    assert!(wf.contains("catLabels(names, bands.step, cpx, ch, F)"));
    let sc = function(js, "scatterChart");
    assert!(sc.contains("const xpx = F.x.px || px, ypx = F.y.px || px"));
    // text is measured in the report font, like the renderer measures its fonts
    assert!(function(js, "tw").contains("TW_CTX.measureText("));
}

#[test]
fn ampm_uses_the_culture_designators() {
    // the `AMPM` token prints the culture's designators (empty in 24-hour cultures); the page's
    // table must list exactly bicraft-locale's cultures and designators
    let js = script();
    assert!(function(js, "vbDate").contains("out += ampmOf(ctx.culture)[pm ? 1 : 0];"));
    let start = js.find("const AMPM = {\n").expect("AMPM table") + "const AMPM = {\n".len();
    let table = &js[start..start + js[start..].find("\n};").expect("table end")];
    let page: Vec<(String, String, String)> = table
        .split("], ")
        .flat_map(|e| e.split("],\n"))
        .map(|e| {
            let parts: Vec<&str> = e.split('\'').collect();
            let unescape = |s: &str| s.replace("\\u202f", "\u{202f}");
            (parts[1].to_string(), unescape(parts[3]), unescape(parts[5]))
        })
        .collect();
    let data = include_str!("../../locale/src/data.rs");
    let mut locale = Vec::new();
    let mut name = None;
    let mut am = None;
    for line in data.lines().map(str::trim) {
        let val = |l: &str, key: &str| {
            l.strip_prefix(key)
                .and_then(|r| r.strip_suffix("\","))
                .map(|r| r.replace("\\u{202f}", "\u{202f}"))
        };
        if let Some(n) = val(line, "name: \"") {
            name = Some(n);
            am = None;
        } else if let (Some(_), Some(a)) = (&name, val(line, "am: \"")) {
            am = Some(a);
        } else if let (Some(n), Some(a), Some(p)) = (&name, &am, val(line, "pm: \"")) {
            locale.push((n.clone(), a.clone(), p));
            name = None;
        }
    }
    assert!(locale.len() > 50, "parsed {} cultures", locale.len());
    assert_eq!(page, locale);
}

#[test]
fn date_patterns_match_bicraft_locale() {
    // the page's DPAT table copies bicraft-locale's short date, long date and long time per
    // culture (its last two columns are the fixed Medium Date / Medium Time patterns); it must
    // list exactly the same cultures and patterns
    let js = script();
    let start = js.find("const DPAT = {\n").expect("DPAT table") + "const DPAT = {\n".len();
    let table = &js[start..start + js[start..].find("\n};").expect("table end")];
    let unescape = |s: &str| s.replace("\\u202f", "\u{202f}").replace("\\u{202f}", "\u{202f}");
    let page: Vec<[String; 4]> = table
        .lines()
        .map(|line| {
            let (name, rest) = line.trim().split_once("': [").expect("entry");
            let pats: Vec<String> = rest
                .trim_end_matches("],")
                .trim_end_matches(']')
                .split("\", \"")
                .map(|p| unescape(p.trim_matches('"')))
                .collect();
            assert_eq!(pats.len(), 5, "{line}");
            [
                name.trim_start_matches('\'').to_string(),
                pats[0].clone(),
                pats[1].clone(),
                pats[2].clone(),
            ]
        })
        .collect();
    let data = include_str!("../../locale/src/data.rs");
    let mut locale = Vec::new();
    let mut cur: Option<[String; 4]> = None;
    for line in data.lines().map(str::trim) {
        let val = |key: &str| {
            line.strip_prefix(key)
                .and_then(|r| r.strip_suffix("\","))
                .map(|r| unescape(&r.replace("\\\"", "\"")))
        };
        if let Some(n) = val("name: \"") {
            cur = Some([n, String::new(), String::new(), String::new()]);
        } else if let Some(c) = cur.as_mut() {
            for (i, key) in ["short_date: \"", "long_date: \"", "long_time: \""]
                .iter()
                .enumerate()
            {
                if let Some(v) = val(key) {
                    c[i + 1] = v;
                    if i == 2 {
                        locale.extend(cur.take());
                    }
                    break;
                }
            }
        }
    }
    assert!(locale.len() > 50, "parsed {} cultures", locale.len());
    assert_eq!(page, locale);
}

#[test]
fn classic_look_follows_the_renderer() {
    // crates/render/src/data.rs / lib.rs: a report without a theme gets Desktop's classic
    // palette, light grey regular 12 px titles, #777 text, #EAEAEA gridlines, 5 px chart padding
    // and the plot 4 px under the title
    let js = script();
    assert!(js.contains("const CLASSIC_COLORS = ['#01b8aa', '#374649', '#fd625e', '#f2c80f',"));
    assert!(js.contains("const CLASSIC_TITLE = '#c8c8c8', CLASSIC_TITLE_PX = 12"));
    let ctx = function(js, "renderCtx");
    assert!(ctx.contains("colors.length ? colors : classic ? CLASSIC_COLORS : DEFAULT_COLORS"));
    assert!(ctx.contains("muted: classic ? '#777777'") && ctx.contains("grid: classic ? '#eaeaea'"));
    let fmt = function(js, "visFormat");
    assert!(fmt.contains("(ctx.classic ? CLASSIC_TITLE_PX : 14)"));
    assert!(fmt.contains("{ fontWeight: 400 }"));
    let b = function(js, "buildRendered");
    assert!(b.contains("CHARTS.has(kind) ? (ctx.classic ? 5 : 8)"));
    assert!(b.contains("Math.round(F.title.size * 1.42)"));
    assert!(b.contains("top + (ctx.classic && CHARTS.has(kind) ? 4 : 8)"));
    assert!(b.contains("clamp(num(PD[k], 0), 0, 200)"));
}

#[test]
fn part_charts_follow_the_renderer() {
    // crates/render/src/parts.rs: 12 px DIN labels (Segoe UI standing in at 0.92), Desktop's
    // radius, label styles, inside / outside labels, funnel rates and treemap squarify
    let js = script();
    assert!(
        PAGE.contains("@font-face { font-family: \"pbc-din\"; src: local(\"Segoe UI\"); size-adjust: 92%; }")
    );
    assert!(js.contains("const LABEL_PX = 12, DIN_STAND_IN = 0.92;"));
    assert!(js.contains("fam = attr('font-family')") && js.contains("fontFamily: fam,"));
    let pie = function(js, "pieChart");
    for needle in [
        "2 + 1 / (1 + Math.exp(-5 * (vh / Math.max(vw, 1) - 1)))",
        "/ denom * 0.8",
        "pie.inner_radius / 100",
        "F.legend.show === true ? placeLegend(",
        "(ctx.classic ? '' : 'Category, percent of total')",
        "['inside', 'insidecenter', 'preferinside']",
        "cy + si * r * 1.13",
        "p2[0] + dir * 13",
        "const lineC = ctx.classic ? '#666666' : ctx.muted;",
    ] {
        assert!(pie.contains(needle), "pieChart lacks {needle}");
    }
    let st = function(js, "sliceText");
    assert!(st.contains(
        "case 'category, data value, percent of total': return [x.name, val() + ' (' + pct() + ')'];"
    ));
    let fun = function(js, "funnelChart");
    for needle in [
        "Math.max(vh / (n + 2), 22.7)",
        "gh = band * 0.89",
        "pctText(ctx, last / first, 0, 1)",
        "x0 + (wAll + w) / 2 + 8",
    ] {
        assert!(fun.contains(needle), "funnelChart lacks {needle}");
    }
    assert!(function(js, "treemapChart").contains("squarify("));
    assert!(function(js, "squarify").contains("const phi = 0.5 * (1 + Math.sqrt(5))"));
}

#[test]
fn small_multiples_reindex_series_settings() {
    // a panel drops the split column: per-series labels and lines move down with their columns
    let sm = function(script(), "smallMultiples");
    assert!(sm.contains("filter(e => isObj(e) && e.column !== ci).map(e => Number.isInteger(e.column) ? Object.assign({}, e, { column: at(e.column) }) : e)"));
    assert!(
        sm.contains("series_labels: reCol(F.raw.series_labels), series_lines: reCol(F.raw.series_lines)")
    );
}

#[test]
fn cards_gauges_and_kpis_follow_the_renderer() {
    // crates/render/src/bodies.rs and parts.rs, drawn by hand (plane + baseline-placed text)
    let js = script();
    let card = function(js, "cardBody");
    assert!(
        card.contains("clamp(Math.min((box.h - (showL ? lpx * 1.5 : 0)) * 0.78, box.w * 0.92 / per), 6, 36)")
    );
    assert!(card.contains("dinFace(ctx)"));
    let tile = function(js, "cardTile");
    for needle in [
        "Math.max(w - 26.6, 4)",
        "const gap = ctx.classic ? 8 : 2;",
        "top + 1.33 * first + gap + 1.06 * second",
    ] {
        assert!(tile.contains(needle), "cardTile lacks {needle}");
    }
    let nc = function(js, "cardNewBody");
    for needle in [
        "lpx: pt(lo, ctx.classic ? 24 : 16), vpx: pt(vo, ctx.classic ? 24 : 28 * PT)",
        "Math.sqrt(rowAspect * colAspect) >= 1.5",
        "top = 21.7, area = Math.max(box.h - 42, 8)",
        "if (ab.show !== true) return;",
        "const sub = plane(tW, rh, x, y);",
    ] {
        assert!(nc.contains(needle), "cardNewBody lacks {needle}");
    }
    let mr = function(js, "multiRowBody");
    assert!(mr.contains("head + (itemRows - 1) * 41.3 + 16 + 40.7"));
    let g = function(js, "gaugeChart");
    for needle in [
        "const inner = R - R * 0.3",
        "Math.min(vh - lift(R) - 6, half - 30)",
        "pct && value >= 0 && value <= 1 && lo < 1) hi = 1;",
        "inner * 1.8 / per), 6, 120)",
        "cy + 13.3",
    ] {
        assert!(g.contains(needle), "gaugeChart lacks {needle}");
    }
    assert!(function(js, "gaugeNum").contains("Math.floor(clamp(o.precision, 0, 10)) : 0"));
    let k = function(js, "kpiChart");
    for needle in [
        "Math.min(box.h * 0.19, box.w * 0.8 / per)",
        "const gcol = ds.cols[gi] || se.col;",
        "toLowerCase() === 'en'",
    ] {
        assert!(k.contains(needle), "kpiChart lacks {needle}");
    }
    // 'goal' comes from the snapshot's UI strings
    assert!(js.contains("'increase', 'decrease', 'goal'"));
    // text: the baseline placed from Chrome's measured offset, central = 0.414 em under y
    assert!(js.contains("const SEGOE_MID = 0.414;"));
    assert!(js.contains("const snapBase = yb => Math.round(yb + 0.2);"));
    assert!(function(js, "liftText").contains("snapBase(yb) - baseOff(px, wt, fam || TW_FONT)"));
    // the canvas border sits outside the page (no 1 px shift and squeeze of every visual)
    assert!(PAGE.contains("position: relative; box-sizing: content-box;"));
}

#[test]
fn grids_follow_the_renderer() {
    // crates/render/src/tables.rs: row heights, sort room, natural widths, scrolling, totals
    let js = script();
    let g = function(js, "gridTable");
    for needle in [
        "line = p => Math.round(p * 1.36)",
        "line(px) + 2 * lay.rowPad + rule",
        "Math.round(Math.round(hpx * 1.6) * 1.4)",
        "(arrowRoom ? SORT_ROOM : 0)",
        "return Math.min(m, px * 30) + pad * 2 + 1;",
        "nRows > fitN && (classic || lay.rowPad != null)",
        "clamp(Math.ceil(room / rowH), 0, 201)",
        "Math.max(box.w - (classic ? THUMB_W : SCROLL_W), 1)",
        "thumbLen(gridH, room, Math.max(nRows * rowH, 1))",
        "if (c.bar) pRect(P, x + w * c.bar.from",
    ] {
        assert!(g.contains(needle), "gridTable lacks {needle}");
    }
    let t = function(js, "tableBody");
    assert!(t.contains("ds.totals.some(x => x != null)") && t.contains("lay.rows = ds.totalRows;"));
    let m = function(js, "matrixBody");
    for needle in [
        "for (const l of arr(ds.subtotals)) {",
        "opt.bottom && rowSub(p.length)",
        "ds.rowLevels != null ? ds.rowLevels : nr",
        "if (colSub(0) && vI.length && (engine || (ckN > 1 && anyAdd)))",
    ] {
        assert!(m.contains(needle), "matrixBody lacks {needle}");
    }
    // the dataset carries the engine's totals, subtotal levels, displayed depth and row count
    let d = function(js, "dataset");
    assert!(d.contains("totals: Array.isArray(d.totals)") && d.contains("rowLevels: nat(d.row_levels)"));
    assert!(d.contains("Math.min(d.total_rows, 1e6)"));
}

#[test]
fn expanded_matrices_follow_the_renderer() {
    // crates/render/src/tables.rs `matrix_body`: member-by-member expansion, header rows per
    // column level, per-level subtotals and labels, bottom subtotals, bold upper levels
    let js = script();
    let d = function(js, "dataset");
    for needle in [
        "rowExp: expansionOf(d.row_expansion), colExp: expansionOf(d.column_expansion)",
        "rowLevelTotals: levelTotals(d.row_level_totals), colLevelTotals: levelTotals(d.column_level_totals)",
    ] {
        assert!(d.contains(needle), "dataset lacks {needle}");
    }
    // a member is expanded when its level is open XOR its path is toggled
    assert!(js.contains(
        "const expandedIn = (e, p, n) => p.length > 0 && p.length < n && (e.open[p.length - 1] === true) !== e.toggled.has(pathKey(p));"
    ));
    let m = function(js, "matrixBody");
    for needle in [
        "const rowOpen = p => ds.rowExp ? expandedIn(ds.rowExp, p, nr) : p.length > 0 && p.length < uniformR;",
        "const colOpen = p => !!ds.colExp && expandedIn(ds.colExp, p, nc);",
        "l[d] && l[d].label != null ? l[d].label : all != null ? all : word",
        "rowLabel: lbl('row_subtotals_label'), colLabel: lbl('column_subtotals_label')",
        "if (colSub(p.length) && vI.length) stack.push([p, true]);",
        "const topN = !nc ? 0 : nV > 1 ? depthL : Math.max(depthL - 1, 0);",
        "if (last === key) row[row.length - 1].span += nV; else row.push({ t: tx, span: nV, bold });",
        "if (opt.bottom && rowSub(p.length)) stack.push([p, true]);",
        "{ t: rowLabel(d), bold: true, ind: d }",
        "const bold = d < deepest",
        "kind === 'shut' || (!opt.bottom && rowSub(d)) ? cellsOf(p, bold) : blankCells()",
        "nr && rowSub(0) && (engine || (nTop > 1 && anyAdd))",
        "return gridTable(box, ctx, heads, body, total, tops, true, lay);",
    ] {
        assert!(m.contains(needle), "matrixBody lacks {needle}");
    }
    // several header rows; a 10 px indent per level at any font size
    assert!(js.contains("const STEP_INDENT = 10;"));
    let g = function(js, "gridTable");
    for needle in [
        "const tops = rowH * top.length,",
        "for (const tr of top) {",
        "leftPad = pad + (c.ind || 0) * STEP_INDENT",
        "(c.ind || 0) * STEP_INDENT);",
        "tw(c.t, hpx, c.bold ? 700 : headW)",
    ] {
        assert!(g.contains(needle), "gridTable lacks {needle}");
    }
}

#[test]
fn matrix_plus_minus_buttons_follow_the_renderer() {
    // crates/render/src/tables.rs: the row headers' +/- icons (rowHeaders.showExpandCollapseButtons)
    // on a member with children, or of the deepest level shown when the hierarchy goes deeper;
    // each sends matrix_expand / matrix_collapse and has a screen-reader label
    let js = script();
    let m = function(js, "matrixBody");
    for needle in [
        "function matrixBody(ds, box, F, ctx, v) {",
        "const btns = t.expand_buttons === true, moreLevels = dr.matrix === true && str(dr.role).toLowerCase() === 'rows' && dr.can_down === true;",
        "if (btns && (d < nr || (d === nr && moreLevels))) row[0].btn = { open: kind === 'open', path: p.slice(),",
        "const MAX_MATRIX_ROWS = 500, MAX_MATRIX_COLUMNS = 60, MAX_MATRIX_LEVELS = 8;",
    ] {
        assert!(
            m.contains(needle) || js.contains(needle),
            "matrixBody lacks {needle}"
        );
    }
    let b = function(js, "matrixButton");
    for needle in [
        "action: c.btn.open ? 'matrix_collapse' : 'matrix_expand', page: IXB.pk, visual: IXB.name, path: c.btn.path",
        "r.setAttribute('role', 'button'); r.setAttribute('aria-label', (c.btn.open ? T('Réduire', 'Collapse') : T('Développer', 'Expand'))",
        "if (!c.btn.open) pRect(",
    ] {
        assert!(b.contains(needle), "matrixButton lacks {needle}");
    }
    let g = function(js, "gridTable");
    assert!(g.contains(
        "(c.btn ? matrixButton(P, c, x + pad + (c.ind || 0) * STEP_INDENT, y, hh, px, color) : 0)"
    ));
    assert!(g.contains("(c.btn ? px * 1.3 : 0)"));
}

#[test]
fn kpis_follow_the_renderer() {
    // crates/render/src/fmt.rs `fmt_kpi` and parts.rs `kpi_chart` (Desktop 2.158, `oracle-gen kpis`)
    let js = script();
    let f = function(js, "fmtKpi");
    for needle in [
        "if (finite(units) && units >= 1) u = units >= 1000 ? units : null;",
        "else if (Math.abs(n) >= 1000 && (general || (p.max || 0) >= 2)) u = DIV[unitOf(Math.abs(n))];",
        "general && !Number.isInteger(n) ? numText(ctx, n, 2, 2, true) : fmtVal(n, col, ctx, false)",
        "precision != null ? [precision, precision] : !general && (p.max || 0) >= 2 ? [p.min || 0, p.max] : [2, 2]",
        "pre + numText(ctx, n / DIV[i], min, Math.max(min, max), general || p.group) + UNITS[i - 1] + post",
    ] {
        assert!(f.contains(needle), "fmtKpi lacks {needle}");
    }
    assert!(function(js, "literalEnds").contains("const ph = x => !x[1] && (x[0] === '0' || x[0] === '#');"));
    // a percentage is grouped only when its format groups ("95050,0%")
    assert!(js.contains("numText(ctx, n * 100, min, max, p ? p.group : true)"));
    assert!(js.contains("const KPI_VALUE_PX = 59.7;"));
    let k = function(js, "kpiChart");
    for needle in [
        "if (pts.length > 1 && hi > lo) {",
        "ctx.classic ? KPI_VALUE_PX : clamp(Math.min(box.h * 0.19, box.w * 0.8 / per), 6, 72)",
        "box.h * 0.519 - 1.7",
        "goal != null ? mid - vpx * 0.12 : mid",
        "box.w / 2 - vpx * 0.097",
        "pRect(P, bx - bw / 2, iy + gh * 0.32, bw, bw, color);",
        "y = vc + vpx * 0.57",
        "pText(P, x0 + tw(head + ' ', px, 600), y, dist, px, ctx.fg, { wt: 600 });",
        "fmtKpi(value, se.col, ctx, finite(ind.display_units) ? ind.display_units : null, prec)",
    ] {
        assert!(k.contains(needle), "kpiChart lacks {needle}");
    }
}

#[test]
fn has_the_open_in_desktop_button() {
    // the server fills the token slot; the button posts with the token header only, never a path
    assert_eq!(
        PAGE.matches(r#"<meta name="bicraft-open-token" content="">"#)
            .count(),
        1
    );
    let s = script();
    assert!(s.contains("fetch('api/open-in-desktop', { method: 'POST'"));
    assert!(s.contains("'X-Bicraft-Token': OPEN_TOKEN"));
    assert!(s.contains("/^[0-9a-f]{32}$/.test(t)"));
    assert!(
        s.contains("U('open_in_desktop'"),
        "label from the snapshot's ui.strings"
    );
    assert!(s.contains("obj(obj(S.snap.source).desktop)"));
}

#[test]
fn interactions_go_through_the_server_only() {
    // the server fills the interaction token slot; the page posts what was clicked, as JSON, with
    // the token header, and draws the next snapshot (the rules are bicraft's, `view.interact`)
    assert_eq!(
        PAGE.matches(r#"<meta name="bicraft-interact-token" content="">"#)
            .count(),
        1
    );
    let s = script();
    assert!(s.contains("fetch('api/interact' + VQ, { method: 'POST'"));
    assert!(s.contains("'X-Bicraft-Token': IX_TOKEN, 'Content-Type': 'application/json'"));
    assert_eq!(s.matches("fetch('api/interact'").count(), 1, "one place posts");
    // every kind of click the snapshot can act on
    for action in [
        "action: 'select'",
        "action: 'slicer_toggle'",
        "action: 'slicer_select_all'",
        "'slicer_range'",
        "'slicer_relative'",
        "'slicer_text'",
        "action: 'page'",
        "action: 'button'",
    ] {
        assert!(s.contains(action), "{action}");
    }
    // points are matched against the snapshot's selection; tooltips read text nodes only
    assert!(s.contains("obj(obj(v.interaction).selected).points"));
    assert!(s.contains("ctrl: !!(e.ctrlKey || e.metaKey)"));
    assert!(s.contains("function ixTip(e)"));
}

#[test]
fn server_mode_uses_relative_urls_a_private_view_and_a_download_link() {
    // one page for `bicraft-cli view` (at `/`) and `bicraft-cli serve` (under `/w/<id>/view/`): every
    // request is relative and carries the private view id when the server set one
    let s = script();
    for absolute in ["'/api/", "\"/api/"] {
        assert!(!s.contains(absolute), "absolute URL {absolute}");
    }
    assert!(s.contains("new EventSource('api/events' + VQ)"));
    assert!(s.contains("fetch('api/state' + VQ,"));
    assert!(s.contains("const VQ = VIEW_ID ? '?v=' + VIEW_ID : '';"));
    assert_eq!(
        PAGE.matches(r#"<meta name="bicraft-view-id" content="">"#)
            .count(),
        1
    );
    assert_eq!(
        PAGE.matches(r#"<meta name="bicraft-download" content="">"#)
            .count(),
        1
    );
    // the download link is built from the server's workspace-relative path only, segment by
    // segment, through the workspace's file route; dot segments give no link
    assert!(s.contains("return '../files/' + parts.map(encodeURIComponent).join('/');"));
    assert!(s.contains("parts.some(p => !p || p === '.' || p === '..')"));
    assert!(s.contains("obj(obj(S.snap.source).download)"));
    assert_eq!(
        s.matches("h('a', { class: 'obtn', id: 'download', href,").count(),
        1
    );
}
