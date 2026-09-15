import { useEffect, useMemo, useRef, useState } from "react";
import type { ProjectStatus, ResolvedProject, VibeConfigReloadedPayload } from "../types";
import { ProjectCard } from "./ProjectCard";
import { ProjectForm } from "./ProjectForm";
import { usePersistentState } from "../hooks/usePersistentState";

type ViewMode = "detailed" | "compact";

interface SidebarProps {
  projects: ResolvedProject[];
  statuses: Map<string, ProjectStatus>;
  currentActions: Map<string, string>;
  ports: Map<string, number[]>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onConfigReloaded: (payload: VibeConfigReloadedPayload) => void;
  onRemoveProject: (id: string) => void;
  onError?: (msg: string) => void;
  width?: number;
}

export function Sidebar({
  projects,
  statuses,
  currentActions,
  ports,
  selectedId,
  onSelect,
  onConfigReloaded,
  onRemoveProject,
  onError,
  width,
}: SidebarProps) {
  const [showForm, setShowForm] = useState(false);
  const [query, setQuery] = useState("");
  const [viewMode, setViewMode] = usePersistentState<ViewMode>(
    "viberunner.sidebar.viewMode",
    "detailed"
  );
  const searchRef = useRef<HTMLInputElement>(null);
  const compact = viewMode === "compact";

  // Cmd/Ctrl+F focuses the search box. Esc clears it.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const isFind =
        (e.metaKey || e.ctrlKey) && (e.key === "f" || e.key === "F");
      if (isFind) {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      } else if (e.key === "Escape" && document.activeElement === searchRef.current) {
        setQuery("");
        searchRef.current?.blur();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return projects;
    return projects.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        p.id.toLowerCase().includes(q) ||
        p.path.toLowerCase().includes(q) ||
        p.actions.some(
          (a) =>
            a.name.toLowerCase().includes(q) ||
            a.command.toLowerCase().includes(q)
        )
    );
  }, [projects, query]);

  return (
    <aside
      className="sidebar"
      style={width ? { width: `${width}px` } : undefined}
    >
      <div className="sidebar__header">
        <span className="sidebar__count">
          {query
            ? `${filtered.length} / ${projects.length}`
            : `${projects.length} project${projects.length === 1 ? "" : "s"}`}
        </span>
        <div className="sidebar__header-actions">
          <div
            className="view-toggle"
            role="group"
            aria-label="Project list view"
          >
            <button
              type="button"
              className={`view-toggle__btn${viewMode === "detailed" ? " view-toggle__btn--active" : ""}`}
              onClick={() => setViewMode("detailed")}
              aria-pressed={viewMode === "detailed"}
              title="Detailed view — shows path, actions, source"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="18" x2="21" y2="18"/></svg>
              <span className="view-toggle__label">Detailed</span>
            </button>
            <button
              type="button"
              className={`view-toggle__btn${viewMode === "compact" ? " view-toggle__btn--active" : ""}`}
              onClick={() => setViewMode("compact")}
              aria-pressed={viewMode === "compact"}
              title="Compact view — icon, name, and status only"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><line x1="3" y1="12" x2="21" y2="12"/></svg>
              <span className="view-toggle__label">Compact</span>
            </button>
          </div>
          <button
            type="button"
            className="btn btn--small sidebar__new-btn"
            onClick={() => setShowForm(true)}
            title="Add a new project"
          >
            + New
          </button>
        </div>
      </div>

      <div className="sidebar__search">
        <input
          ref={searchRef}
          type="search"
          className="sidebar__search-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search…  (⌘F)"
          aria-label="Filter projects"
        />
        {query && (
          <button
            type="button"
            className="sidebar__search-clear"
            onClick={() => setQuery("")}
            aria-label="Clear search"
            title="Clear (Esc)"
          >
            ×
          </button>
        )}
      </div>

      <div className={`sidebar__list${compact ? " sidebar__list--compact" : ""}`}>
        {projects.length === 0 ? (
          <div className="sidebar__empty">
            <p>No projects yet.</p>
            <p className="sidebar__hint">
              Click <strong>+ New</strong> to add a folder. If the folder
              has a <code>.codex/environments/environment.toml</code>,
              VibeRunner auto-discovers its actions. Otherwise you can
              enter the commands manually.
            </p>
          </div>
        ) : filtered.length === 0 ? (
          <div className="sidebar__empty">
            <p>No projects match "{query}".</p>
          </div>
        ) : (
          filtered.map((p) => (
            <ProjectCard
              key={p.id}
              project={p}
              status={statuses.get(p.id) ?? "stopped"}
              currentAction={currentActions.get(p.id) ?? null}
              ports={ports.get(p.id) ?? []}
              selected={p.id === selectedId}
              compact={compact}
              onSelect={() => onSelect(p.id)}
              onConfigReloaded={onConfigReloaded}
              onRemoveProject={onRemoveProject}
              onError={onError}
              busy={
                statuses.get(p.id) === "running" ||
                statuses.get(p.id) === "starting" ||
                statuses.get(p.id) === "stopping"
              }
            />
          ))
        )}
      </div>

      {showForm && (
        <ProjectForm
          existingIds={projects.map((p) => p.id)}
          onClose={() => setShowForm(false)}
          onAdded={(payload) => {
            onConfigReloaded(payload);
          }}
        />
      )}
    </aside>
  );
}