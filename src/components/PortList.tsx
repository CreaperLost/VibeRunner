import { openBrowserUrl } from "../utils";
import { Icon } from "./Icon";

interface PortListProps {
  ports: number[];
  active: boolean;
}

/**
 * Listening TCP ports owned by the project's process tree, as clickable
 * localhost links. The first entry is the URL the app printed, if any.
 */
export function PortList({ ports, active }: PortListProps) {
  if (ports.length === 0) {
    return (
      <div className="empty-state">
        <Icon name="plug" size={22} />
        <p>{active ? "No listening ports yet." : "Not running."}</p>
        <p className="empty-state__hint">
          Ports appear when a process started by this project listens on one.
        </p>
      </div>
    );
  }

  return (
    <ul className="port-list">
      {ports.map((port, i) => (
        <li key={port}>
          <button
            type="button"
            className="port-list__item"
            onClick={() => openBrowserUrl(`http://localhost:${port}`).catch((e) => console.error(e))}
            title={`Open http://localhost:${port}`}
          >
            <span className="port-list__number">:{port}</span>
            <span className="port-list__url">http://localhost:{port}</span>
            {i === 0 && <span className="badge badge--accent">app</span>}
            <Icon name="external" size={14} className="port-list__open" />
          </button>
        </li>
      ))}
    </ul>
  );
}
