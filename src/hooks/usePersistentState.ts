import { useCallback, useEffect, useRef, useState } from "react";

/**
 * `useState` backed by `localStorage`. The value persists across page
 * reloads and app restarts.
 *
 * - On mount: reads `localStorage[key]`, falls back to `defaultValue`
 *   (also written through if the key is absent so reads stay stable).
 * - On change: writes the new value to `localStorage` and updates state.
 *
 * Storage failures (private mode, quota, etc.) are swallowed and the
 * state behaves like a plain `useState` — the UI never breaks because
 * persistence is unavailable.
 */
export function usePersistentState<T>(
  key: string,
  defaultValue: T
): [T, (next: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = window.localStorage.getItem(key);
      if (raw === null) return defaultValue;
      return JSON.parse(raw) as T;
    } catch {
      return defaultValue;
    }
  });

  // Track the latest value so the storage-write effect doesn't have to
  // depend on it (and thus re-fire on every render).
  const valueRef = useRef(value);
  useEffect(() => {
    valueRef.current = value;
  }, [value]);

  // Persist on every change. Errors are swallowed (e.g. quota).
  useEffect(() => {
    try {
      window.localStorage.setItem(key, JSON.stringify(value));
    } catch {
      /* ignore */
    }
  }, [key, value]);

  // Cross-tab sync: if another window updates the same key, pick it up.
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key !== key || e.newValue === null) return;
      try {
        setValue(JSON.parse(e.newValue) as T);
      } catch {
        /* ignore */
      }
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, [key]);

  const update = useCallback((next: T | ((prev: T) => T)) => {
    setValue((prev) => {
      const resolved =
        typeof next === "function"
          ? (next as (prev: T) => T)(prev)
          : next;
      // Also push through the ref so the next render's effect write
      // matches the latest state without an extra render.
      valueRef.current = resolved;
      return resolved;
    });
  }, []);

  return [value, update];
}