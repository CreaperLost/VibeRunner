import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { openLocalPath } from "./utils";
import type {
  ProjectStatus,
  ResolvedProject,
  StatusPayload,
  VibeConfig,
  VibeConfigPath,
  VibeConfigReloadedPayload,
} from "./types";
import { Sidebar } from "./components/Sidebar";
import { ProjectDetail } from "./components/ProjectDetail";
import {
  useCurrentActions,
  useProjectPorts,
  useProjectStatuses,
} from "./hooks/useRunnerEvents";
import "./styles.css";

function App() {
  const [, setConfig] = useState<VibeConfig | null>(null);
  const [projects, setProjects] = useState<ResolvedProject[]>([]);
  const [configPath, setConfigPath] = useState<string>("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);
  const [pendingId, setPendingId] = useState<string | null>(null);

  const statuses = useProjectStatuses();
  const currentActions = useCurrentActions();
  const ports = useProjectPorts();

  /** Apply a freshly-loaded config + resolved projects; fix selection
   *  if the previously selected project no longer exists. */
  const applyConfig = useCallback(
    (cfg: VibeConfig, resolved: ResolvedProject[], path: string) => {
      setConfig(cfg);
      setProjects(resolved);
      setConfigPath(path);
      setSelectedId((cur) => {
        if (cur && resolved.some((p) => p.id === cur)) return cur;
        return resolved[0]?.id ?? null;
      });
    },
    []
  );

  const reload = useCallback(async () => {
    setReloading(true);
    setError(null);
    try {
      const cfg = await invoke<VibeConfig>("reload_config");
      const resolved = await invoke<ResolvedProject[]>("list_projects");
      const p = await invoke<VibeConfigPath>("get_config_path");
      applyConfig(cfg, resolved, p.path);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setReloading(false);
    }
  }, [applyConfig]);

  // Initial load.
  useEffect(() => {
    (async () => {
      try {
        const cfg = await invoke<VibeConfig>("get_config");
        const resolved = await invoke<ResolvedProject[]>("list_projects");
        const p = await invoke<VibeConfigPath>("get_config_path");
        applyConfig(cfg, resolved, p.path);
      } catch (e) {
        setError(typeof e === "string" ? e : String(e));
      }
    })();
  }, [applyConfig]);

  // File-watcher / Add / Remove → config:reloaded event.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    (async () => {
      const u = await listen<VibeConfigReloadedPayload>(
        "config:reloaded",
        (e) => {
          if (cancelled) return;
          applyConfig(e.payload.config, e.payload.projects, e.payload.path);
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
  }, [applyConfig]);

  // Native notification on crash. We request permission once on mount,
  // then fire a notification whenever any project transitions to
  // "crashed" (auto-restart failures included). One notification per
  // crash, not on every status update.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    (async () => {
      let granted = await isPermissionGranted();
      if (!granted) {
        const perm = await requestPermission();
        granted = perm === "granted";
      }
      if (!granted) return;

      const u = await listen<StatusPayload>("project:status", (e) => {
        if (cancelled) return;
        if (e.payload.status !== "crashed") return;
        const id = e.payload.id;
        const name = projects.find((p) => p.id === id)?.name ?? id;
        const reason = e.payload.reason ? ` — ${e.payload.reason}` : "";
        sendNotification({
          title: `${name} crashed`,
          body: `VibeRunner: ${name} stopped unexpectedly${reason}`,
        });
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
    // We intentionally only run this once. `projects` is captured at
    // mount time — by design, we want crash notifications even if the
    // user has the project list open in a stale closure.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runAction = useCallback(
    async (projectId: string, actionName: string) => {
      setPendingId(projectId);
      setError(null);
      try {
        await invoke("run_action", { projectId, actionName });
      } catch (e) {
        setError(
          `run_action(${actionName}) failed: ${
            typeof e === "string" ? e : String(e)
          }`
        );
      } finally {
        setPendingId((cur) => (cur === projectId ? null : cur));
      }
    },
    []
  );

  const setupProject = useCallback(
    async (projectId: string) => {
      setPendingId(projectId);
      setError(null);
      try {
        await invoke("setup_project", { projectId });
      } catch (e) {
        setError(
          `setup_project failed: ${typeof e === "string" ? e : String(e)}`
        );
      } finally {
        setPendingId((cur) => (cur === projectId ? null : cur));
      }
    },
    []
  );

  const buildProject = useCallback(
    async (projectId: string) => {
      setPendingId(projectId);
      setError(null);
      try {
        await invoke("build_project", { projectId });
      } catch (e) {
        setError(
          `build_project failed: ${typeof e === "string" ? e : String(e)}`
        );
      } finally {
        setPendingId((cur) => (cur === projectId ? null : cur));
      }
    },
    []
  );

  const stopProject = useCallback(async (projectId: string) => {
    setPendingId(projectId);
    setError(null);
    try {
      await invoke("stop_project", { projectId });
    } catch (e) {
      setError(
        `stop_project failed: ${typeof e === "string" ? e : String(e)}`
      );
    } finally {
      setPendingId((cur) => (cur === projectId ? null : cur));
    }
  }, []);

  const restartProject = useCallback(async (projectId: string) => {
    setPendingId(projectId);
    setError(null);
    try {
      await invoke("restart_project", { projectId });
    } catch (e) {
      setError(
        `restart_project failed: ${typeof e === "string" ? e : String(e)}`
      );
    } finally {
      setPendingId((cur) => (cur === projectId ? null : cur));
    }
  }, []);

  // Bulk actions: start every stopped project, stop every active one.
  const inactive = projects.filter((p) => {
    const s = statuses.get(p.id) ?? "stopped";
    return s === "stopped" || s === "crashed";
  });
  const active = projects.filter((p) => {
    const s = statuses.get(p.id);
    return s === "running" || s === "starting" || s === "stopping";
  });

  const runAll = useCallback(async () => {
    for (const p of inactive) {
      const primary = p.primaryAction ?? p.actions[0]?.name;
      if (primary) runAction(p.id, primary);
    }
  }, [inactive, runAction]);

  const stopAll = useCallback(async () => {
    for (const p of active) {
      stopProject(p.id);
    }
  }, [active, stopProject]);

  const openConfigInEditor = useCallback(async () => {
    if (!configPath) return;
    try {
      await openLocalPath(configPath);
    } catch (e) {
      setError(
        `Could not open config file: ${typeof e === "string" ? e : String(e)}`
      );
    }
  }, [configPath]);

  const selectedProject =
    projects.find((p) => p.id === selectedId) ?? null;
  const selectedStatus: ProjectStatus =
    (selectedId && statuses.get(selectedId)) || "stopped";

  return (
    <div className="app">
      <header className="app__header">
        <div className="app__brand">
          <span className="app__logo">▶</span>
          <h1 className="app__title">VibeRunner</h1>
        </div>
        <div className="app__meta">
          <span
            className="app__config-path"
            title={configPath || "(no config loaded)"}
          >
            {configPath || "(no config loaded)"}
          </span>
          <div className="app__bulk">
            <button
              type="button"
              className="btn btn--small"
              onClick={runAll}
              disabled={inactive.length === 0}
              title={
                inactive.length === 0
                  ? "No stopped projects to start"
                  : `Start ${inactive.length} stopped project${
                      inactive.length === 1 ? "" : "s"
                    }`
              }
            >
              ▶ Run all
            </button>
            <button
              type="button"
              className="btn btn--small"
              onClick={stopAll}
              disabled={active.length === 0}
              title={
                active.length === 0
                  ? "No active projects to stop"
                  : `Stop ${active.length} active project${
                      active.length === 1 ? "" : "s"
                    }`
              }
            >
              ■ Stop all
            </button>
          </div>
          <button
            type="button"
            className="btn"
            onClick={reload}
            disabled={reloading}
            title="Reload vibe.config.json from disk"
          >
            {reloading ? "Reloading…" : "↻ Reload"}
          </button>
          <button
            type="button"
            className="btn"
            onClick={openConfigInEditor}
            disabled={!configPath}
            title={
              configPath
                ? `Open ${configPath} in your default editor`
                : "No config loaded"
            }
          >
            ↗ Open
          </button>
        </div>
      </header>

      {error && (
        <div
          className="app__error"
          role="alert"
          onClick={() => setError(null)}
        >
          <strong>Error:</strong> {error}
          <span className="app__error-dismiss"> (click to dismiss)</span>
        </div>
      )}

      <div className="app__body">
        <Sidebar
          projects={projects}
          statuses={statuses}
          currentActions={currentActions}
          ports={ports}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onConfigReloaded={(payload) => {
            applyConfig(payload.config, payload.projects, payload.path);
          }}
          onRemoveProject={(id) => {
            setProjects((prev) => prev.filter((p) => p.id !== id));
            setSelectedId((cur) => {
              if (cur === id) {
                const remaining = projects.filter((p) => p.id !== id);
                return remaining[0]?.id ?? null;
              }
              return cur;
            });
          }}
          onError={setError}
        />
        <ProjectDetail
          project={selectedProject}
          status={selectedStatus}
          currentAction={
            selectedId ? currentActions.get(selectedId) ?? null : null
          }
          detectedPorts={selectedId ? ports.get(selectedId) ?? [] : []}
          pending={pendingId === selectedId}
          onRunAction={runAction}
          onSetup={setupProject}
          onBuild={buildProject}
          onStop={stopProject}
          onRestart={restartProject}
        />
      </div>
    </div>
  );
}

export default App;
