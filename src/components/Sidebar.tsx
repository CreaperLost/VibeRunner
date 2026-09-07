import { useEffect, useMemo, useRef, useState } from "react";
import type { ProjectStatus, ResolvedProject, VibeConfigReloadedPayload } from "../types";
import { ProjectCard } from "./ProjectCard";
import { ProjectForm } from "./ProjectForm";

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
}: SidebarProps) {
  const [showForm, setShowForm] = useState(false);
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);

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
    <aside className="sidebar">
      <div className="sidebar__header">
        <span className="sidebar__count">
          {query
            ? `${filtered.length} / ${projects.length}`
            : `${projects.length} project${projects.length === 1 ? "" : "s"}`}
        </span>
        <button
          type="button"
          className="btn btn--small"
          onClick={() => setShowForm(true)}
          title="Add a new project"
        >
          + New
        </button>
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

      <div className="sidebar__list">
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
