import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { errorText, isActiveStatus, openLocalPath } from "./utils";
import type {
  ResolvedProject,
  StatusPayload,
  VibeConfig,
  VibeConfigPath,
  VibeConfigReloadedPayload,
} from "./types";
import { Sidebar } from "./components/Sidebar";
import { ProjectDetail } from "./components/ProjectDetail";
import { Icon } from "./components/Icon";
import { runtimeOf, useProjectRuntime } from "./hooks/useRunnerEvents";
import { usePersistentState } from "./hooks/usePersistentState";

type Theme = "system" | "light" | "dark";
const THEME_ORDER: Theme[] = ["system", "light", "dark"];
import "./styles.css";

function App() {
  const [, setConfig] = useState<VibeConfig | null>(null);
  const [projects, setProjects] = useState<ResolvedProject[]>([]);
  const [configPath, setConfigPath] = useState<string>("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);
  const [pendingId, setPendingId] = useState<string | null>(null);

  const runtime = useProjectRuntime();

  const [theme, setTheme] = usePersistentState<Theme>("viberunner.theme", "system");
  useEffect(() => {
    if (theme === "system") document.documentElement.removeAttribute("data-theme");
    else document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  const [sidebarWidth, setSidebarWidth] = usePersistentState<number>(
    "viberunner.sidebar.width",
    340
  );
  const [isResizing, setIsResizing] = useState(false);

  const startResizing = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizing(true);
  }, []);

  const resetSidebarWidth = useCallback(() => {
    setSidebarWidth(340);
  }, [setSidebarWidth]);

  useEffect(() => {
    if (!isResizing) return;

    const onMouseMove = (e: MouseEvent) => {
      const minW = 260;
      const maxW = Math.max(minW, Math.min(650, window.innerWidth - 360));
      const nextW = Math.max(minW, Math.min(e.clientX, maxW));
      setSidebarWidth(nextW);
    };

    const onMouseUp = () => {
      setIsResizing(false);
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, [isResizing, setSidebarWidth]);

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
      setError(errorText(e));
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
        setError(errorText(e));
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
  const projectsRef = useRef(projects);
  projectsRef.current = projects;
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
        const name = projectsRef.current.find((p) => p.id === id)?.name ?? id;
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
            errorText(e)
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
          `setup_project failed: ${errorText(e)}`
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
          `build_project failed: ${errorText(e)}`
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
        `stop_project failed: ${errorText(e)}`
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
        `restart_project failed: ${errorText(e)}`
      );
    } finally {
      setPendingId((cur) => (cur === projectId ? null : cur));
    }
  }, []);

  // Bulk actions: start every stopped project, stop every active one.
  const inactive = projects.filter((p) => !isActiveStatus(runtimeOf(runtime, p.id).status));
  const active = projects.filter((p) => isActiveStatus(runtimeOf(runtime, p.id).status));

  const runAll = useCallback(async () => {
    for (const p of inactive) {
      if (p.primaryAction) runAction(p.id, p.primaryAction);
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
        `Could not open config file: ${errorText(e)}`
      );
    }
  }, [configPath]);

  const selectedProject =
    projects.find((p) => p.id === selectedId) ?? null;
  const nextTheme = THEME_ORDER[(THEME_ORDER.indexOf(theme) + 1) % THEME_ORDER.length];

  return (
    <div className="app">
      <header className="app__header">
        <div className="app__brand">
          <span className="app__logo">
            <Icon name="play" size={12} />
          </span>
          <h1 className="app__title">VibeRunner</h1>
        </div>
        <button
          type="button"
          className="app__config-path"
          onClick={openConfigInEditor}
          disabled={!configPath}
          title={configPath ? `Open ${configPath}` : "No config loaded"}
        >
          {configPath || "(no config loaded)"}
        </button>
        <div className="app__actions">
          <button
            type="button"
            className="btn btn--small btn--ghost"
            onClick={runAll}
            disabled={inactive.length === 0}
            title={inactive.length ? `Start ${inactive.length} idle project(s)` : "Nothing to start"}
          >
            <Icon name="play" size={12} />
            Run all
          </button>
          <button
            type="button"
            className="btn btn--small btn--ghost"
            onClick={stopAll}
            disabled={active.length === 0}
            title={active.length ? `Stop ${active.length} active project(s)` : "Nothing running"}
          >
            <Icon name="stop" size={12} />
            Stop all
          </button>
          <span className="app__divider" />
          <button
            type="button"
            className="icon-btn"
            onClick={reload}
            disabled={reloading}
            title="Reload vibe.config.json"
          >
            <Icon name="refresh" size={15} className={reloading ? "spin" : undefined} />
          </button>
          <button
            type="button"
            className="icon-btn"
            onClick={() => setTheme(nextTheme)}
            title={`Theme: ${theme} (click for ${nextTheme})`}
          >
            <Icon name={theme === "light" ? "sun" : theme === "dark" ? "moon" : "monitor"} size={15} />
          </button>
        </div>
      </header>

      {error && (
        <div className="app__error" role="alert">
          <Icon name="alert" size={14} />
          <span className="app__error-text">{error}</span>
          <button type="button" className="icon-btn icon-btn--sm" onClick={() => setError(null)} title="Dismiss">
            <Icon name="x" size={12} />
          </button>
        </div>
      )}

      <div className={`app__body${isResizing ? " app__body--resizing" : ""}`}>
        <Sidebar
          projects={projects}
          runtime={runtime}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onConfigReloaded={(payload) => {
            applyConfig(payload.config, payload.projects, payload.path);
          }}
          onError={setError}
          width={sidebarWidth}
        />
        <div
          className={`app__splitter${isResizing ? " app__splitter--dragging" : ""}`}
          onMouseDown={startResizing}
          onDoubleClick={resetSidebarWidth}
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize projects sidebar"
          title="Drag to resize projects sidebar · Double-click to reset"
        />
        <ProjectDetail
          project={selectedProject}
          runtime={runtimeOf(runtime, selectedId)}
          pending={pendingId === selectedId}
          onRunAction={runAction}
          onSetup={setupProject}
          onBuild={buildProject}
          onStop={stopProject}
          onRestart={restartProject}
          onError={setError}
        />
      </div>
    </div>
  );
}

export default App;
