<!-- ./desktop/src/lib/views/ShellSidebar.svelte
     Brand lockup, main navigation, active-library chip and engine status.
     Pure view: navigation and the folder picker are route callbacks. -->
<script lang="ts">
  import { APP_VERSION } from "../app-version.js";
  import { ChevronRight, Clapperboard, FolderOpen, History, Film, Settings2, Video, Wrench } from "@lucide/svelte";
  import type { Environment, Page } from "../types.js";

  const primaryNavigation: { id: Page; label: string; icon: typeof Film }[] = [
    { id: "library", label: "Library", icon: Film },
    { id: "run", label: "Run", icon: Settings2 },
    { id: "youtube", label: "YouTube", icon: Video },
  ];
  const utilityNavigation: { id: Page; label: string; icon: typeof Film }[] = [
    { id: "settings", label: "Settings", icon: Wrench },
    { id: "reports", label: "Reports", icon: History },
  ];

  interface Props {
    environment: Environment | null;
    environmentLoading: boolean;
    booting: boolean;
    root: string;
    failedCount: number;
    navItemActive: (id: Page) => boolean;
    openPage: (page: Page) => void;
    browseFolder: () => unknown;
    fileName: (path: string) => string;
  }
  let { environment, environmentLoading, booting, root, failedCount, navItemActive, openPage, browseFolder, fileName }: Props = $props();
</script>

<aside class="sidebar">
  <div class="brand-lockup">
    <div class="brand-mark"><Clapperboard size={20} strokeWidth={2.2} /></div>
    <div><div class="brand-name">HEVC <span>REBUILD</span></div><div class="brand-edition" title={environment?.os.name ? `Running on ${environment.os.name} ${environment.os.version}`.trim() : undefined}>STUDIO</div></div>
  </div>

  <nav aria-label="Main navigation">
    <div class="workspace-label">LIBRARY</div>
    {#each primaryNavigation as item (item.id)}
      {@const Icon = item.icon}
      <button class="nav-item" class:active={navItemActive(item.id)} onclick={() => openPage(item.id)} aria-label={item.label} title={item.label} aria-current={navItemActive(item.id) ? "page" : undefined}>
        <Icon size={18} strokeWidth={1.8} /><span>{item.label}</span>
        {#if navItemActive(item.id)}<ChevronRight class="nav-arrow" size={15} />{/if}
      </button>
    {/each}
    <div class="workspace-label sidebar-nav-utility">APP</div>
    {#each utilityNavigation as item (item.id)}
      {@const Icon = item.icon}
      <button class="nav-item" class:active={navItemActive(item.id)} onclick={() => openPage(item.id)} aria-label={item.label} title={item.label} aria-current={navItemActive(item.id) ? "page" : undefined}>
        <Icon size={18} strokeWidth={1.8} /><span>{item.label}</span>
        {#if item.id === "reports" && failedCount}<span class="nav-count">{failedCount}</span>{/if}
        {#if navItemActive(item.id)}<ChevronRight class="nav-arrow" size={15} />{/if}
      </button>
    {/each}
  </nav>

  <div class="sidebar-grow"></div>
  <div class="sidebar-library">
    <div class="sidebar-library-home">
      <span class="sidebar-library-icon" aria-hidden="true"><FolderOpen size={17} /></span>
      <span class="sidebar-library-copy"><span>ACTIVE LIBRARY</span><strong title={root}>{root ? fileName(root) : "No folder selected"}</strong></span>
    </div>
    <button class="icon-button subdued" aria-label="Choose media folder" title="Choose a media folder" onclick={browseFolder}><ChevronRight size={16} /></button>
  </div>
  <div class="sidebar-bottom">
    <span class="connection-dot" class:green={environment?.ready}></span>
    <span>{environment?.ready ? "Engine ready" : environmentLoading || booting ? "Checking engine…" : "Setup required"}</span>
    <span class="version-tag">v{APP_VERSION}</span>
  </div>
</aside>
