import { useEffect, useMemo, useRef, useState } from "react";
import type { ProjectRuntime, ResolvedProject, VibeConfigReloadedPayload } from "../types";
import { ProjectCard } from "./ProjectCard";
import { ProjectForm } from "./ProjectForm";
import { Icon } from "./Icon";
import { runtimeOf } from "../hooks/useRunnerEvents";
import { usePersistentState } from "../hooks/usePersistentState";

type ViewMode = "detailed" | "compact";

interface SidebarProps {
  projects: ResolvedProject[];
  runtime: Map<string, ProjectRuntime>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onConfigReloaded: (payload: VibeConfigReloadedPayload) => void;
  onError?: (msg: string) => void;
  width?: number;
}

export function Sidebar({
  projects,
  runtime,
  selectedId,
  onSelect,
  onConfigReloaded,
  onError,
  width,
}: SidebarProps) {
  const [showForm, setShowForm] = useState(false);
  const [query, setQuery] = useState("");
  const [viewMode, setViewMode] = usePersistentState<ViewMode>("viberunner.sidebar.viewMode", "detailed");
  const searchRef = useRef<HTMLInputElement>(null);
  const compact = viewMode === "compact";
  const isMac = navigator.platform.toLowerCase().includes("mac");

  // Cmd/Ctrl+F focuses the search box. Esc clears it.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
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
        p.actions.some((a) => a.name.toLowerCase().includes(q) || a.command.toLowerCase().includes(q))
    );
  }, [projects, query]);

  const running = projects.filter((p) => runtime.get(p.id)?.status === "running").length;

  return (
    <aside className="sidebar" style={width ? { width: `${width}px` } : undefined}>
      <div className="sidebar__header">
        <div className="sidebar__heading">
          <span className="sidebar__title">Projects</span>
          <span className="sidebar__count">
            {query ? `${filtered.length}/${projects.length}` : projects.length}
            {running > 0 && <span className="sidebar__running"> · {running} running</span>}
          </span>
        </div>
        <div className="segmented" role="group" aria-label="Project list view">
          <button
            type="button"
            className={`segmented__btn${!compact ? " segmented__btn--active" : ""}`}
            onClick={() => setViewMode("detailed")}
            aria-pressed={!compact}
            title="Detailed view"
          >
            <Icon name="list" size={13} />
          </button>
          <button
            type="button"
            className={`segmented__btn${compact ? " segmented__btn--active" : ""}`}
            onClick={() => setViewMode("compact")}
            aria-pressed={compact}
            title="Compact view"
          >
            <Icon name="rows" size={13} />
          </button>
        </div>
        <button type="button" className="btn btn--small btn--primary" onClick={() => setShowForm(true)} title="Add a project">
          <Icon name="plus" size={13} />
          New
        </button>
      </div>

      <div className="search">
        <Icon name="search" size={14} className="search__icon" />
        <input
          ref={searchRef}
          type="search"
          className="search__input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search projects"
          aria-label="Filter projects"
        />
        {query ? (
          <button type="button" className="search__clear" onClick={() => setQuery("")} aria-label="Clear search">
            <Icon name="x" size={12} />
          </button>
        ) : (
          <kbd className="search__kbd">{isMac ? "⌘F" : "Ctrl F"}</kbd>
        )}
      </div>

      <div className={`sidebar__list${compact ? " sidebar__list--compact" : ""}`}>
        {projects.length === 0 ? (
          <div className="empty-state">
            <Icon name="folder" size={22} />
            <p>No projects yet.</p>
            <p className="empty-state__hint">
              Click <strong>New</strong> to add a folder. A <code>.codex/environments/environment.toml</code> inside
              it is picked up automatically; otherwise enter commands manually.
            </p>
          </div>
        ) : filtered.length === 0 ? (
          <div className="empty-state">
            <p>No projects match “{query}”.</p>
          </div>
        ) : (
          filtered.map((p) => (
            <ProjectCard
              key={p.id}
              project={p}
              runtime={runtimeOf(runtime, p.id)}
              selected={p.id === selectedId}
              compact={compact}
              onSelect={() => onSelect(p.id)}
              onConfigReloaded={onConfigReloaded}
              onError={onError}
            />
          ))
        )}
      </div>

      {showForm && (
        <ProjectForm
          existingIds={projects.map((p) => p.id)}
          onClose={() => setShowForm(false)}
          onAdded={(payload) => onConfigReloaded(payload)}
        />
      )}
    </aside>
  );
}
