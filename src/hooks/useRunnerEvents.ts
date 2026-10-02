import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  PortsPayload,
  ProjectRuntime,
  RestartingPayload,
  StatusPayload,
  StatusSnapshot,
} from "../types";

const IDLE: ProjectRuntime = {
  status: "stopped",
  action: null,
  reason: null,
  startedAtMs: null,
  ports: [],
};

export function runtimeOf(map: Map<string, ProjectRuntime>, id: string | null): ProjectRuntime {
  return (id && map.get(id)) || IDLE;
}

/**
 * Live runtime view of every project: status, current action, reason,
 * start time and listening ports. Driven by `project:status` and
 * `project:ports` events, and seeded from `get_statuses` on mount so a
 * webview reload (or HMR) doesn't show running projects as stopped.
 */
export function useProjectRuntime(): Map<string, ProjectRuntime> {
  const [runtime, setRuntime] = useState<Map<string, ProjectRuntime>>(() => new Map());

  useEffect(() => {
    const unlistens: UnlistenFn[] = [];
    let cancelled = false;
    // Ids that received a live event before the snapshot arrived — the
    // event is newer, so the snapshot must not overwrite it.
    const touched = new Set<string>();

    const update = (id: string, patch: (cur: ProjectRuntime) => ProjectRuntime) => {
      if (cancelled) return;
      touched.add(id);
      setRuntime((prev) => {
        const cur = prev.get(id) ?? IDLE;
        const next = patch(cur);
        if (next === cur) return prev;
        const map = new Map(prev);
        map.set(id, next);
        return map;
      });
    };

    (async () => {
      unlistens.push(
        await listen<StatusPayload>("project:status", (e) => {
          const p = e.payload;
          update(p.id, (cur) => {
            const active = p.status === "running" || p.status === "starting" || p.status === "stopping";
            return {
              status: p.status,
              action: p.action ?? (active ? cur.action : null),
              reason: p.reason ?? null,
              startedAtMs: p.startedAtMs ?? (active ? cur.startedAtMs : null),
              ports: active ? cur.ports : [],
            };
          });
        })
      );
      unlistens.push(
        await listen<PortsPayload>("project:ports", (e) => {
          update(e.payload.id, (cur) => {
            const same =
              cur.ports.length === e.payload.ports.length &&
              cur.ports.every((p, i) => p === e.payload.ports[i]);
            return same ? cur : { ...cur, ports: e.payload.ports };
          });
        })
      );

      try {
        const snaps = await invoke<StatusSnapshot[]>("get_statuses");
        if (cancelled) return;
        setRuntime((prev) => {
          const map = new Map(prev);
          for (const s of snaps) {
            if (touched.has(s.id)) continue;
            map.set(s.id, {
              status: s.status,
              action: s.action ?? null,
              reason: s.reason ?? null,
              startedAtMs: s.startedAtMs ?? null,
              ports: s.ports,
            });
          }
          return map;
        });
      } catch (e) {
        console.error("get_statuses failed", e);
      }
    })();

    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
    };
  }, []);

  return runtime;
}

/**
 * Subscription to `project:restarting` for one project. Used by the
 * detail panel to show a transient "Restarting in Ns (attempt x/y)…"
 * banner.
 */
export function useProjectRestarting(
  projectId: string,
  onRestarting: (attempt: number, max: number, delayMs: number) => void
): void {
  useEffect(() => {
    if (!projectId) return;
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    (async () => {
      const u = await listen<RestartingPayload>("project:restarting", (e) => {
        if (cancelled) return;
        if (e.payload.id === projectId) {
          onRestarting(e.payload.attempt, e.payload.max, e.payload.delayMs);
        }
      });
      if (cancelled) {
        u();
        return;
      }
      unlisten = u;
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [projectId, onRestarting]);
}

/** Ticking "now" for elapsed-time displays. Only ticks while `enabled`. */
export function useNow(enabled: boolean, intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!enabled) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(t);
  }, [enabled, intervalMs]);
  return now;
}
