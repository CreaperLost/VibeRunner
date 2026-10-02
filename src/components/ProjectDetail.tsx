import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ArtifactsScan,
  ProjectArtifact,
  ProjectRuntime,
  ResolvedAction,
  ResolvedProject,
} from "../types";
import { StatusPill } from "./StatusPill";
import { LogViewer, type LogViewerHandle } from "./LogViewer";
import { PortList } from "./PortList";
import { Icon, actionIcon } from "./Icon";
import { Avatar } from "./ProjectCard";
import { useProjectRestarting } from "../hooks/useRunnerEvents";
import { usePersistentState } from "../hooks/usePersistentState";
import {
  buildAction,
  errorText,
  formatBytes,
  isActiveStatus,
  openBrowserUrl,
  openLocalPath,
  revealLocalPath,
  setupAction,
  stopAction,
  visibleProjectWarnings,
} from "../utils";

interface ProjectDetailProps {
  project: ResolvedProject | null;
  runtime: ProjectRuntime;
  pending: boolean;
  onRunAction: (projectId: string, actionName: string) => void;
  onSetup: (projectId: string) => void;
  onBuild: (projectId: string) => void;
  onStop: (projectId: string) => void;
  onRestart: (projectId: string) => void;
  onError: (msg: string) => void;
}

type Tab = "logs" | "ports" | "details";

/** Runs whose app was already auto-opened (`id:startedAtMs`). Module
 *  level so switching projects back and forth doesn't re-open tabs. */
const autoOpenedRuns = new Set<string>();

/**
 * Right-hand panel. The wrapper only decides between the empty state
 * and the real view; the real view is keyed by project id, so every
 * hook runs unconditionally and per-project state resets on switch.
 * (Returning early before hooks crashed React when the last project
 * was removed.)
 */
export function ProjectDetail(props: ProjectDetailProps) {
  if (!props.project) {
    return (
      <section className="detail detail--empty">
        <div className="detail__empty-msg">
          <Icon name="terminal" size={28} />
          <p>Select a project to see its actions and logs.</p>
        </div>
      </section>
    );
  }
  return <ProjectView key={props.project.id} {...props} project={props.project} />;
}

