import type { ProjectStatus } from "../types";

interface StatusPillProps {
  status: ProjectStatus;
  /** Optional suffix like "Run" or "Setup" to label which action is alive. */
  action?: string | null;
  size?: "sm" | "md";
}

const LABELS: Record<ProjectStatus, string> = {
  stopped: "Stopped",
  starting: "Starting…",
  running: "Running",
  stopping: "Stopping…",
  crashed: "Crashed",
};

const DOT_COLORS: Record<ProjectStatus, string> = {
  stopped: "var(--c-muted)",
  starting: "var(--c-warning)",
  running: "var(--c-success)",
  stopping: "var(--c-warning-strong, #b45309)",
  crashed: "var(--c-danger)",
};

export function StatusPill({ status, action, size = "sm" }: StatusPillProps) {
  // Show "Running Run", "Running Setup" so the user knows which
  // action's PTY they're looking at when a project can run multiple.
  const showAction =
    (status === "running" || status === "starting" || status === "stopping") &&
    action;
  return (
    <span className={`status-pill status-pill--${size} status-pill--${status}`}>
      <span
        className="status-pill__dot"
        style={{ background: DOT_COLORS[status] }}
      />
      {LABELS[status]}
      {showAction && <span className="status-pill__action">{action}</span>}
    </span>
  );
}
