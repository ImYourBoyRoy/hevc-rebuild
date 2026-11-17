<script lang="ts">
  // A small "i" button that explains a setting in plain language. Opens on hover, keyboard focus
  // or click/tap, closes on Escape or when focus leaves, and never submits or triggers the label
  // it sits inside.
  import { Info } from "@lucide/svelte";
  interface Props { label: string; title?: string; text: string }
  let { label, title = "", text }: Props = $props();
  let pinned = $state(false);
  const id = `info-tip-${Math.random().toString(36).slice(2, 9)}`;
  function toggle(event: MouseEvent) {
    event.preventDefault();
    event.stopPropagation();
    pinned = !pinned;
  }
  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && pinned) {
      pinned = false;
      event.stopPropagation();
    }
  }
</script>
<span class="info-tip" class:pinned onfocusout={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) pinned = false; }} onkeydown={onKeydown} role="presentation">
  <button type="button" class="info-tip-button" aria-label={label} aria-describedby={id} aria-expanded={pinned} onclick={toggle}><Info size={14} aria-hidden="true" /></button>
  <span class="info-tip-popover" role="tooltip" {id}>{#if title}<strong>{title}</strong>{/if}<span>{text}</span></span>
</span>
