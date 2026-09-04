// Component barrel export
export { default as DownloadRow } from "./DownloadRow.svelte";
export { default as FileIcon } from "./FileIcon.svelte";
export { default as ProgressBar } from "./ProgressBar.svelte";
export { default as StatusBadge } from "./StatusBadge.svelte";
export { default as DropdownMenu } from "./DropdownMenu.svelte";
export { default as InlineSpeedLimit } from "./InlineSpeedLimit.svelte";
export { default as GlobalSearch } from "./GlobalSearch.svelte";
export { default as CommandPalette } from "./CommandPalette.svelte";
export { default as SmartFilters } from "./SmartFilters.svelte";
export { default as SettingsTabs } from "./SettingsTabs.svelte";
export { default as StatusBar } from "./StatusBar.svelte";
export { default as Toast } from "./Toast.svelte";
export { default as CaptureDialog } from "./CaptureDialog.svelte";
export { default as BulkActionBar } from "./BulkActionBar.svelte";
export { default as AuthDialog } from "./AuthDialog.svelte";
// `authTarget` is the writable store that controls the
// per-download auth dialog's visibility. It lives in a
// sibling `.ts` module (not in the Svelte component's
// `<script>` block) because Svelte's TypeScript surface
// only re-exports the component's default export — named
// `export const` values inside `<script>` are not reachable
// from the outside. See `./authTarget.ts` for the store
// itself.
export { authTarget } from "./authTarget";
export { default as SpeedGraph } from "../SpeedGraph.svelte";