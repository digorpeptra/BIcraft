// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): the categorical data view read
// as rows (one per category value), with each column's display name, format string and roles.
// From the shape the public `powerbi-visuals-api` types give `DataView.categorical` and
// `DataView.metadata.columns`.
"use strict";

/** Most rows and value columns read (the visuals' data view mappings keep below). */
export const MAX_ROWS = 30000;
export const MAX_COLUMNS = 64;

/** A column of the table: its name, format string and the roles it is bound to. */
export interface Column {
    name: string;
    format: string;
    roles: string[];
    values: any[];
    /** The data view's own column object (for `withCategory`). */
    source: any;
}

/** The categorical data view as rows. */
export interface Table {
    category: Column | null;
    values: Column[];
    rows: number;
}

function rolesOf(source: any): string[] {
    const r = source && source.roles && typeof source.roles === "object" ? source.roles : {};
    return Object.keys(r).filter((k) => !!r[k]);
}

function columnOf(c: any): Column {
    const source = c && c.source ? c.source : {};
    const values = c && Array.isArray(c.values) ? c.values.slice(0, MAX_ROWS) : [];
    return {
        name: typeof source.displayName === "string" ? source.displayName : "",
        format: typeof source.format === "string" ? source.format : "",
        roles: rolesOf(source),
        values,
        source: c,
    };
}

/** The first category and the value columns of `dataView.categorical`. */
export function tableOf(dataView: any): Table {
    const cat = dataView && dataView.categorical ? dataView.categorical : null;
    const category = cat && Array.isArray(cat.categories) && cat.categories.length > 0 ? columnOf(cat.categories[0]) : null;
    const values: Column[] = [];
    const list: any[] = cat && cat.values && typeof cat.values.length === "number" ? cat.values : [];
    for (let k = 0; k < list.length && k < MAX_COLUMNS; k++) {
        values.push(columnOf(list[k]));
    }
    let rows = category ? category.values.length : 0;
    for (const v of values) {
        rows = Math.max(rows, v.values.length);
    }
    return { category, values, rows: Math.min(rows, MAX_ROWS) };
}

/** The roles that have at least one field in the data view (`metadata.columns[].roles`). */
export function boundRoles(dataView: any): string[] {
    const cols: any[] = dataView && dataView.metadata && Array.isArray(dataView.metadata.columns) ? dataView.metadata.columns : [];
    const out: string[] = [];
    for (const c of cols.slice(0, 4096)) {
        for (const r of rolesOf(c)) {
            if (out.indexOf(r) < 0) {
                out.push(r);
            }
        }
    }
    return out;
}

/** The roles of `required` that no field is bound to. */
export function missingRoles(dataView: any, required: string[]): string[] {
    const bound = boundRoles(dataView);
    return required.filter((r) => bound.indexOf(r) < 0);
}

/** A value of the visual's format objects (`metadata.objects[object][property]`), else `fallback`. */
export function objectValue<T>(dataView: any, object: string, property: string, fallback: T): T {
    const objects = dataView && dataView.metadata ? dataView.metadata.objects : undefined;
    const o = objects && typeof objects === "object" ? objects[object] : undefined;
    if (!o || typeof o !== "object" || !(property in o)) {
        return fallback;
    }
    const v = o[property];
    if (typeof fallback === "string" && v && typeof v === "object" && v.solid && typeof v.solid.color === "string") {
        return v.solid.color as any;
    }
    return typeof v === typeof fallback ? v : fallback;
}
