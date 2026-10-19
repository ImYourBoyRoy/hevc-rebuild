<script lang="ts">
  import { onDestroy, tick, type Snippet } from "svelte";

  interface Props {
    open: boolean;
    labelId: string;
    descriptionId?: string;
    class?: string;
    closeOnBackdrop?: boolean;
    initialFocus?: string;
    returnFocus?: HTMLElement | null;
    onClose: () => void;
    children: Snippet;
    element?: HTMLDialogElement | null;
  }
  let {
    open, labelId, descriptionId, class: className = "confirm-modal",
    closeOnBackdrop = false, initialFocus = ".modal-actions .secondary, [data-modal-initial-focus]",
    onClose, children, returnFocus = null, element = $bindable<HTMLDialogElement | null>(null),
  }: Props = $props();
  let opener: HTMLElement | null = null;
  let presented = false;

  function restoreFocus() {
    const target = returnFocus?.isConnected ? returnFocus : opener;
    opener = null;
    if (target?.isConnected) target.focus({ preventScroll: true });
  }
  function closed() {
    if (!presented) return;
    presented = false;
    onClose();
    restoreFocus();
  }
  $effect(() => {
    if (!element) return;
    if (open && !presented) {
      opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      presented = true;
      if (!element.open) element.showModal();
      const dialog = element;
      void tick().then(() => {
        if (!dialog.open) return;
        const preferred = dialog.querySelector<HTMLElement>(initialFocus);
        const focusable = preferred && !preferred.matches(":disabled") ? preferred : dialog;
        focusable.focus({ preventScroll: true });
      });
    } else if (!open && presented) {
      element.close();
    }
  });
  onDestroy(() => {
    if (element?.open) element.close();
    if (presented) closed();
    restoreFocus();
  });
</script>

<dialog
  bind:this={element}
  class={className}
  class:shared-modal={true}
  aria-labelledby={labelId}
  aria-describedby={descriptionId}
  tabindex="-1"
  onclose={closed}
  oncancel={() => { /* Native Escape closes and emits the same close contract. */ }}
  onkeydown={(event) => {
    if (event.key !== "Tab" || !element) return;
    const focusable = [...element.querySelectorAll<HTMLElement>(
      'button, input, select, textarea, a[href], [tabindex]:not([tabindex="-1"])'
    )].filter((node) => !node.matches(":disabled, [hidden]") && node.getClientRects().length > 0 && node.tabIndex >= 0);
    const first = focusable[0];
    const last = focusable.at(-1);
    if (!first || !last) { event.preventDefault(); element.focus(); return; }
    if (event.shiftKey && (document.activeElement === first || document.activeElement === element)) {
      event.preventDefault(); last.focus();
    } else if (!event.shiftKey && (document.activeElement === last || document.activeElement === element)) {
      event.preventDefault(); first.focus();
    }
  }}
  onclick={(event) => {
    if (closeOnBackdrop && event.target === element && element) {
      const bounds = element.getBoundingClientRect();
      if (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom) element.close();
    }
  }}
>
  {@render children()}
</dialog>

<style>
  .shared-modal { max-width: calc(100vw - 32px); max-height: calc(100dvh - 32px); overflow: auto; overscroll-behavior: contain; }
  .shared-modal::backdrop { background: rgb(8 14 20 / .65); backdrop-filter: blur(3px); }
  .shared-modal:not(.metadata-review-dialog) { position: fixed; inset: 0; margin: auto; }
</style>
