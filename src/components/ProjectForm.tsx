import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import type {
  ActionConfig,
  ProjectConfig,
  VibeConfigReloadedPayload,
} from "../types";

interface ProjectFormProps {
  existingIds: string[];
  onClose: () => void;
  onAdded: (payload: VibeConfigReloadedPayload) => void;
}

interface FormState {
  mode: "auto" | "manual";
  id: string;
  name: string;
  path: string;
  primaryAction: string;
  setupCommand: string;
  buildCommand: string;
  actions: ActionConfig[];
}

function emptyForm(): FormState {
  return {
    mode: "auto",
    id: "",
    name: "",
    path: "",
    primaryAction: "",
    setupCommand: "",
    buildCommand: "",
    actions: [
      { name: "Run", icon: "run", command: "", detached: false },
      { name: "Stop", icon: "stop", command: "", detached: false },
    ],
  };
}

const ID_PATTERN = /^[a-z0-9][a-z0-9_-]*$/i;

export function ProjectForm({ existingIds, onClose, onAdded }: ProjectFormProps) {
  const [form, setForm] = useState<FormState>(emptyForm);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const setField = <K extends keyof FormState>(k: K, v: FormState[K]) =>
    setForm((f) => ({ ...f, [k]: v }));

  const pickFolder = async () => {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Select project folder",
      });
      if (typeof picked === "string") {
        setForm((f) => {
          // Auto-fill the id from the folder name (kebab-cased).
          if (!f.id) {
            const baseName = picked.split("/").filter(Boolean).pop() ?? "";
            const slug = baseName
              .replace(/\.app$/i, "")
              .replace(/[^a-zA-Z0-9_-]+/g, "-")
              .replace(/^-+|-+$/g, "")
              .toLowerCase();
            return { ...f, path: picked, id: slug || f.id };
          }
          return { ...f, path: picked };
        });
      }
    } catch (e) {
      setError(`folder picker failed: ${typeof e === "string" ? e : String(e)}`);
    }
  };

  const setAction = (i: number, patch: Partial<ActionConfig>) =>
    setForm((f) => ({
      ...f,
      actions: f.actions.map((a, idx) => (idx === i ? { ...a, ...patch } : a)),
    }));
  const addAction = () =>
    setForm((f) => ({
      ...f,
      actions: [
        ...f.actions,
        { name: "", icon: "", command: "", detached: false },
      ],
    }));
  const removeAction = (i: number) =>
    setForm((f) => ({ ...f, actions: f.actions.filter((_, idx) => idx !== i) }));

  const validate = (f: FormState): string | null => {
    if (!f.id.trim()) return "id is required";
    if (!ID_PATTERN.test(f.id.trim()))
      return "id must be kebab-case (letters, digits, hyphens, underscores)";
    if (existingIds.includes(f.id.trim()))
      return `id '${f.id.trim()}' is already in use`;
    if (!f.path.trim()) return "path is required";
    if (f.mode === "manual") {
      if (f.actions.length === 0) return "manual projects need at least one action";
      for (const a of f.actions) {
        if (!a.name.trim()) return "every action needs a name";
        if (!a.command.trim()) return `action '${a.name}' needs a command`;
      }
    }
    return null;
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    const err = validate(form);
    if (err) {
      setError(err);
      return;
    }
    setSaving(true);
    try {
      const project: ProjectConfig = {
        id: form.id.trim(),
        path: form.path.trim(),
        name: form.name.trim() || null,
        primaryAction: form.primaryAction.trim() || null,
        manual: form.mode === "manual",
        setup: form.setupCommand.trim()
          ? { command: form.setupCommand.trim() }
          : null,
        build: form.buildCommand.trim()
          ? { command: form.buildCommand.trim() }
          : null,
        actions: form.mode === "manual" ? form.actions : [],
        env: {},
        autoRestart: null,
      };
      const payload = await invoke<VibeConfigReloadedPayload>("add_project", { project });
      onAdded(payload);
      onClose();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <form
        className="modal"
        onClick={(e) => e.stopPropagation()}
        onSubmit={handleSubmit}
      >
        <header className="modal__header">
          <h2 className="modal__title">New Project</h2>
          <button
            type="button"
            className="modal__close"
            onClick={onClose}
            aria-label="Close"
          >
            ×
          </button>
        </header>

        {error && (
          <div className="modal__error" role="alert">
            {error}
          </div>
        )}

        <div className="modal__body">
          <div className="form-row form-row--inline">
            <label>Mode</label>
            <div className="radio-group">
              <label>
                <input
                  type="radio"
                  name="mode"
                  value="auto"
                  checked={form.mode === "auto"}
                  onChange={() => setField("mode", "auto")}
                />
                Auto-discover (TOML)
              </label>
              <label>
                <input
                  type="radio"
                  name="mode"
                  value="manual"
                  checked={form.mode === "manual"}
                  onChange={() => setField("mode", "manual")}
                />
                Manual (inline commands)
              </label>
            </div>
          </div>

          <div className="form-row">
            <label htmlFor="pf-id">id *</label>
            <input
              id="pf-id"
              type="text"
              value={form.id}
              onChange={(e) => setField("id", e.target.value)}
              placeholder="e.g. assetflow, axm"
              autoFocus
            />
          </div>

          <div className="form-row">
            <label htmlFor="pf-path">Project folder *</label>
            <div className="form-row__path">
              <input
                id="pf-path"
                type="text"
                value={form.path}
                onChange={(e) => setField("path", e.target.value)}
                placeholder="/Users/you/projects/my-app"
              />
              <button
                type="button"
                className="btn btn--small"
                onClick={pickFolder}
              >
                Browse…
              </button>
            </div>
            {form.mode === "auto" && (
              <p className="form-hint">
                VibeRunner will look for{" "}
                <code>.codex/environments/environment.toml</code> in this
                folder. If found, the buttons are auto-generated from its
                <code> [[actions]]</code>.
              </p>
            )}
          </div>

          <div className="form-row">
            <label htmlFor="pf-name">display name (optional)</label>
            <input
              id="pf-name"
              type="text"
              value={form.name}
              onChange={(e) => setField("name", e.target.value)}
              placeholder="Defaults to the TOML's `name` or the id"
            />
          </div>

          {form.mode === "manual" && (
            <>
              <fieldset className="form-section">
                <legend>Setup (optional)</legend>
                <div className="form-row">
                  <label htmlFor="pf-setup">command</label>
                  <input
                    id="pf-setup"
                    type="text"
                    value={form.setupCommand}
                    onChange={(e) => setField("setupCommand", e.target.value)}
                    placeholder="e.g. npm install"
                  />
                </div>
              </fieldset>

              <fieldset className="form-section">
                <legend>Build (optional)</legend>
                <div className="form-row">
                  <label htmlFor="pf-build">command</label>
                  <input
                    id="pf-build"
                    type="text"
                    value={form.buildCommand}
                    onChange={(e) => setField("buildCommand", e.target.value)}
                    placeholder="e.g. npm run build"
                  />
                </div>
              </fieldset>

              <fieldset className="form-section">
                <legend>Actions</legend>
                {form.actions.map((a, i) => (
                  <div className="form-action-row" key={i}>
                    <input
                      type="text"
                      value={a.name}
                      onChange={(e) => setAction(i, { name: e.target.value })}
                      placeholder="Name"
                      className="form-action-name"
                    />
                    <input
                      type="text"
                      value={a.icon ?? ""}
                      onChange={(e) => setAction(i, { icon: e.target.value })}
                      placeholder="icon"
                      className="form-action-icon"
                    />
                    <input
                      type="text"
                      value={a.command}
                      onChange={(e) => setAction(i, { command: e.target.value })}
                      placeholder="command"
                      className="form-action-cmd"
                    />
                    <label
                      className="form-action-detached"
                      title="Check if the command detaches (e.g. nohup ... &) — VibeRunner will then track the whole process tree"
                    >
                      <input
                        type="checkbox"
                        checked={a.detached ?? false}
                        onChange={(e) =>
                          setAction(i, { detached: e.target.checked })
                        }
                      />
                      detached
                    </label>
                    <button
                      type="button"
                      className="btn-icon"
                      onClick={() => removeAction(i)}
                      aria-label="Remove action"
                      title="Remove"
                    >
                      ×
                    </button>
                  </div>
                ))}
                <button
                  type="button"
                  className="btn btn--small"
                  onClick={addAction}
                >
                  + Add action
                </button>
              </fieldset>
            </>
          )}

          <div className="form-row">
            <label htmlFor="pf-primary">primary action (for Restart)</label>
            <input
              id="pf-primary"
              type="text"
              value={form.primaryAction}
              onChange={(e) => setField("primaryAction", e.target.value)}
              placeholder='Defaults to first "Run" action'
            />
          </div>
        </div>

        <footer className="modal__footer">
          <button
            type="button"
            className="btn"
            onClick={onClose}
            disabled={saving}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="btn btn--primary"
            disabled={saving}
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </footer>
      </form>
    </div>
  );
}
