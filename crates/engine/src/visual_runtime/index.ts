// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): its exports in one module.
// Nothing here touches the browser when loaded (only `showLanding` and `showMessage` need a
// `document`), so BIcraft's tests run it under Node with a stand-in host and elements.
"use strict";

export { Interactions } from "./interactions";
export { contrastOf, contrastPrim } from "./contrast";
export { boundRoles, missingRoles, objectValue, tableOf } from "./data";
export { dateLabel, formatDate, formatNumber, formatValue, isDateFormat, roundDigits, sections } from "./format";
export { card, color, group, model, number, textArea, textInput, toggle } from "./formatting";
export { landingText } from "./landing";
export { keyAction, moveFocus } from "./keys";
export { categoryIds, clickPlan, dimmed, multiIds, rowsOf } from "./selection";
export { joinNames, langOf, pick, t } from "./strings";
export { cornerPath, decorate, freshData, motionAllowed } from "./paint";
export { BICRAFT_PALETTE, PALETTES, PRESETS, lookFlat, lookOf } from "./style";
export { TEXT_PROPS, mergeText, textCards, textOf } from "./textsettings";
export { tooltipItems, tooltipText } from "./tooltips";