function ProjectView({
  project,
  runtime,
  pending,
  onRunAction,
  onSetup,
  onBuild,
  onStop,
  onRestart,
  onError,
}: ProjectDetailProps & { project: ResolvedProject }) {
  const { status, ports } = runtime;
  const isActive = isActiveStatus(status);
  const isCrashed = status === "crashed";
  const warnings = visibleProjectWarnings(project.warnings);
  const canRun = !pending && !isActive;

  const setup = setupAction(project);
  const build = buildAction(project);
  const stop = stopAction(project);
  const primary = project.actions.find((a) => a.name === project.primaryAction) ?? null;
  const special = new Set([setup?.name, build?.name, stop?.name, primary?.name]);
  const others = project.actions.filter((a) => !special.has(a.name));
  const appPort = ports[0] ?? null;

  const [tab, setTab] = usePersistentState<Tab>("viberunner.detail.tab", "logs");
  const [logMax, setLogMax] = useState(false);
  const [copied, setCopied] = useState(false);
  const [isScrolledUp, setIsScrolledUp] = useState(false);
  const logRef = useRef<LogViewerHandle | null>(null);

  // ---- auto-open -----------------------------------------------------------
  // Only for the primary (Run) action, once per run. Build/Setup runs
  // and port changes mid-run never open a browser tab.
  const [autoOpen, setAutoOpen] = usePersistentState<boolean>("viberunner.autoOpen", true);
  useEffect(() => {
    if (!autoOpen || status !== "running" || !appPort || !runtime.startedAtMs) return;
    if (!primary || runtime.action !== primary.name) return;
    const key = `${project.id}:${runtime.startedAtMs}`;
    if (autoOpenedRuns.has(key)) return;
    autoOpenedRuns.add(key);
    openBrowserUrl(`http://localhost:${appPort}`).catch((e) => console.error("auto-open failed", e));
  }, [autoOpen, status, appPort, runtime.startedAtMs, runtime.action, primary, project.id]);

  // ---- artifacts -------------------------------------------------------------
  const [scan, setScan] = useState<ArtifactsScan | null>(null);
  const [scanning, setScanning] = useState(false);
  const scanSeq = useRef(0);
  const runScan = useCallback(async () => {
    const seq = ++scanSeq.current;
    setScanning(true);
    try {
      const result = await invoke<ArtifactsScan>("scan_project_artifacts", { path: project.path });
      if (seq === scanSeq.current) setScan(result);
    } catch {
      if (seq === scanSeq.current) setScan(null);
    } finally {
      if (seq === scanSeq.current) setScanning(false);
    }
  }, [project.path]);

  useEffect(() => {
    runScan();
  }, [runScan]);

  // Re-scan when a run finishes (a Build/Package just produced files).
  const wasActive = useRef(isActive);
  useEffect(() => {
    if (wasActive.current && !isActive) runScan();
    wasActive.current = isActive;
  }, [isActive, runScan]);

  // …and on config reloads (Reload button, file watcher, add/remove).
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    listen("config:reloaded", () => runScan()).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [runScan]);

  const openArtifact = async (a: ProjectArtifact) => {
    try {
      await openLocalPath(a.path);
    } catch (e) {
      onError(`Could not open ${a.displayName}: ${errorText(e)}`);
    }
  };

  const openApp = async () => {
    let port = appPort;
    if (!port) {
      const guess = project.env?.["PORT"] || project.env?.["VITE_PORT"] || "3000";
      const answer = window.prompt("No listening port detected yet.\nLocalhost port to open:", guess);
      const n = answer ? parseInt(answer.trim(), 10) : NaN;
      if (!(n > 0 && n <= 65535)) return;
      port = n;
    }
    openBrowserUrl(`http://localhost:${port}`).catch((e) => onError(errorText(e)));
  };

  const copyLogs = async () => {
    if (await logRef.current?.copyLogs()) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    }
  };

  const reveal = () =>
    revealLocalPath(project.path).catch((e) => onError(`Could not reveal ${project.path}: ${errorText(e)}`));

  const actionButton = (a: ResolvedAction, onClick: () => void, variant = "") => (
    <button
      key={a.name}
      type="button"
      className={`btn ${variant}`}
      onClick={onClick}
      disabled={!canRun}
      title={a.command}
    >
      <Icon name={actionIcon(a.icon)} size={14} />
      {a.name}
    </button>
  );

  return (
    <section className={`detail${logMax ? " detail--log-max" : ""}`}>
      <header className="detail__header">
        <div className="detail__title-row">
          <Avatar id={project.id} name={project.name} size="lg" />
          <div className="detail__title-text">
            <h2 className="detail__title">{project.name}</h2>
            <div className="detail__subtitle">
              <button type="button" className="detail__path" onClick={reveal} title="Show in file manager">
                <Icon name="folder" size={12} />
                <span>{project.path}</span>
              </button>
              <span className={`badge badge--${project.source}`}>
                {project.source === "toml" ? "TOML" : project.source === "manual" ? "Manual" : "Empty"}
              </span>
            </div>
          </div>
          <StatusPill
            status={status}
            action={runtime.action}
            reason={runtime.reason}
            startedAtMs={runtime.startedAtMs}
            size="md"
          />
        </div>

        <div className="toolbar" role="toolbar" aria-label="Project actions">
          <div className="toolbar__group">
            {primary && actionButton(primary, () => onRunAction(project.id, primary.name), "btn--primary")}
            <button
              type="button"
              className={`btn ${isCrashed ? "btn--warning" : isActive ? "btn--danger" : ""}`}
              onClick={() => onStop(project.id)}
              disabled={pending || !(isActive || isCrashed) || status === "stopping"}
              title={
                isCrashed
                  ? "Clean up leftovers of the crashed run (runs the stop script, kills remaining processes)"
                  : stop
                    ? `Stop (${stop.command})`
                    : "Stop (Ctrl+C, then terminate the process tree)"
              }
            >
              <Icon name="stop" size={13} />
              {isCrashed ? "Clean up" : "Stop"}
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => onRestart(project.id)}
              disabled={pending || !primary || status === "stopping"}
              title={primary ? `Stop, run Setup if any, then "${primary.name}"` : "No primary action to restart"}
            >
              <Icon name="restart" size={14} />
              Restart
            </button>
          </div>

          {(setup || build || others.length > 0) && (
            <div className="toolbar__group">
              {setup && actionButton(setup, () => onSetup(project.id))}
              {build && actionButton(build, () => onBuild(project.id))}
              {others.map((a) => actionButton(a, () => onRunAction(project.id, a.name)))}
            </div>
          )}

          <div className="toolbar__spacer" />

          <div className="toolbar__group">
            <button
              type="button"
              className={`btn ${appPort ? "btn--accent" : ""}`}
              onClick={openApp}
              title={appPort ? `Open http://localhost:${appPort}` : "Open a localhost port in your browser"}
            >
              <Icon name="globe" size={14} />
              {appPort ? `localhost:${appPort}` : "Open app"}
            </button>
            <label className="switch" title="Open the app in your browser once, when Run first reports its URL">
              <input type="checkbox" checked={autoOpen} onChange={(e) => setAutoOpen(e.target.checked)} />
              <span className="switch__track" />
              <span className="switch__label">Auto-open</span>
            </label>
          </div>
        </div>

        {warnings.length > 0 && (
          <div className="callout callout--warning" role="alert">
            <Icon name="alert" size={14} />
            <div>
              {warnings.map((w, i) => (
                <div key={i}>{w}</div>
              ))}
            </div>
          </div>
        )}

        <RestartBanner projectId={project.id} />

        <ArtifactsBar scan={scan} scanning={scanning} onRefresh={runScan} onOpen={openArtifact} />
      </header>

      <nav className="tabs" role="tablist">
        {(
          [
            ["logs", "Logs", "terminal"],
            ["ports", `Ports${ports.length ? ` · ${ports.length}` : ""}`, "plug"],
            ["details", "Details", "info"],
          ] as const
        ).map(([id, label, icon]) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            className={`tabs__tab${tab === id ? " tabs__tab--active" : ""}`}
            onClick={() => setTab(id)}
          >
            <Icon name={icon} size={14} />
            {label}
          </button>
        ))}
        {tab === "logs" && (
          <div className="tabs__tools">
            <button
              type="button"
              className={`icon-btn${isScrolledUp ? " icon-btn--active" : ""}`}
              onClick={() => logRef.current?.scrollToBottom()}
              title={isScrolledUp ? "Jump to live output" : "Following live output"}
            >
              <Icon name="arrowDown" size={14} />
            </button>
            <button type="button" className="icon-btn" onClick={copyLogs} title="Copy logs">
              <Icon name={copied ? "check" : "copy"} size={14} />
            </button>
            <button type="button" className="icon-btn" onClick={() => logRef.current?.clear()} title="Clear logs">
              <Icon name="eraser" size={14} />
            </button>
            <button
              type="button"
              className={`icon-btn${logMax ? " icon-btn--active" : ""}`}
              onClick={() => setLogMax((v) => !v)}
              title={logMax ? "Restore layout" : "Maximize logs"}
            >
              <Icon name={logMax ? "minimize" : "maximize"} size={14} />
            </button>
          </div>
        )}
      </nav>

      <div className="detail__panel" role="tabpanel">
        {/* Kept mounted so the terminal keeps its scrollback while other tabs are open. */}
        <div className={`detail__logs${tab === "logs" ? "" : " is-hidden"}`}>
          <LogViewer
            projectId={project.id}
            expanded={logMax || tab === "logs"}
            onActionsReady={(a) => {
              logRef.current = a;
            }}
            onScrolledUpChange={setIsScrolledUp}
          />
        </div>
        {tab === "ports" && (
          <div className="detail__scroll">
            <PortList ports={ports} active={isActive} />
          </div>
        )}
        {tab === "details" && (
          <div className="detail__scroll">
            <DetailsTable project={project} />
          </div>
        )}
      </div>
    </section>
  );
}

