import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  ProjectStatus,
  ResolvedProject,
  VibeConfigReloadedPayload,
} from "../types";
import { StatusPill } from "./StatusPill";
import { openLocalPath, visibleProjectWarnings } from "../utils";

interface ProjectCardProps {
  project: ResolvedProject;
  status: ProjectStatus;
  currentAction: string | null;
  ports?: number[];
  selected: boolean;
  /** Compact view: avatar + name + status pill only. */
  compact?: boolean;
  onSelect: () => void;
  onConfigReloaded: (payload: VibeConfigReloadedPayload) => void;
  onRemoveProject: (id: string) => void;
  /** True if the project is currently active — disallows removal. */
  busy?: boolean;
  /** Optional callback to surface an error to the app-level banner. */
  onError?: (msg: string) => void;
}

/**
 * Stable, visually-distinct hue derived from the project id. The same
 * project always gets the same color, so the sidebar reads as a list
 * of identifiable chips rather than a rainbow.
 */
function avatarStyle(id: string): { background: string; color: string } {
  let h = 0;
  for (let i = 0; i < id.length; i++) {
    h = (h * 31 + id.charCodeAt(i)) >>> 0;
  }
  // Spread hues across the wheel but keep saturation/lightness in a
  // tasteful band so every chip stays legible.
  const hue = h % 360;
  return {
    background: `hsl(${hue}, 55%, 42%)`,
    color: "#ffffff",
  };
}

function avatarLetter(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return "?";
  // First non-whitespace, non-symbol character — works for emoji-less
  // names like "AI-Agent-Engineer" while still showing A.
  for (const ch of trimmed) {
    if (/[\p{L}\p{N}]/u.test(ch)) return ch.toUpperCase();
  }
  return trimmed[0].toUpperCase();
}

export function ProjectCard({
  project,
  status,
  currentAction,
  ports: _ports = [],
  selected,
  compact = false,
  onSelect,
  onConfigReloaded,
  onRemoveProject,
  busy = false,
  onError,
}: ProjectCardProps) {
  const [removing, setRemoving] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const warnings = visibleProjectWarnings(project.warnings);

  const handleRemove = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (busy) return;
    if (!confirming) {
      setConfirming(true);
      setTimeout(() => setConfirming(false), 3000);
      return;
    }
    setRemoving(true);
    // Optimistically remove from UI immediately
    onRemoveProject(project.id);
    try {
      const payload = await invoke<VibeConfigReloadedPayload>("remove_project", { id: project.id });
      onConfigReloaded(payload);
    } catch (e) {
      console.error("remove_project failed", e);
      onError?.(typeof e === "string" ? e : String(e));
    } finally {
      setRemoving(false);
      setConfirming(false);
    }
  };

  const handleOpenInFinder = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await openLocalPath(project.path);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error("openLocalPath failed", project.path, msg);
      onError?.(
        `Couldn't open ${project.path} in Finder: ${msg}\n` +
          (msg.toLowerCase().includes("acl")
            ? "Tip: this folder has a macOS ACL xattr. " +
              "Try `xattr -d com.apple.macl " +
              project.path +
              "` in Terminal, or move the folder out of Desktop/Documents."
            : "")
      );
    }
  };

  const handleOpenConfig = async (e: React.MouseEvent) => {
    e.stopPropagation();
    let target = project.path;
    if (project.source === "toml") {
      const tomlPath = `${project.path}/.codex/environments/environment.toml`;
      target = tomlPath;
    }
    try {
      await openLocalPath(target);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error("openLocalPath failed", target, msg);
      onError?.(`Couldn't open ${target}: ${msg}`);
    }
  };

  const sourceLabel =
    project.source === "toml"
      ? "TOML"
      : project.source === "manual"
        ? "Manual"
        : "Empty";

  // Compact action summary: "Run · Stop" or "Setup · Run · Stop"
  const actionSummary = project.actions
    .map((a) => a.name)
    .join(" · ");

  const avatar = (
    <span
      className="runner-card__avatar"
      style={avatarStyle(project.id)}
      aria-hidden="true"
    >
      {avatarLetter(project.name)}
    </span>
  );

  const statusPill = (
    <StatusPill status={status} action={currentAction} />
  );

  if (compact) {
    // Single-row layout: [avatar] name ........... [status]
    // Pure click-to-select. No URL buttons, no action buttons, no path.
    return (
      <div
        role="button"
        tabIndex={0}
        className={`runner-card runner-card--compact${selected ? " runner-card--selected" : ""}`}
        onClick={onSelect}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onSelect();
          }
        }}
        title={project.name}
      >
        {avatar}
        <span className="runner-card__name runner-card__name--compact" title={project.name}>
          {project.name}
        </span>
        <div className="runner-card__status-group">{statusPill}</div>
      </div>
    );
  }

  return (
    <div
      role="button"
      tabIndex={0}
      className={`runner-card${selected ? " runner-card--selected" : ""}${
        busy ? " runner-card--busy" : ""
      }`}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <div className="runner-card__header">
        {avatar}
        <span className="runner-card__name" title={project.name}>{project.name}</span>
        <div className="runner-card__status-group">{statusPill}</div>
      </div>
      <code className="runner-card__command">{actionSummary || "(no actions)"}</code>
      {warnings.length > 0 && (
        <div
          className="runner-card__warning"
          title={warnings.join("\n")}
        >
          ⚠ {warnings[0]}
        </div>
      )}
      <div className="runner-card__footer">
        <div className="runner-card__cwd" title={project.path}>
          <span className="runner-card__path">{project.path}</span>
          <span className="runner-card__sep">·</span>
          <span className="runner-card__source">{sourceLabel}</span>
        </div>
        <div className="runner-card__actions">
          <button
            type="button"
            className="runner-card__action-btn"
            onClick={handleOpenInFinder}
            title={`Open ${project.path} in Finder`}
            aria-label="Open in Finder"
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z"/></svg>
          </button>
          <button
            type="button"
            className="runner-card__action-btn"
            onClick={handleOpenConfig}
            title={
              project.source === "toml"
                ? "Open environment.toml in your default editor"
                : "Open the project folder"
            }
            aria-label="Open config"
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
          </button>
          <button
            type="button"
            className={`runner-card__action-btn runner-card__action-btn--danger${
              confirming ? " runner-card__action-btn--confirm" : ""
            }`}
            onClick={handleRemove}
            disabled={removing || busy}
            title={
              busy
                ? "Cannot remove a running project"
                : confirming
                  ? "Click again to confirm"
                  : "Remove this project"
            }
            aria-label="Remove project"
          >
            {removing ? (
              "…"
            ) : confirming ? (
              "Confirm?"
            ) : (
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/></svg>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
