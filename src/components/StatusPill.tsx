import type { ProjectStatus } from "../types";
import { useNow } from "../hooks/useRunnerEvents";
import { formatElapsed, isActiveStatus } from "../utils";

interface StatusPillProps {
  status: ProjectStatus;
  /** Which action's PTY is alive ("Run", "Build (Windows)"). */
  action?: string | null;
  /** Why the run ended ("exit 1") — shown in the tooltip / for crashes. */
  reason?: string | null;
  /** Start time of the active action, for an elapsed timer. */
  startedAtMs?: number | null;
  size?: "sm" | "md";
}

const LABELS: Record<ProjectStatus, string> = {
  stopped: "Stopped",
  starting: "Starting",
  running: "Running",
  stopping: "Stopping",
  crashed: "Crashed",
};

export function StatusPill({ status, action, reason, startedAtMs, size = "sm" }: StatusPillProps) {
  const active = isActiveStatus(status);
  const now = useNow(active && !!startedAtMs);
  const elapsed = active && startedAtMs ? formatElapsed(now - startedAtMs) : null;
  const showReason = !active && reason && size === "md";

  const title = [
    LABELS[status],
    active && action ? action : null,
    elapsed ? `for ${elapsed}` : null,
    reason ? `(${reason})` : null,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <span className={`status-pill status-pill--${size} status-pill--${status}`} title={title}>
      <span className="status-pill__dot" />
      <span className="status-pill__label">{LABELS[status]}</span>
      {active && action && <span className="status-pill__action">{action}</span>}
      {elapsed && <span className="status-pill__elapsed">{elapsed}</span>}
      {showReason && <span className="status-pill__reason">{reason}</span>}
    </span>
  );
}
