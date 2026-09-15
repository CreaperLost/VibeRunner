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
  isScrolledUp: () => boolean;
}

interface LogViewerProps {
  projectId: string;
  expanded?: boolean;
  onActionsReady?: (actions: LogViewerHandle) => void;
  onScrolledUpChange?: (scrolledUp: boolean) => void;
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
export function LogViewer({
  projectId,
  expanded = false,
  onActionsReady,
  onScrolledUpChange,
}: LogViewerProps) {
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

    let userScrolledUp = false;

    // Track user scrolling: if user manually scrolls away from the bottom, pause auto-scroll
    const scrollSub = term.onScroll(() => {
      const buffer = term.buffer.active;
      const atBottom = buffer.viewportY >= buffer.baseY;
      if (atBottom && userScrolledUp) {
        userScrolledUp = false;
        onScrolledUpChange?.(false);
        term.scrollToBottom();
      } else if (!atBottom && !userScrolledUp) {
        userScrolledUp = true;
        onScrolledUpChange?.(true);
      }
    });

    // Handle mouse wheel scrolling:
    // When logs are at the bottom and user scrolls down, or at the top and user scrolls up,
    // forward the scroll to the parent detail window (.detail) so ports and header can be reached.
    term.attachCustomWheelEventHandler((ev: WheelEvent) => {
      if (Math.abs(ev.deltaY) < Math.abs(ev.deltaX)) {
        return true;
      }

      const buffer = term.buffer.active;
      const isAtBottom = buffer.viewportY >= buffer.baseY;
      const isAtTop = buffer.viewportY <= 0;

      if (ev.deltaY > 0 && isAtBottom) {
        const detail = container.closest(".detail");
        if (detail) {
          detail.scrollTop += ev.deltaY;
        }
        ev.preventDefault();
        return false;
      }

      if (ev.deltaY < 0 && isAtTop) {
        const detail = container.closest(".detail");
        if (detail) {
          detail.scrollTop += ev.deltaY;
        }
        ev.preventDefault();
        return false;
      }

      return true;
    });

    const syncPty = () => {
      try {
        fit.fit();
        // Exact pixel height sync: eliminates subpixel and fractional line remainder
        // so xterm's scrollbar and viewport scroll truly to the very last line.
        const core = (term as any)._core;
        const cellHeight = core?._renderService?.dimensions?.css?.cell?.height;
        if (cellHeight && cellHeight > 0 && term.rows > 0) {
          const exactHeight = Math.round(term.rows * cellHeight);
          if (exactHeight > 0 && container.style.height !== `${exactHeight}px`) {
            container.style.height = `${exactHeight}px`;
          }
        }
        if (term.cols && term.rows) {
          invoke("resize_pty", {
            projectId,
            rows: term.rows,
            cols: term.cols,
          }).catch(() => {});
        }
        if (!userScrolledUp) {
          term.scrollToBottom();
        }
      } catch {
        // Container momentarily unmeasured during layout
      }
    };

    syncPty();
    requestAnimationFrame(() => syncPty());
    document.fonts?.ready?.then(() => syncPty());

    // Expose actions to parent
    onActionsReady?.({
      scrollToBottom: () => {
        userScrolledUp = false;
        onScrolledUpChange?.(false);
        term.scrollToBottom();
      },
      clear: () => {
        userScrolledUp = false;
        onScrolledUpChange?.(false);
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
      isScrolledUp: () => userScrolledUp,
    });

    // Replay any buffered logs for this project and scroll to bottom
    const buffered = logBuffers.get(projectId);
    if (buffered && buffered.chunks.length > 0) {
      for (let i = 0; i < buffered.chunks.length; i++) {
        const isLast = i === buffered.chunks.length - 1;
        term.write(
          buffered.chunks[i],
          isLast
            ? () => {
                userScrolledUp = false;
                onScrolledUpChange?.(false);
                term.scrollToBottom();
              }
            : undefined
        );
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
        term.write(chunk, () => {
          if (!userScrolledUp) {
            term.scrollToBottom();
          }
        });
      }
    };
    subscribers.add(onOutput);

    // Re-fit when container or parent panel size changes (e.g. sidebar resize, window resize)
    const observeTarget = container.parentElement ?? container;
    const ro = new ResizeObserver(() => {
      syncPty();
    });
    ro.observe(observeTarget);

    return () => {
      subscribers.delete(onOutput);
      scrollSub.dispose();
      sub.dispose();
      ro.disconnect();
      term.dispose();
      fitRef.current = null;
    };
  }, [projectId]);

  // Re-fit and sync when expanded state toggles (after CSS transition completes)
  useEffect(() => {
    const timer = setTimeout(() => {
      try {
        fitRef.current?.fit();
      } catch {
        // ignore
      }
    }, 220);
    return () => clearTimeout(timer);
  }, [expanded]);

  return (
    <div className={`log-viewer${expanded ? " log-viewer--expanded" : ""}`}>
      <div ref={containerRef} className="log-viewer__terminal" />
    </div>
  );
}
