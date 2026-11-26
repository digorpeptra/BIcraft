// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): what a key does on a focused
// data point (the visual declares `supportsKeyboardFocus`, so the report's keyboard navigation
// enters it; Tab moves between points).
"use strict";

export type KeyAction = "select" | "multiSelect" | "clear" | "next" | "previous" | "first" | "last" | "menu" | null;

/** The action of `key` (a `KeyboardEvent.key`), with Ctrl (or ⌘) and Shift held or not. */
export function keyAction(key: string, ctrl: boolean, shift: boolean): KeyAction {
    switch (key) {
        case "Enter":
        case " ":
        case "Spacebar":
            return ctrl ? "multiSelect" : "select";
        case "Escape":
        case "Esc":
            return "clear";
        case "ArrowRight":
        case "ArrowDown":
        case "Right":
        case "Down":
            return "next";
        case "ArrowLeft":
        case "ArrowUp":
        case "Left":
        case "Up":
            return "previous";
        case "Home":
            return "first";
        case "End":
            return "last";
        case "ContextMenu":
        case "Apps":
            return "menu";
        case "F10":
            return shift ? "menu" : null;
        default:
            return null;
    }
}

/** The point an arrow, Home or End moves to among `count` points from `current` (no wrap). */
export function moveFocus(current: number, action: KeyAction, count: number): number {
    if (count <= 0) {
        return -1;
    }
    const at = current < 0 ? 0 : Math.min(current, count - 1);
    switch (action) {
        case "next":
            return Math.min(at + 1, count - 1);
        case "previous":
            return Math.max(at - 1, 0);
        case "first":
            return 0;
        case "last":
            return count - 1;
        default:
            return at;
    }
}
