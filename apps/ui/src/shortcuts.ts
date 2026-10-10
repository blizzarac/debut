import { useEffect, useRef } from "react";
import type { Binding, Chord, Keymap } from "./engine";

/** Keyboard shortcuts (TL-12). One window listener looks the pressed chord up in
 * the active keymap and runs whatever component registered that action, so keys
 * are defined once per preset (in the engine) instead of in every component. */

const handlers = new Map<string, Array<() => void>>();
let bindings: Binding[] = [];
let capture: ((c: Chord | null) => void) | null = null;

/** Hand the next key chord to `cb` instead of running a shortcut (for
 * rebinding); Escape gives null. */
export function captureNextChord(cb: (c: Chord | null) => void) {
  capture = cb;
}

/** The keymap to dispatch with (an engine preset). */
export function setKeymap(map: Keymap | null) {
  bindings = map?.bindings ?? [];
}

/** Run `handler` when `action`'s shortcut is pressed while this component is mounted. */
export function useShortcut(action: string, handler: (() => void) | null | undefined) {
  const latest = useRef(handler);
  latest.current = handler;
  useEffect(() => {
    const run = () => latest.current?.();
    const list = handlers.get(action) ?? [];
    list.push(run);
    handlers.set(action, list);
    return () => {
      const l = handlers.get(action) ?? [];
      handlers.set(
        action,
        l.filter((h) => h !== run),
      );
    };
  }, [action]);
}

/** The chord a key event is, in the engine's notation: lowercased key, the Mac
 * delete key as "delete", Ctrl or ⌘ as `cmd`. */
export function chordOf(e: KeyboardEvent): { key: string; cmd: boolean; shift: boolean; alt: boolean } {
  let key = e.key.toLowerCase();
  if (key === "backspace") key = "delete";
  return { key, cmd: e.ctrlKey || e.metaKey, shift: e.shiftKey, alt: e.altKey };
}

/** The action bound to a key event, if any. */
export function actionFor(e: KeyboardEvent): string | null {
  const c = chordOf(e);
  const b = bindings.find((b) => b.key === c.key && b.cmd === c.cmd && b.shift === c.shift && b.alt === c.alt);
  return b?.action ?? null;
}

/** Install the window listener; returns its removal. Typing in a field never
 * triggers shortcuts. */
export function installShortcuts(): () => void {
  const onKey = (e: KeyboardEvent) => {
    if (capture) {
      if (["Shift", "Control", "Meta", "Alt"].includes(e.key)) return;
      e.preventDefault();
      const cb = capture;
      capture = null;
      cb(e.key === "Escape" ? null : chordOf(e));
      return;
    }
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
    const action = actionFor(e);
    const list = action ? handlers.get(action) : undefined;
    if (!list || list.length === 0) return;
    e.preventDefault();
    list[list.length - 1]();
  };
  window.addEventListener("keydown", onKey);
  return () => window.removeEventListener("keydown", onKey);
}

/** "⌘⇧Z"-style label for a binding. */
export function chordLabel(b: Chord): string {
  const mac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);
  const names: Record<string, string> = { " ": "Space", arrowleft: "←", arrowright: "→", delete: mac ? "⌫" : "Del", home: "Home" };
  const key = names[b.key] ?? b.key.toUpperCase();
  return `${b.cmd ? (mac ? "⌘" : "Ctrl+") : ""}${b.alt ? (mac ? "⌥" : "Alt+") : ""}${b.shift ? (mac ? "⇧" : "Shift+") : ""}${key}`;
}