function DetailsTable({ project }: { project: ResolvedProject }) {
  const envKeys = Object.keys(project.env ?? {});
  return (
    <dl className="kv">
      <dt>Path</dt>
      <dd>
        <code>{project.path}</code>
      </dd>
      <dt>Source</dt>
      <dd>
        {project.source === "toml"
          ? ".codex/environments/environment.toml"
          : project.source === "manual"
            ? "vibe.config.json (manual)"
            : "no actions configured"}
      </dd>
      <dt>Restart runs</dt>
      <dd>{project.primaryAction ?? "—"}</dd>
      {project.autoRestart?.enabled && (
        <>
          <dt>Auto-restart</dt>
          <dd>
            up to {project.autoRestart.maxRetries}×, {project.autoRestart.delayMs} ms apart
          </dd>
        </>
      )}
      {envKeys.length > 0 && (
        <>
          <dt>Env overrides</dt>
          <dd>
            <code>{envKeys.join(", ")}</code>
          </dd>
        </>
      )}
      {project.warnings.length > 0 && (
        <>
          <dt>Notes</dt>
          <dd>
            {project.warnings.map((w, i) => (
              <div key={i}>{w}</div>
            ))}
          </dd>
        </>
      )}
      <dt>Actions</dt>
      <dd>
        <table className="actions-table">
          <tbody>
            {project.actions.map((a) => (
              <tr key={a.name}>
                <td className="actions-table__name">
                  <Icon name={actionIcon(a.icon)} size={13} />
                  {a.name}
                  {a.detached && <span className="badge">detached</span>}
                </td>
                <td>
                  <code>{a.command}</code>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </dd>
    </dl>
  );
}

/** Transient banner while an auto-restart is pending. */
function RestartBanner({ projectId }: { projectId: string }) {
  const [info, setInfo] = useState<{ attempt: number; max: number; delayMs: number } | null>(null);
  const onRestarting = useCallback(
    (attempt: number, max: number, delayMs: number) => setInfo({ attempt, max, delayMs }),
    []
  );
  useProjectRestarting(projectId, onRestarting);

  useEffect(() => {
    if (!info) return;
    const t = setTimeout(() => setInfo(null), info.delayMs + 500);
    return () => clearTimeout(t);
  }, [info]);

  if (!info) return null;
  return (
    <div className="callout callout--info" role="status">
      <Icon name="restart" size={14} />
      Crashed — auto-restarting in {(info.delayMs / 1000).toFixed(1)}s (attempt {info.attempt}/{info.max})
    </div>
  );
}

/**
 * Discovered build outputs: best installer + best runnable app, with
 * runners-up in an overflow menu. Hidden when nothing was found.
 */
function ArtifactsBar({
  scan,
  scanning,
  onRefresh,
  onOpen,
}: {
  scan: ArtifactsScan | null;
  scanning: boolean;
  onRefresh: () => void;
  onOpen: (a: ProjectArtifact) => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  if (!scan || (!scan.install && !scan.portable)) return null;

  const chip = (a: ProjectArtifact, label: string, icon: "download" | "play") => (
    <button
      type="button"
      className="artifact"
      onClick={() => onOpen(a)}
      title={`${a.path}\n${formatBytes(a.sizeBytes)} · ${new Date(a.modifiedMs).toLocaleString()}`}
    >
      <Icon name={icon} size={13} />
      <span className="artifact__label">{label}</span>
      <span className="artifact__name">{a.displayName}</span>
      <span className="artifact__meta">
        {formatBytes(a.sizeBytes)} · {new Date(a.modifiedMs).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
      </span>
    </button>
  );

  return (
    <div className="artifacts">
      <Icon name="package" size={14} className="artifacts__icon" />
      {scan.install && chip(scan.install, "Install", "download")}
      {scan.portable && chip(scan.portable, "Run", "play")}
      {scan.others.length > 0 && (
        <div className="menu">
          <button
            type="button"
            className="icon-btn"
            onClick={() => setMenuOpen((v) => !v)}
            onBlur={() => setTimeout(() => setMenuOpen(false), 150)}
            title="Other build outputs"
          >
            <Icon name="chevronDown" size={14} />
          </button>
          {menuOpen && (
            <div className="menu__list" role="menu">
              {scan.others.map((a) => (
                <button key={a.path} type="button" role="menuitem" className="menu__item" onClick={() => onOpen(a)}>
                  <Icon name={a.kind === "install" ? "download" : "play"} size={13} />
                  <span className="menu__item-name">{a.displayName}</span>
                  <span className="menu__item-meta">{a.parent}</span>
                </button>
              ))}
            </div>
          )}
        </div>
      )}
      <button type="button" className="icon-btn" onClick={onRefresh} disabled={scanning} title="Rescan the project folder">
        <Icon name="refresh" size={14} className={scanning ? "spin" : undefined} />
      </button>
    </div>
  );
}
