// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): the landing page a visual shows
// while its required roles have no field (the visual declares `supportsLandingPage` and
// `supportsEmptyDataView`, so the host updates it without data), and short messages.
"use strict";

import { joinNames, t } from "./strings";

/** The landing page's text: which roles to fill, in the locale's language. */
export function landingText(locale: string | undefined | null, roleNames: string[]): string {
    return t(locale, "landingAdd", joinNames(locale, roleNames));
}

/** Replaces the content of `root` with a landing page: a title and a line of text. */
export function showLanding(root: HTMLElement, title: string, text: string): void {
    clear(root);
    const box = document.createElement("div");
    box.className = "bicraft-landing";
    const h = document.createElement("p");
    h.className = "bicraft-landing-title";
    h.textContent = title;
    const p = document.createElement("p");
    p.className = "bicraft-landing-text";
    p.textContent = text;
    box.appendChild(h);
    box.appendChild(p);
    root.appendChild(box);
}

/** Replaces the content of `root` with one line of text. */
export function showMessage(root: HTMLElement, text: string): void {
    clear(root);
    const p = document.createElement("p");
    p.className = "bicraft-message";
    p.textContent = text;
    root.appendChild(p);
}

export function clear(root: Element): void {
    while (root.firstChild) {
        root.removeChild(root.firstChild);
    }
}
