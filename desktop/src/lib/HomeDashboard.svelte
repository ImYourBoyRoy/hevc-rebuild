<!-- ./desktop/src/lib/HomeDashboard.svelte
     Home for the next library, organizer, rebuild, or YouTube action.
     Tool checks and recent runs stay on the page under those actions.
     The parent owns navigation and folder selection. -->
<script lang="ts">
  import { Check, X } from "@lucide/svelte";
  import type { CommandCheck, Environment, Page, RunReport } from "#lib/types.js";
  import "../styles/components/home.css";

  interface StatusCount {
    key: string;
    label: string;
    count: number;
  }

  interface SetupLog {
    jobId: string;
    modifiedUnix: number;
    sizeBytes: number;
  }

  interface Props {
    environment: Environment | null;
    environmentLoading: boolean;
    sourceCount: number;
    fileCount: number;
    activeFolder: string;
    statusCounts: StatusCount[];
    activityLabel: string;
    reports: RunReport[];
    setupLogs: SetupLog[];
    onOpen: (page: Page) => void;
    onOpenReport: (csvPath: string) => void;
    onAddFolder: () => void;
    onAddFiles: () => void;
    onRefreshChecks: () => void;
  }

  let {
    environment,
    environmentLoading,
    sourceCount,
    fileCount,
    activeFolder,
    statusCounts,
    activityLabel,
    reports,
    setupLogs,
    onOpen,
    onOpenReport,
    onAddFolder,
    onAddFiles,
    onRefreshChecks,
  }: Props = $props();

  let checksOpen = $state(false);
  let checks = $derived([...(environment?.checks ?? [])].sort((left, right) => Number(right.required && !right.available) - Number(left.required && !left.available)));
  let readyChecks = $derived(checks.filter((check) => check.available).length);
  let missingRequired = $derived(checks.filter((check) => check.required && !check.available));
  let recentReports = $derived([...reports].sort((left, right) => right.created - left.created).slice(0, 5));
  let recentLogs = $derived([...setupLogs].sort((left, right) => right.modifiedUnix - left.modifiedUnix).slice(0, 4));
  let hasMedia = $derived(fileCount > 0);
  let focus = $derived<"library" | "run" | "add">(activityLabel ? "run" : hasMedia ? "library" : "add");
  let checkSummary = $derived.by(() => {
    if (environmentLoading && checks.length === 0) return "Checking the installed tools…";
    if (checks.length === 0) return "Tools have not been checked";
    if (missingRequired.length > 0) return `${missingRequired.length} required tool${missingRequired.length === 1 ? "" : "s"} missing`;
    return `${readyChecks} tools ready`;
  });
  let intro = $derived.by(() => {
    if (activityLabel) return activityLabel;
    if (hasMedia) {
      const files = `${fileCount} file${fileCount === 1 ? "" : "s"}`;
      const sources = `${sourceCount} source${sourceCount === 1 ? "" : "s"}`;
      return activeFolder ? `${files} across ${sources} · ${activeFolder}` : `${files} across ${sources}`;
    }
    if (sourceCount > 0 && activeFolder) return `${activeFolder} is in the queue. This scan did not match any video files.`;
    return "Scan media, review the match, then write metadata or rebuild.";
  });
  let queueLine = $derived(statusCounts.map((item) => `${item.count} ${item.label.toLowerCase()}`).join(" · "));

  function stamp(unixSeconds: number): string {
    return new Date(unixSeconds * 1000).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  }

  function reportName(report: RunReport): string {
    return (report.csv_path.split(/[\\/]/u).filter(Boolean).at(-1) ?? "Run").replace(/\.csv$/iu, "");
  }

  function reportLine(report: RunReport): string {
    const parts = Object.entries(report.counts ?? {}).filter(([, count]) => count > 0).map(([status, count]) => `${count} ${status.replaceAll("_", " ")}`);
    return parts.length ? parts.join(" · ") : `${report.rows.length} files`;
  }

  function checkDetail(check: CommandCheck): string {
    if (!check.available) return check.required ? "Required" : "Not found";
    const token = (check.version ?? "").match(/\d+\.\d+(?:\.\d+)?(?:-[A-Za-z0-9.+]+)?/u)?.[0];
    return token || "Ready";
  }
</script>

<section class="page-heading">
  <div>
    <div class="eyebrow">WORKSPACE</div>
    <h1>Home</h1>
    <p>{intro}</p>
  </div>
</section>

