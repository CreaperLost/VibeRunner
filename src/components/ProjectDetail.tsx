import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ArtifactsScan,
  ProjectArtifact,
  ProjectStatus,
  ResolvedAction,
  ResolvedProject,
  VibeConfigReloadedPayload,
} from "../types";
import { StatusPill } from "./StatusPill";
import { LogViewer, type LogViewerHandle } from "./LogViewer";
import { PortList } from "./PortList";
import { useProjectRestarting } from "../hooks/useRunnerEvents";
import { openBrowserUrl, openLocalPath } from "../utils";

interface ProjectDetailProps {
  project: ResolvedProject | null;
  status: ProjectStatus;
  currentAction: string | null;
  /** Currently-detected listening ports for the selected project. */
  detectedPorts: number[];
  pending: boolean;
  onRunAction: (projectId: string, actionName: string) => void;
  onSetup: (projectId: string) => void;
  onBuild?: (projectId: string) => void;
  onStop: (projectId: string) => void;
  onRestart: (projectId: string) => void;
}

/** Map an icon string from the TOML/action config to a unicode glyph. */
function iconGlyph(icon: string | null | undefined): string {
  switch (icon) {
    case "run":
      return "▶";
    case "stop":
      return "■";
    case "tool":
    case "setup":
      return "🔧";
    case "build":
      return "🛠";
    case "test":
      return "✓";
    case "migrate":
      return "↷";
    default:
      return "•";
  }
}

