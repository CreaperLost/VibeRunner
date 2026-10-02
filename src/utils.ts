import { invoke } from "@tauri-apps/api/core";
import { openUrl as pluginOpenUrl } from "@tauri-apps/plugin-opener";
import type { ProjectStatus, ResolvedAction, ResolvedProject } from "./types";

/** Platform filtering is expected; only show actionable project warnings. */
export function visibleProjectWarnings(warnings: string[]): string[] {
  return warnings.filter((warning) => !/^hidden on \S+ \(platform mismatch\):/.test(warning));
}

export function isActiveStatus(s: ProjectStatus): boolean {
  return s === "running" || s === "starting" || s === "stopping";
}

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

// Action roles. These mirror `config::stop_action` / `build_action` in
// the backend — first icon match wins, then name.

export function stopAction(project: ResolvedProject): ResolvedAction | null {
  return (
    project.actions.find((a) => a.icon === "stop") ??
    project.actions.find((a) => a.name.toLowerCase() === "stop") ??
    null
  );
}

export function buildAction(project: ResolvedProject): ResolvedAction | null {
  const candidates = project.actions.filter((a) => a.name !== "Setup");
  return (
    candidates.find((a) => a.icon === "build") ??
    candidates.find((a) => a.name.toLowerCase() === "build") ??
    null
  );
}

export function setupAction(project: ResolvedProject): ResolvedAction | null {
  return project.actions.find((a) => a.name === "Setup") ?? null;
}

/** "4s", "2m 05s", "1h 03m" */
export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${String(m % 60).padStart(2, "0")}m`;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Open a URL in the user's default browser.
 * Uses the backend `open_url` command directly to bypass plugin sandbox/capability scopes.
 */
export async function openBrowserUrl(url: string): Promise<void> {
  try {
    await invoke("open_url", { url });
  } catch (e) {
    console.warn("open_url command failed, falling back to plugin:", e);
    await pluginOpenUrl(url);
  }
}

/** Open a local file or directory with the OS default application. */
export async function openLocalPath(path: string): Promise<void> {
  await invoke("open_path", { path });
}

/** Show a file or folder, selected, in the system file manager. */
export async function revealLocalPath(path: string): Promise<void> {
  await invoke("reveal_in_finder", { path });
}
