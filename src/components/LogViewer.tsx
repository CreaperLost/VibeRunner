import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import "@xterm/xterm/css/xterm.css";
import type { OutputPayload } from "../types";

export interface LogViewerHandle {
  scrollToBottom: () => void;
  clear: () => void;
  copyLogs: () => Promise<boolean>;
}

interface LogViewerProps {
  projectId: string;
  expanded?: boolean;
  onActionsReady?: (actions: LogViewerHandle) => void;
}

// Global in-memory log replay buffer per project.
// Retains recent output so switching projects does not lose logs,
// and output emitted while another project is selected is preserved.
const MAX_BUFFER_BYTES = 1024 * 1024; // 1 MB per project

interface ProjectLogBuffer {
  chunks: Uint8Array[];
  totalBytes: number;
}

const logBuffers = new Map<string, ProjectLogBuffer>();
type OutputSubscriber = (projectId: string, chunk: Uint8Array) => void;
const subscribers = new Set<OutputSubscriber>();

let globalListenerStarted = false;
function initGlobalOutputListener() {
  if (globalListenerStarted) return;
  globalListenerStarted = true;

  listen<OutputPayload>("project:output", (e) => {
    const id = e.payload.id;
    const chunk = new Uint8Array(e.payload.chunk);

    let buf = logBuffers.get(id);
    if (!buf) {
      buf = { chunks: [], totalBytes: 0 };
      logBuffers.set(id, buf);
    }
    buf.chunks.push(chunk);
    buf.totalBytes += chunk.byteLength;

    while (buf.totalBytes > MAX_BUFFER_BYTES && buf.chunks.length > 1) {
      const dropped = buf.chunks.shift();
      if (dropped) {
        buf.totalBytes -= dropped.byteLength;
      }
    }

    for (const sub of subscribers) {
      sub(id, chunk);
    }
  });
}

/**
 * Live PTY log viewer for a single project. Backed by an xterm.js Terminal.
 *
 * - Replays buffered output on mount so switching projects does not lose logs.
 * - Subscribes to `project:output` events for `projectId` and writes
 *   the raw bytes to the terminal (so ANSI colors render correctly).
 * - Pipes user keystrokes (anything the user types into the terminal)
 *   back to the PTY via the `write_to_pty` command.
 * - Re-fits on container resize and expanded state toggle.
 */
export function LogViewer({ projectId, expanded = false, onActionsReady }: LogViewerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;
    const container = containerRef.current;

    initGlobalOutputListener();

    const term = new Terminal({
      fontFamily:
        '"SF Mono", Menlo, Monaco, Consolas, "Liberation Mono", monospace',
      fontSize: 12,
      lineHeight: 1.25,
      cursorBlink: false,
      convertEol: true,
      disableStdin: false,
      scrollback: 10000,
      scrollOnUserInput: true,
      smoothScrollDuration: 0,
      theme: {
        background: "#0f172a",
        foreground: "#cbd5e1",
        cursor: "#94a3b8",
        selectionBackground: "#334155",
      },
    });

    const fit = new FitAddon();
    fitRef.current = fit;
    term.loadAddon(fit);
    term.open(container);

    const syncPty = () => {
      try {
        fit.fit();
        if (term.cols && term.rows) {
          invoke("resize_pty", {
            projectId,
            rows: term.rows,
            cols: term.cols,
          }).catch(() => {});
        }
      } catch {
        // Container momentarily unmeasured during layout
      }
    };

    syncPty();

    // Expose actions to parent
    onActionsReady?.({
      scrollToBottom: () => {
        term.scrollToBottom();
      },
      clear: () => {
        term.clear();
        logBuffers.delete(projectId);
      },
      copyLogs: async () => {
        const buf = logBuffers.get(projectId);
        let text = "";
        if (buf && buf.chunks.length > 0) {
          const decoder = new TextDecoder();
          text = buf.chunks.map((c) => decoder.decode(c, { stream: true })).join("");
          // Strip ANSI terminal styling codes for clean clipboard text
          text = text.replace(/\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])/g, "");
        }
        if (!text) {
          term.selectAll();
          text = term.getSelection();
          term.clearSelection();
        }
        if (text) {
          try {
            await navigator.clipboard.writeText(text);
            return true;
          } catch (err) {
            console.error("Failed to copy logs to clipboard", err);
            return false;
          }
        }
        return false;
      },
    });

    // Replay any buffered logs for this project and scroll to bottom
    const buffered = logBuffers.get(projectId);
    if (buffered && buffered.chunks.length > 0) {
      for (let i = 0; i < buffered.chunks.length; i++) {
        const isLast = i === buffered.chunks.length - 1;
        term.write(buffered.chunks[i], isLast ? () => term.scrollToBottom() : undefined);
      }
    }

    // Pipe user keystrokes back to the PTY.
    const sub = term.onData((data) => {
      invoke("write_to_pty", { projectId, data }).catch((e) => {
        console.error("write_to_pty failed", e);
      });
    });

    // Subscribe to live output events for this project
    const onOutput: OutputSubscriber = (id, chunk) => {
      if (id === projectId) {
        const buffer = term.buffer.active;
        const isNearBottom = buffer.viewportY >= buffer.baseY - 2;
        term.write(chunk, () => {
          if (isNearBottom) {
            term.scrollToBottom();
          }
        });
      }
    };
    subscribers.add(onOutput);

    // Explicit container mouse wheel scroll support
    const handleWheel = (e: WheelEvent) => {
      if (e.deltaY !== 0) {
        const lines = Math.sign(e.deltaY) * Math.max(1, Math.round(Math.abs(e.deltaY) / 25));
        term.scrollLines(lines);
      }
    };
    container.addEventListener("wheel", handleWheel, { passive: true });

    // Re-fit when container size changes
    const ro = new ResizeObserver(() => {
      syncPty();
    });
    ro.observe(container);

    return () => {
      container.removeEventListener("wheel", handleWheel);
      subscribers.delete(onOutput);
      sub.dispose();
      ro.disconnect();
      term.dispose();
      fitRef.current = null;
    };
  }, [projectId]);

  // Re-fit and sync when expanded state toggles
  useEffect(() => {
    const timer = setTimeout(() => {
      try {
        fitRef.current?.fit();
      } catch {
        // ignore
      }
    }, 100);
    return () => clearTimeout(timer);
  }, [expanded]);


  return (
    <div
      ref={containerRef}
      className={`log-viewer${expanded ? " log-viewer--expanded" : ""}`}
    />
  );
}
