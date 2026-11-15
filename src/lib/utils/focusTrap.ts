// Svelte action for centred dialogs (docs/UI_DESIGN_PLAN.md §9).
//
// `role="dialog" aria-modal="true"` promises three things the browser
// does not do for us: focus moves into the dialog, Tab stays inside it,
// and focus returns to the opener when it closes. `use:focusTrap` on the
// dialog element supplies all three. The element is inside an `{#if}`,
// so Svelte calls the action on open and `destroy()` on close.
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), ' +
  'select:not([disabled]), textarea:not([disabled]), ' +
  '[tabindex]:not([tabindex="-1"])';

export function focusTrap(node: HTMLElement) {
  const previous = document.activeElement as HTMLElement | null;
  const focusables = () =>
    Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE));

  focusables()[0]?.focus();

  function onKeydown(e: KeyboardEvent) {
    if (e.key !== "Tab") return;
    const items = focusables();
    if (items.length === 0) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  node.addEventListener("keydown", onKeydown);
  return {
    destroy() {
      node.removeEventListener("keydown", onKeydown);
      if (previous && document.contains(previous)) previous.focus();
    },
  };
}
