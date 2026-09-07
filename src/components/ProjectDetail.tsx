import { useEffect, useRef, useState } from "react";
import type {
  ProjectStatus,
  ResolvedAction,
  ResolvedProject,
} from "../types";
import { StatusPill } from "./StatusPill";
import { LogViewer, type LogViewerHandle } from "./LogViewer";
import { PortList } from "./PortList";
import { useProjectRestarting } from "../hooks/useRunnerEvents";
import { openBrowserUrl } from "../utils";

interface ProjectDetailProps {
  project: ResolvedProject | null;
  status: ProjectStatus;
  currentAction: string | null;
  /** Currently-detected listening ports for the selected project. */
  detectedPorts: number[];
  pending: boolean;
  onRunAction: (projectId: string, actionName: string) => void;
  onSetup: (projectId: string) => void;
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
  const canRunAction = !pending && !isActive;
  const canStop = !pending && isActive;
  const canRestart = !pending;

  // The implicit "Setup" action is the first one with name "Setup".
  const setupAction = project.actions.find((a) => a.name === "Setup") ?? null;
  // User-defined actions: everything except the implicit Setup and Stop (which has its own dedicated button).
  const userActions = project.actions.filter(
    (a) => a.name !== "Setup" && a.name.toLowerCase() !== "stop" && a.icon !== "stop"
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
  const logActionsRef = useRef<LogViewerHandle | null>(null);

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

      <div className="detail__actions">
        {setupAction && (
          <ActionButton
            action={setupAction}
            disabled={!canRunAction}
            title="Run the project's setup script"
            onClick={() => onSetup(project.id)}
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
          className="btn"
          onClick={() => onStop(project.id)}
          disabled={!canStop}
          title="Stop the project (runs stop script or terminates processes)"
        >
          ■ Stop
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
              className="btn btn--tiny"
              onClick={() => logActionsRef.current?.scrollToBottom()}
              title="Scroll to bottom"
            >
              ↓ Bottom
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