<div class="home-layout">
  <div class="home-actions">
    <article class="home-action" class:home-action-primary={focus === "add" || focus === "library"}>
      <p class="home-kicker">Library</p>
      <h2>{hasMedia ? "Review the library" : "Add media"}</h2>
      <p>{hasMedia ? "Posters, titles, and the proposed filename are on the library cards." : "Scan a folder or files into one queue. Posters and titles appear as each file is matched."}</p>
      {#if hasMedia && queueLine}<p class="home-queue">{queueLine}</p>{/if}
      <div class="home-action-buttons">
        {#if hasMedia}
          <button class="button" class:primary={focus === "library"} class:secondary={focus !== "library"} type="button" onclick={() => onOpen("library")}>Open library</button>
          <button class="button secondary" type="button" onclick={onAddFolder}>Add folder</button>
        {:else}
          <button class="button primary" type="button" onclick={onAddFolder}>Add folder</button>
          <button class="button secondary" type="button" onclick={onAddFiles}>Add files</button>
        {/if}
      </div>
    </article>

    <article class="home-action">
      <p class="home-kicker">Organize</p>
      <h2>Confirm the match</h2>
      <p>Review titles, posters, season and episode text, and the filename before anything is written.</p>
      <div class="home-action-buttons">
        <button class="button secondary" type="button" onclick={() => onOpen("metadata")}>Open organizer</button>
      </div>
    </article>

    <article class="home-action" class:home-action-primary={focus === "run"}>
      <p class="home-kicker">Rebuild</p>
      <h2>Write metadata or rebuild</h2>
      <p>Metadata-only keeps the video. A full rebuild encodes it to HEVC after validation.</p>
      {#if activityLabel}<p class="home-queue">{activityLabel}</p>{/if}
      <div class="home-action-buttons">
        <button class="button" class:primary={focus === "run"} class:secondary={focus !== "run"} type="button" onclick={() => onOpen("run")}>Open run setup</button>
      </div>
    </article>

    <article class="home-action">
      <p class="home-kicker">YouTube</p>
      <h2>Prepare an upload</h2>
      <p>Make a separate copy for YouTube. The original file stays in place.</p>
      <div class="home-action-buttons">
        <button class="button secondary" type="button" onclick={() => onOpen("youtube")}>Open YouTube prep</button>
      </div>
    </article>
  </div>

  <section class="home-panel" aria-labelledby="home-checks-title">
    <div class="home-panel-heading">
      <h2 id="home-checks-title">Tools</h2>
      <div class="home-panel-actions">
        <span>{checkSummary}</span>
        <button class="text-button" type="button" onclick={onRefreshChecks}>Refresh</button>
        {#if checks.length}<button class="text-button" type="button" onclick={() => (checksOpen = !checksOpen)}>{checksOpen ? "Hide tools" : "Show tools"}</button>{/if}
        {#if missingRequired.length}<button class="text-button" type="button" onclick={() => onOpen("run")}>Run setup</button>{/if}
      </div>
    </div>
    {#if missingRequired.length}
      <ul class="home-alerts">
        {#each missingRequired as check (check.key)}
          <li>
            <span class="home-mark" aria-hidden="true"><X size={14} /></span>
            <strong>{check.label}</strong>
            <small>Required</small>
          </li>
        {/each}
      </ul>
    {/if}
    {#if checksOpen && checks.length}
      <ul class="home-checks">
        {#each checks as check (check.key)}
          <li class="home-check" class:home-check-missing={check.required && !check.available}>
            <span class="home-mark" aria-hidden="true">{#if check.available}<Check size={14} />{:else}<X size={14} />{/if}</span>
            <strong>{check.label}</strong>
            <small>{checkDetail(check)}</small>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <div class="home-split">
    <section class="home-panel" aria-labelledby="home-runs-title">
      <div class="home-panel-heading">
        <h2 id="home-runs-title">Recent runs</h2>
        <button class="text-button" type="button" onclick={() => onOpen("reports")}>Reports</button>
      </div>
      {#if recentReports.length}
        <ul class="home-logs">
          {#each recentReports as report (report.csv_path)}
            <li>
              <button class="home-log" type="button" onclick={() => onOpenReport(report.csv_path)}>
                <strong>{reportName(report)}</strong>
                <small>{stamp(report.created)} · {reportLine(report)}</small>
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="home-quiet">No completed runs yet.</p>
      {/if}
    </section>

    <section class="home-panel" aria-labelledby="home-logs-title">
      <div class="home-panel-heading">
        <h2 id="home-logs-title">Setup logs</h2>
        <button class="text-button" type="button" onclick={() => onOpen("run")}>Run setup</button>
      </div>
      {#if recentLogs.length}
        <ul class="home-logs">
          {#each recentLogs as log (log.jobId)}
            <li class="home-log">
              <strong>Package or helper setup</strong>
              <small>{stamp(log.modifiedUnix)} · {log.sizeBytes >= 1048576 ? `${(log.sizeBytes / 1048576).toFixed(1)} MiB` : `${Math.max(1, Math.round(log.sizeBytes / 1024))} KiB`}</small>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="home-quiet">No package or helper setup logs yet.</p>
      {/if}
    </section>
  </div>
</div>