export function ProjectDetail({
  project,
  status,
  currentAction,
  detectedPorts,
  pending,
  onRunAction,
  onSetup,
  onBuild,
  onStop,
  onRestart,
}: ProjectDetailProps) {
  if (!project) {
    return (
      <section className="detail detail--empty">
        <div className="detail__empty-msg">
          <p>Select a project to see its details.</p>
        </div>
      </section>
    );
  }

  const isActive =
    status === "running" || status === "starting" || status === "stopping";
  const isCrashed = status === "crashed";
  const canRunAction = !pending && !isActive;
  const canStop = !pending && (isActive || isCrashed);
  const canRestart = !pending;

  // The implicit "Setup" action is the first one with name "Setup".
  const setupAction = project.actions.find((a) => a.name === "Setup") ?? null;
  // The "Build" action if configured or present.
  const buildAction =
    project.actions.find(
      (a) =>
        (a.name.toLowerCase() === "build" || a.icon === "build") &&
        a.name !== "Setup"
    ) ?? null;
  // User-defined actions: everything except the implicit Setup, Build, and Stop (which have their own dedicated buttons).
  const userActions = project.actions.filter(
    (a) =>
      a.name !== "Setup" &&
      a.name.toLowerCase() !== "build" &&
      a.icon !== "build" &&
      a.name.toLowerCase() !== "stop" &&
      a.icon !== "stop"
  );
  const hasRestart = project.primaryAction !== null;

  const primaryPort = detectedPorts.length > 0 ? Math.min(...detectedPorts) : null;
  const [autoOpen, setAutoOpen] = useState(() => {
    return localStorage.getItem("viberunner_auto_open") !== "false";
  });
  const autoOpenedRef = useRef<string | null>(null);

  useEffect(() => {
    localStorage.setItem("viberunner_auto_open", String(autoOpen));
  }, [autoOpen]);

  // Auto-open in browser when the port is detected and the app is active
  useEffect(() => {
    if (!autoOpen || !primaryPort || !isActive) {
      if (!isActive) {
        autoOpenedRef.current = null;
      }
      return;
    }
    const key = `${project.id}:${primaryPort}`;
    if (autoOpenedRef.current !== key) {
      autoOpenedRef.current = key;
      openBrowserUrl(`http://localhost:${primaryPort}`).catch((e) =>
        console.error("auto openUrl failed", e)
      );
    }
  }, [autoOpen, primaryPort, isActive, project.id]);

  const [logExpanded, setLogExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const [isScrolledUp, setIsScrolledUp] = useState(false);
  const logActionsRef = useRef<LogViewerHandle | null>(null);

  // Auto-discovered build artifacts (.dmg, .app, .exe, ...). Scanned
  // on mount + whenever the project path changes or the user clicks
  // Refresh. Used to surface "Install" and "Run Portable" buttons
  // alongside the TOML-defined actions.
  const [scan, setScan] = useState<ArtifactsScan | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const scanSeqRef = useRef(0);

  const runScan = async (path: string) => {
    const seq = ++scanSeqRef.current;
    setScanning(true);
    setScanError(null);
    try {
      const result = await invoke<ArtifactsScan>("scan_project_artifacts", { path });
      if (seq !== scanSeqRef.current) return; // a newer scan superseded us
      setScan(result);
    } catch (e) {
      if (seq !== scanSeqRef.current) return;
      setScanError(typeof e === "string" ? e : String(e));
      setScan(null);
    } finally {
      if (seq === scanSeqRef.current) setScanning(false);
    }
  };

  // Scan whenever the selected project (path) changes. This is also
  // the "refresh on select" behavior; the user can force a fresh
  // scan via the Refresh button after a Build completes.
  useEffect(() => {
    if (project) runScan(project.path);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.path]);

  // After a build, re-scan so a freshly produced .app / .dmg shows
  // up without a manual refresh. We detect "build finished" by
  // watching `pending`: it flips true → false around a build.
  const prevPendingRef = useRef(pending);
  useEffect(() => {
    if (prevPendingRef.current && !pending && project) {
      runScan(project.path);
    }
    prevPendingRef.current = pending;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending]);

  // Re-scan whenever the global config is reloaded. Covers:
  //   - the header "↻ Reload" button (calls reload_config in App.tsx,
  //     which emits config:reloaded),
  //   - the file-watcher auto-reload on edits to vibe.config.json,
  //   - any add/remove project operation.
  // Subscribed once with a ref so the listener survives project
  // changes without re-subscribing.
  const projectForReloadRef = useRef(project);
  useEffect(() => {
    projectForReloadRef.current = project;
  }, [project]);
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    (async () => {
      const u = await listen<VibeConfigReloadedPayload>(
        "config:reloaded",
        () => {
          if (cancelled) return;
          const cur = projectForReloadRef.current;
          if (cur) runScan(cur.path);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleCopyLogs = async () => {
    if (logActionsRef.current) {
      const ok = await logActionsRef.current.copyLogs();
      if (ok) {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      }
    }
  };

  const handleOpenBrowser = async () => {
    if (primaryPort) {
      await openBrowserUrl(`http://localhost:${primaryPort}`);
      return;
    }

    const candidatePort =
      project.env?.["PORT"] ||
      project.env?.["VITE_PORT"] ||
      (detectedPorts.length > 0 ? String(detectedPorts[0]) : "3000");


    const answer = window.prompt(
      `No listening port detected automatically yet.\nEnter localhost port to open:`,
      candidatePort
    );
    if (answer && answer.trim()) {
      const portNum = parseInt(answer.trim(), 10);
      if (!isNaN(portNum) && portNum > 0 && portNum <= 65535) {
        await openBrowserUrl(`http://localhost:${portNum}`);
      }
    }
  };

  return (
    <section className="detail">
      <header className="detail__header">
        <div>
          <h2 className="detail__title">{project.name}</h2>
          <p className="detail__id">
            id: {project.id} ·{" "}
            <span className="detail__source">
              {project.source === "toml"
                ? "auto (TOML)"
                : project.source === "manual"
                  ? "manual"
                  : "empty"}
            </span>
          </p>
        </div>
        <div className="detail__status">
          <StatusPill status={status} action={currentAction} size="md" />
        </div>
      </header>

      {project.warnings.length > 0 && (
        <div className="detail__warnings" role="alert">
          {project.warnings.map((w, i) => (
            <div key={i}>⚠ {w}</div>
          ))}
        </div>
      )}

      <ArtifactsSection
        scan={scan}
        scanning={scanning}
        scanError={scanError}
        onRefresh={() => runScan(project.path)}
        openArtifact={async (artifact) => {
          // Use openLocalPath → backend open_path command, which
          // dispatches to the OS default handler (macOS `open`,
          // Windows ShellExecute, Linux xdg-open). For .dmg this
          // mounts + opens Finder; for .app it launches the bundle;
          // for .exe / .AppImage it runs them.
          await openLocalPath(artifact.path);
        }}
      />

      <div className="detail__actions">
        {setupAction && (
          <ActionButton
            action={setupAction}
            disabled={!canRunAction}
            title="Run the project's setup script"
            onClick={() => onSetup(project.id)}
          />
        )}
        {buildAction && (
          <ActionButton
            action={buildAction}
            disabled={!canRunAction}
            title={`Run build (${buildAction.command})`}
            onClick={() =>
              onBuild
                ? onBuild(project.id)
                : onRunAction(project.id, buildAction.name)
            }
          />
        )}
        {userActions.map((a) => (
          <ActionButton
            key={a.name}
            action={a}
            disabled={pending || isActive}
            title={a.command}
            onClick={() => onRunAction(project.id, a.name)}
          />
        ))}
        <button
          type="button"
          className={`btn ${isCrashed ? "btn--warning" : ""}`}
          onClick={() => onStop(project.id)}
          disabled={!canStop}
          title={
            isCrashed
              ? "Clean up crashed application (runs stop script and terminates remaining processes)"
              : "Stop the project (runs stop script or terminates processes)"
          }
        >
          ■ {isCrashed ? "Stop (Cleanup)" : "Stop"}
        </button>
        <button
          type="button"
          className="btn"
          onClick={() => onRestart(project.id)}
          disabled={!canRestart || !hasRestart}
          title={
            hasRestart
              ? `Stop, then run "${project.primaryAction}"`
              : "No primary action configured for Restart"
          }
        >
          ↻ Restart
        </button>
        <button
          type="button"
          className={`btn ${primaryPort ? "btn--accent" : ""}`}
          onClick={handleOpenBrowser}
          title={
            primaryPort
              ? `Open http://localhost:${primaryPort} in your default browser`
              : isActive
                ? "Click to open custom or default localhost port"
                : "Start project or click to open localhost in browser"
          }
        >
          🌐 {primaryPort ? `Open App (:${primaryPort})` : "Open App"}
        </button>


        <label
          className="detail__auto-open-toggle"
          title="Automatically open the app in your browser once its port is detected"
        >
          <input
            type="checkbox"
            checked={autoOpen}
            onChange={(e) => setAutoOpen(e.target.checked)}
          />
          <span>Auto-open</span>
        </label>
      </div>

      <OpenSiteBar
        detectedPorts={detectedPorts}
        onOpen={(port) =>
          openBrowserUrl(`http://localhost:${port}`).catch((e) =>
            console.error("openUrl failed", e)
          )
        }
      />

      <RestartBanner projectId={project.id} />

      <dl className="detail__config">
        <div className="detail__row">
          <dt>Path</dt>
          <dd>
            <code>{project.path}</code>
          </dd>
        </div>
        {project.autoRestart?.enabled && (
          <div className="detail__row">
            <dt>Auto-restart</dt>
            <dd>
              up to <strong>{project.autoRestart.maxRetries}</strong> time
              {project.autoRestart.maxRetries === 1 ? "" : "s"}, delay{" "}
              <strong>{project.autoRestart.delayMs}ms</strong> between attempts
            </dd>
          </div>
        )}
        {project.actions.length > 0 && (
          <div className="detail__row">
            <dt>Actions</dt>
            <dd>
              <table className="env-table">
                <tbody>
                  {project.actions.map((a) => (
                    <tr key={a.name}>
                      <td>
                        <code>{a.name}</code>
                      </td>
                      <td>
                        <code>{a.command}</code>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </dd>
          </div>
        )}
      </dl>

      <section className="detail__panel">
        <header className="detail__panel-header">
          <div>
            <h3>Logs</h3>
            <span className="detail__panel-hint">
              live PTY · click to type input
            </span>
          </div>
          <div className="log-panel__controls">
            <button
              type="button"
              className={`btn btn--tiny${isScrolledUp ? " btn--active" : ""}`}
              onClick={() => logActionsRef.current?.scrollToBottom()}
              title={isScrolledUp ? "Jump to live bottom" : "At bottom"}
            >
              ↓ Bottom{isScrolledUp ? " •" : ""}
            </button>
            <button
              type="button"
              className="btn btn--tiny"
              onClick={handleCopyLogs}
              title="Copy terminal logs to clipboard"
            >
              {copied ? "✓ Copied" : "📋 Copy"}
            </button>
            <button
              type="button"
              className="btn btn--tiny"
              onClick={() => logActionsRef.current?.clear()}
              title="Clear terminal output"
            >
              ⊘ Clear
            </button>
            <button
              type="button"
              className={`btn btn--tiny${logExpanded ? " btn--active" : ""}`}
              onClick={() => setLogExpanded((e) => !e)}
              title={logExpanded ? "Collapse log viewer" : "Expand log viewer"}
            >
              {logExpanded ? "⤓ Normal" : "⤒ Expand"}
            </button>
          </div>
        </header>
        <LogViewer
          projectId={project.id}
          expanded={logExpanded}
          onActionsReady={(actions) => {
            logActionsRef.current = actions;
          }}
          onScrolledUpChange={setIsScrolledUp}
        />
      </section>

      <section className="detail__panel">
        <header className="detail__panel-header">
          <h3>Ports</h3>
          <span className="detail__panel-hint">
            detected listeners · click to open
          </span>
        </header>
        <PortList projectId={project.id} ports={detectedPorts} />
      </section>
    </section>
  );
}

interface ActionButtonProps {
  action: ResolvedAction;
  disabled: boolean;
  title: string;
  onClick: () => void;
}

function ActionButton({ action, disabled, title, onClick }: ActionButtonProps) {
  const isPrimary = action.icon === "run" || action.name.toLowerCase() === "run";
  return (
    <button
      type="button"
      className={`btn ${isPrimary ? "btn--primary" : ""}`}
      onClick={onClick}
      disabled={disabled}
      title={title}
    >
      <span className="btn__icon">{iconGlyph(action.icon)}</span> {action.name}
    </button>
  );
}

/**
 * Prominent "Open site" bar. Enabled when at least one port has
 * been detected; clicking it opens the lowest-numbered port in the
 * user's default browser (via `tauri-plugin-opener`'s `openUrl`).
 *
 * The "lowest port" heuristic picks the main app when a project
 * opens multiple ports (e.g. 5173 for Vite + 8000 for an API):
 * the smaller port is usually the user-facing one.
 */
function OpenSiteBar({
  detectedPorts,
  onOpen,
}: {
  detectedPorts: number[];
  onOpen: (port: number) => void;
}) {
  if (detectedPorts.length === 0) {
    return (
      <div className="detail__open-site detail__open-site--empty">
        <span className="detail__open-site-label">🌐</span>
        <span className="detail__open-site-hint">
          No listening port detected yet. Once the app is up, open it
          here.
        </span>
      </div>
    );
  }
  const primary = Math.min(...detectedPorts);
  const extra = detectedPorts.length - 1;
  return (
    <div className="detail__open-site">
      <button
        type="button"
        className="btn btn--accent"
        onClick={() => onOpen(primary)}
        title={`Open http://localhost:${primary} in your default browser`}
      >
        🌐 Open site · localhost:{primary}
      </button>
      {extra > 0 && (
        <span className="detail__open-site-extras">
          +{extra} other port{extra === 1 ? "" : "s"} (see below)
        </span>
      )}
    </div>
  );
}

/**
 * Transient banner shown briefly when the backend signals an
 * auto-restart. Auto-clears after the configured delay so it doesn't
 * linger once the new process is up.
 */
function RestartBanner({ projectId }: { projectId: string }) {
  const [info, setInfo] = useState<{
    attempt: number;
    max: number;
    delayMs: number;
  } | null>(null);

  useProjectRestarting(
    projectId,
    (attempt, max, delayMs) => {
      setInfo({ attempt, max, delayMs });
    }
  );

  useEffect(() => {
    if (!info) return;
    const t = setTimeout(() => setInfo(null), info.delayMs + 500);
    return () => clearTimeout(t);
  }, [info]);

  if (!info) return null;
  return (
    <div className="detail__restart-banner" role="status">
      <span className="detail__restart-icon">↻</span>
      Crashed — auto-restarting in {(info.delayMs / 1000).toFixed(1)}s
      (attempt {info.attempt}/{info.max})…
    </div>
  );
}

/**
 * Auto-discovered build artifacts section.
 *
 * Shows an "Install" button when an installable artifact is found
 * (`.dmg` on mac, `.msi` on Windows, `.deb` / `.rpm` on Linux) and a
 * "Run Portable" button when a portable one is (`.app`, `.exe`,
 * `.AppImage`). Hides entirely when neither is found so it doesn't
 * add visual noise to projects without builds.
 *
 * The Refresh button re-runs the scan. The parent already auto-scans
 * on selection + after a build completes; this is for manual
 * recovery (e.g., user ran a build outside of VibeRunner).
 */
function ArtifactsSection({
  scan,
  scanning,
  scanError,
  onRefresh,
  openArtifact,
}: {
  scan: ArtifactsScan | null;
  scanning: boolean;
  scanError: string | null;
  onRefresh: () => void;
  openArtifact: (artifact: ProjectArtifact) => Promise<void>;
}) {
  const hasAny = !!(scan?.install || scan?.portable);
  // While the very first scan is in flight, render nothing — we
  // don't want a flash of "no artifacts found" before the scan
  // completes. After the first scan completes, render even if empty
  // so the Refresh button is reachable.
  if (scanning && !scan && !scanError) return null;

  return (
    <section className="detail__artifacts">
      <header className="detail__artifacts-header">
        <div className="detail__artifacts-title">
          <span className="detail__artifacts-label">Discovered artifacts</span>
          <span className="detail__artifacts-hint">
            auto-scanned from the project folder
          </span>
        </div>
        <button
          type="button"
          className="btn btn--tiny"
          onClick={onRefresh}
          disabled={scanning}
          title="Re-parse the project folder and rescan for build artifacts"
          aria-label="Refresh artifacts"
        >
          {scanning ? "…" : "↻ Refresh"}
        </button>
      </header>
      <div className="detail__artifacts-body">
        {scanError && (
          <div className="detail__artifacts-error" role="alert">
            ⚠ {scanError}
          </div>
        )}
        {!scanError && !hasAny && (
          <div className="detail__artifacts-empty">
            No installable or portable artifacts found. Run{" "}
            <code>Build .app</code> / <code>Build .dmg</code> first, or
            drop a built artifact in the project folder.
          </div>
        )}
        {scan?.install && (
          <ArtifactButton
            artifact={scan.install}
            label="Install"
            onClick={() => openArtifact(scan.install!)}
          />
        )}
        {scan?.portable && (
          <ArtifactButton
            artifact={scan.portable}
            label="Run Portable"
            onClick={() => openArtifact(scan.portable!)}
          />
        )}
      </div>
    </section>
  );
}

function ArtifactButton({
  artifact,
  label,
  onClick,
}: {
  artifact: ProjectArtifact;
  label: string;
  onClick: () => void;
}) {
  const sizeMb = (artifact.sizeBytes / (1024 * 1024)).toFixed(1);
  const mtime = new Date(artifact.modifiedMs);
  const dateLabel = mtime.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
  return (
    <button
      type="button"
      className="btn btn--accent detail__artifact-btn"
      onClick={onClick}
      title={`${artifact.path}\n${sizeMb} MB · ${dateLabel}`}
    >
      <span className="detail__artifact-label">{label}</span>
      <span className="detail__artifact-name" title={artifact.displayName}>
        {artifact.displayName}
      </span>
      <span className="detail__artifact-meta">
        {sizeMb} MB · {dateLabel}
      </span>
    </button>
  );
}
