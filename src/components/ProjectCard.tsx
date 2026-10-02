import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ProjectRuntime, ResolvedProject, VibeConfigReloadedPayload } from "../types";
import { StatusPill } from "./StatusPill";
import { Icon } from "./Icon";
import { errorText, isActiveStatus, openLocalPath, visibleProjectWarnings } from "../utils";

interface ProjectCardProps {
  project: ResolvedProject;
  runtime: ProjectRuntime;
  selected: boolean;
  /** Compact view: avatar + name + status only. */
  compact?: boolean;
  onSelect: () => void;
  onConfigReloaded: (payload: VibeConfigReloadedPayload) => void;
  onError?: (msg: string) => void;
}

/** Stable hue derived from the project id. */
function hueOf(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return h % 360;
}

function avatarLetter(name: string): string {
  for (const ch of name.trim()) {
    if (/[\p{L}\p{N}]/u.test(ch)) return ch.toUpperCase();
  }
  return name.trim()[0]?.toUpperCase() ?? "?";
}

export function Avatar({ id, name, size = "md" }: { id: string; name: string; size?: "md" | "lg" }) {
  const hue = hueOf(id);
  return (
    <span
      className={`avatar avatar--${size}`}
      style={{
        background: `linear-gradient(135deg, hsl(${hue} 62% 52%), hsl(${(hue + 40) % 360} 58% 40%))`,
      }}
      aria-hidden="true"
    >
      {avatarLetter(name)}
    </span>
  );
}

export function ProjectCard({
  project,
  runtime,
  selected,
  compact = false,
  onSelect,
  onConfigReloaded,
  onError,
}: ProjectCardProps) {
  const [removing, setRemoving] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const warnings = visibleProjectWarnings(project.warnings);
  const busy = isActiveStatus(runtime.status);

  const handleRemove = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (busy) return;
    if (!confirming) {
      setConfirming(true);
      setTimeout(() => setConfirming(false), 3000);
      return;
    }
    setRemoving(true);
    try {
      const payload = await invoke<VibeConfigReloadedPayload>("remove_project", { id: project.id });
      onConfigReloaded(payload);
    } catch (err) {
      onError?.(`Could not remove ${project.name}: ${errorText(err)}`);
    } finally {
      setRemoving(false);
      setConfirming(false);
    }
  };

  const open = (target: string) => async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await openLocalPath(target);
    } catch (err) {
      const msg = errorText(err);
      onError?.(
        `Couldn't open ${target}: ${msg}` +
          (msg.toLowerCase().includes("acl")
            ? `\nTip: this folder has a macOS ACL xattr. Try \`xattr -d com.apple.macl ${target}\`.`
            : "")
      );
    }
  };

  const tomlPath = `${project.path}/.codex/environments/environment.toml`;
  const pill = (
    // The action name lives in the detail header; here it would squeeze
    // the project name to nothing.
    <StatusPill status={runtime.status} reason={runtime.reason} startedAtMs={compact ? null : runtime.startedAtMs} />
  );

  const common = {
    role: "button" as const,
    tabIndex: 0,
    onClick: onSelect,
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        onSelect();
      }
    },
    "aria-current": selected || undefined,
  };

  if (compact) {
    return (
      <div {...common} className={`card card--compact${selected ? " card--selected" : ""}`} title={project.name}>
        <Avatar id={project.id} name={project.name} />
        <span className="card__name">{project.name}</span>
        {pill}
      </div>
    );
  }

  return (
    <div {...common} className={`card${selected ? " card--selected" : ""}${busy ? " card--busy" : ""}`}>
      <div className="card__row">
        <Avatar id={project.id} name={project.name} />
        <div className="card__main">
          <span className="card__name" title={project.name}>
            {project.name}
          </span>
          <span className="card__path" title={project.path}>
            {project.path}
          </span>
        </div>
        {pill}
      </div>

      {warnings.length > 0 && (
        <div className="card__warning" title={warnings.join("\n")}>
          <Icon name="alert" size={12} />
          <span>{warnings[0]}</span>
        </div>
      )}

      <div className="card__footer">
        <div className="card__chips">
          {runtime.ports.slice(0, 2).map((p) => (
            <span key={p} className="chip chip--port">
              :{p}
            </span>
          ))}
          <span className="chip">{project.source === "toml" ? "TOML" : project.source === "manual" ? "Manual" : "Empty"}</span>
          {/* Ports matter more while running; the count would just get clipped. */}
          {runtime.ports.length < 2 && (
            <span className="chip chip--muted">
              {project.actions.length} action{project.actions.length === 1 ? "" : "s"}
            </span>
          )}
        </div>
        <div className="card__actions">
          <button type="button" className="icon-btn icon-btn--sm" onClick={open(project.path)} title="Open folder">
            <Icon name="folder" size={13} />
          </button>
          {project.source === "toml" && (
            <button type="button" className="icon-btn icon-btn--sm" onClick={open(tomlPath)} title="Open environment.toml">
              <Icon name="settings" size={13} />
            </button>
          )}
          <button
            type="button"
            className={`icon-btn icon-btn--sm icon-btn--danger${confirming ? " icon-btn--confirm" : ""}`}
            onClick={handleRemove}
            disabled={removing || busy}
            title={busy ? "Stop the project before removing it" : confirming ? "Click again to remove" : "Remove project"}
          >
            {confirming ? <span className="icon-btn__text">Remove?</span> : <Icon name="trash" size={13} />}
          </button>
        </div>
      </div>
    </div>
  );
}
