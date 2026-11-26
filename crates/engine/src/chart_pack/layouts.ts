// BIcraft's chart pack (docs/formats/chart-pack.md): every layout in one module, for the golden
// tests (crates/engine/tests/report/chart_pack_tests.rs), which run it under Node.
"use strict";

import { Out } from "./kit";
import { layoutCalendar } from "./calendar";
import { layoutLineArea } from "./linearea";
import { layoutMekko } from "./mekko";
import { layoutOrg } from "./org";
import { layoutSankey } from "./sankey";
import { layoutScatter } from "./scatter";
import { layoutVenn } from "./venn";
import { layoutWaterfall } from "./waterfall";
import { layoutGauge } from "./gauge";

export { layoutCalendar, layoutGauge, layoutLineArea, layoutMekko, layoutOrg, layoutSankey, layoutScatter, layoutVenn, layoutWaterfall };

/** The layout of a visual of the pack by its kind (org, venn, mekko, calendar, sankey, scatter, linearea). */
export function layout(kind: string, input: any, w: number, h: number): Out {
    switch (kind) {
        case "org":
            return layoutOrg(input, w, h);
        case "venn":
            return layoutVenn(input, w, h);
        case "mekko":
            return layoutMekko(input, w, h);
        case "calendar":
            return layoutCalendar(input, w, h);
        case "sankey":
            return layoutSankey(input, w, h);
        case "scatter":
            return layoutScatter(input, w, h);
        case "linearea":
            return layoutLineArea(input, w, h);
        case "waterfall":
            return layoutWaterfall(input, w, h);
        case "gauge":
            return layoutGauge(input, w, h);
        default:
            return { prims: [], hits: [] };
    }
}
