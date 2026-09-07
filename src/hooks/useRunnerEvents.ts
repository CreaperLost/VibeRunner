import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  OutputPayload,
  PortsPayload,
  ProjectStatus,
  RestartingPayload,
  StatusPayload,
} from "../types";

/**
 * Subscribes to `project:status` events and returns a Map<id, ProjectStatus>
 * that the UI can read directly. Updates flow only one way: from the
 * backend → into this hook → into React state.
 *
 * Also tracks the name of the action whose PTY is alive (e.g. "Run",
 * "Setup") so the detail view can label the status pill.
 */
export function useProjectStatuses(): Map<string, ProjectStatus> {
  const [statuses, setStatuses] = useState<Map<string, ProjectStatus>>(
    () => new Map()
  );

  useEffect(() => {
    const unlistens: UnlistenFn[] = [];
    let cancelled = false;

    (async () => {
      const setStatus = (id: string, status: ProjectStatus) => {
        if (cancelled) return;
        setStatuses((prev) => {
          if (prev.get(id) === status) return prev;
          const next = new Map(prev);
          next.set(id, status);
          return next;
        });
      };

      unlistens.push(
        await listen<StatusPayload>("project:status", (e) => {
          setStatus(e.payload.id, e.payload.status);
        })
      );
    })();

    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
    };
  }, []);

  return statuses;
}

/** Map of project_id → action_name of the currently-running action. */
export function useCurrentActions(): Map<string, string> {
  const [actions, setActions] = useState<Map<string, string>>(() => new Map());

  useEffect(() => {
    const unlistens: UnlistenFn[] = [];
    let cancelled = false;

    (async () => {
      const setAction = (id: string, action: string | null) => {
        if (cancelled) return;
        setActions((prev) => {
          const next = new Map(prev);
          if (action) next.set(id, action);
          else next.delete(id);
          return next;
        });
      };

      unlistens.push(
        await listen<StatusPayload>("project:status", (e) => {
          // Only track the action when the project is actively
          // running/starting. Otherwise clear it.
          const alive = e.payload.status === "running" || e.payload.status === "starting";
          setAction(e.payload.id, alive ? e.payload.action ?? null : null);
        })
      );
    })();

    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
    };
  }, []);

  return actions;
}

/**
 * Subscribes to `project:output` events for a specific project and
 * invokes `onChunk` with each raw byte chunk.
 */
export function useProjectOutput(
  projectId: string,
  onChunk: (bytes: Uint8Array) => void
): void {
  useEffect(() => {
    if (!projectId) return;
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    (async () => {
      const u = await listen<OutputPayload>("project:output", (e) => {
        if (cancelled) return;
        if (e.payload.id === projectId) {
          onChunk(new Uint8Array(e.payload.chunk));
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
  }, [projectId, onChunk]);
}

/**
 * One-shot subscription to `project:restarting` for the given project.
 * Used by the detail panel to show a transient "Restarting in Ns
 * (attempt x/y)…" banner.
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
      const u = await listen<RestartingPayload>(
        "project:restarting",
        (e) => {
          if (cancelled) return;
          if (e.payload.id === projectId) {
            onRestarting(e.payload.attempt, e.payload.max, e.payload.delayMs);
          }
        }
      );
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

/** Map of project_id → currently-detected listening ports. */
export function useProjectPorts(): Map<string, number[]> {
  const [ports, setPorts] = useState<Map<string, number[]>>(() => new Map());

  useEffect(() => {
    const unlistens: UnlistenFn[] = [];
    let cancelled = false;

    (async () => {
      const setPortsFor = (id: string, list: number[]) => {
        if (cancelled) return;
        setPorts((prev) => {
          if (list.length === 0) {
            if (!prev.has(id)) return prev;
            const next = new Map(prev);
            next.delete(id);
            return next;
          }
          const prevList = prev.get(id);
          if (
            prevList &&
            prevList.length === list.length &&
            prevList.every((p, i) => p === list[i])
          ) {
            return prev;
          }
          const next = new Map(prev);
          next.set(id, list);
          return next;
        });
      };

      unlistens.push(
        await listen<PortsPayload>("project:ports", (e) => {
          setPortsFor(e.payload.id, e.payload.ports);
        })
      );

      unlistens.push(
        await listen<StatusPayload>("project:status", (e) => {
          if (e.payload.status === "stopped" || e.payload.status === "crashed") {
            setPortsFor(e.payload.id, []);
          }
        })
      );
    })();

    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
    };
  }, []);

  return ports;
}
