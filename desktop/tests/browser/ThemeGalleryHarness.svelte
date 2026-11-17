<script lang="ts">
  // Side-by-side theme gallery: one iframe per theme (instrument skins key off <html>).
  const params = new URLSearchParams(location.search);
  const themes = (params.get('themes') ?? 'light,dark,cinema,oled,dos,phosphor,ion,breach').split(',').filter(Boolean);
</script>
<div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(560px,1fr));gap:14px;padding:14px;background:#888">
  {#each themes as theme (theme)}
    <figure style="margin:0"><figcaption style="font:700 12px system-ui;color:#fff;padding:2px 0 4px">{theme}</figcaption>
      <iframe title={`${theme} theme`} src={`?fixture=chrome&theme=${theme}`} style="width:100%;height:420px;border:0;border-radius:6px;background:#000"></iframe>
    </figure>
  {/each}
</div>
