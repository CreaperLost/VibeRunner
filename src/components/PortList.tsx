import { openBrowserUrl } from "../utils";

interface PortListProps {
  projectId?: string;
  ports: number[];
}

/**
 * Shows the list of TCP ports a running process is listening on, as
 * clickable links that open the local URL in the default browser.
 *
 * Renders directly from the app-level per-project port map.
 */
export function PortList({ ports }: PortListProps) {

  if (ports.length === 0) {
    return (
      <p className="port-list__empty">
        No listening ports detected yet.
      </p>
    );
  }

  return (
    <ul className="port-list">
      {ports.map((port) => (
        <li key={port} className="port-list__item">
          <button
            type="button"
            className="port-list__link"
            onClick={() => {
              openBrowserUrl(`http://localhost:${port}`).catch((e) =>
                console.error("openBrowserUrl failed", e)
              );
            }}
            title={`Open http://localhost:${port} in your browser`}
          >
            <span className="port-list__number">{port}</span>
            <span className="port-list__url">localhost:{port}</span>
            <span className="port-list__open">↗</span>
          </button>
        </li>
      ))}
    </ul>
  );
}
