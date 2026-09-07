import { invoke } from "@tauri-apps/api/core";
import { openUrl as pluginOpenUrl } from "@tauri-apps/plugin-opener";

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

/**
 * Open a local file or directory using the OS default application.
 * Uses the backend `open_path` command directly (running /usr/bin/open on macOS).
 */
export async function openLocalPath(path: string): Promise<void> {
  await invoke("open_path", { path });
}
