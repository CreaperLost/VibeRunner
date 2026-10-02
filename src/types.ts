// Mirrors the Rust config types in `src-tauri/src/config.rs` and the
// event payloads in `src-tauri/src/events.rs`. Keep these in sync —
// they're the wire format between Rust and the UI.

// =============================================================================
// vibe.config.json (on disk)
// =============================================================================

/** A project's auto-restart policy (optional). */
export interface RestartPolicy {
  enabled: boolean;
  maxRetries: number;
  delayMs: number;
}

/** Inline shell command for manual projects. */
export interface ManualCommand {
  command: string;
}

/** Inline action for manual projects. */
export interface ActionConfig {
  name: string;
  icon?: string | null;
  command: string;
  /**
   * `true` for actions whose command launches a background process
   * and returns quickly (e.g. `nohup start.sh &`). When set,
   * VibeRunner tracks the whole process tree rooted at the PTY —
   * the project stays "Running" as long as any descendant is alive.
   * Default `false`.
   */
  detached?: boolean;
  /**
   * Restrict this action to one OS: `windows`, `unix`, `macos`,
   * `linux`, or `any`. Absent/null means available everywhere.
   *
   * This is load-bearing, not cosmetic: Restart and Stop resolve
   * their action by first `icon` match, so a cross-platform project
   * whose per-OS actions share an `icon` must scope them or Windows
   * will pick the bash variant.
   */
  platform?: string | null;
}

/**
 * One entry in `vibe.config.json`. Either auto-discovered (pointed
 * at a folder; the TOML inside supplies the actions) or fully manual
 * (inline actions).
 */
export interface ProjectConfig {
  id: string;
  path: string;
  name?: string | null;
  /** Action the built-in Restart should run. If unset, the resolver
   *  picks the first action with name="Run" or icon="run". */
  primaryAction?: string | null;
  /** Skip TOML discovery and use the inline setup + actions. */
  manual: boolean;
  setup?: ManualCommand | null;
  build?: ManualCommand | null;
  actions: ActionConfig[];
  env: Record<string, string>;
  autoRestart?: RestartPolicy | null;
}

export interface VibeConfig {
  version: number;
  projects: ProjectConfig[];
}

export interface VibeConfigPath {
  path: string;
}

// =============================================================================
// Resolved (after merging TOML in)
// =============================================================================

export interface ResolvedAction {
  name: string;
  icon?: string | null;
  command: string;
  /** See `ActionConfig.detached`. Propagated from the source. */
  detached?: boolean;
}

export type ProjectSource = "toml" | "manual" | "empty";

export interface ResolvedProject {
  id: string;
  name: string;
  path: string;
  source: ProjectSource;
  /** All buttons, including the implicit "Setup" if one is configured. */
  actions: ResolvedAction[];
  /** Action name the built-in Restart should run. */
  primaryAction: string | null;
  /** Non-fatal issues — e.g., "path does not exist", "no TOML found". */
  warnings: string[];
  autoRestart: RestartPolicy | null;
  /** Configured environment variable overrides. */
  env?: Record<string, string>;
}

// =============================================================================
// Status
// =============================================================================

/** Runtime status of a project. Mirrors `Status` in `runner.rs`. */
export type ProjectStatus =
  | "stopped"
  | "starting"
  | "running"
  | "stopping"
  | "crashed";

// =============================================================================
// Event payloads (mirror src-tauri/src/events.rs)
// =============================================================================

export interface StatusPayload {
  id: string;
  status: ProjectStatus;
  /** Name of the action whose PTY is now alive (e.g. "Run", "Setup"). */
  action?: string | null;
  /** Optional human-readable reason (e.g. spawn failure). */
  reason?: string | null;
}

export interface OutputPayload {
  id: string;
  /** Raw PTY bytes as a JSON number array. Convert to Uint8Array for xterm. */
  chunk: number[];
}

export interface PortsPayload {
  id: string;
  ports: number[];
}

export interface RestartingPayload {
  id: string;
  attempt: number;
  max: number;
  delayMs: number;
}

export interface VibeConfigReloadedPayload {
  config: VibeConfig;
  projects: ResolvedProject[];
  path: string;
}

// =============================================================================
// Auto-discovered build artifacts (mirrors src-tauri/src/artifacts.rs)
// =============================================================================

/** What kind of button the artifact powers. */
export type ArtifactKind = "install" | "portable";

/** A single discovered installable / portable artifact. */
export interface ProjectArtifact {
  kind: ArtifactKind;
  path: string;
  displayName: string;
  /** Parent directory — for UI hints like "in target/release/bundle/macos". */
  parent: string;
  sizeBytes: number;
  modifiedMs: number;
}

/** Top-ranked Install + Portable for a project (either may be null). */
export interface ArtifactsScan {
  install: ProjectArtifact | null;
  portable: ProjectArtifact | null;
}
