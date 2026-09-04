// Shared visibility store for the per-download `AuthDialog`.
//
// The dialog is a singleton rendered once at the App level
// (`App.svelte` mounts it alongside the rest of the dialog
// stack). The `DownloadRow`'s dropdown menu dispatches an
// `auth` action which calls `authTarget.set({ id, download })`
// to open the dialog against a specific download; the dialog
// itself listens to `$authTarget` and renders / closes
// accordingly.
//
// This store lives in a separate module (rather than as a
// named `export const` inside `AuthDialog.svelte`'s `<script>`
// block) because Svelte's TypeScript surface only re-exports
// the component's default export — a named `export const`
// inside a `<script>` is not reachable through
// `import { authTarget } from "./AuthDialog.svelte"`. The
// standard Svelte workaround is a sibling `.ts` module.
import { writable } from "svelte/store";
import type { Download } from "../types";

/** The dialog's target — null when closed. */
export type AuthDialogTarget = { id: string; download: Download } | null;

export const authTarget = writable<AuthDialogTarget>(null);
