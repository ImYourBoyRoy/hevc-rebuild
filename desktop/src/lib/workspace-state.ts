import type {
  MediaQueueRunState, MediaQueueSource, RunFinishedEvent, RunLineEvent,
  RunMode, RunProgressEvent, RunResourceEvent, RuntimeResourceSnapshot,
} from './types.js';

export interface QueueState {
  sources: MediaQueueSource[];
  activeRoot: string | null;
  loaded: boolean;
  file: string;
  persistenceWarning: string;
  persistenceError: string;
  selectedPaths: string[];
  runStates: Record<string, MediaQueueRunState>;
}
export interface RunState {
  id: string;
  starting: boolean;
  mode: RunMode;
  stopAfterCurrent: boolean;
  aborting: boolean;
  lines: string[];
  progress: RunProgressEvent | null;
  startedAt: number | null;
  progressSeenAt: number | null;
  resources: RuntimeResourceSnapshot | null;
  message: string;
}
export interface NotificationState {
  message: string;
  kind: 'info' | 'success' | 'error';
}
export interface WorkspaceState { queue: QueueState; run: RunState; notice: NotificationState }
export function initialWorkspaceState(): WorkspaceState {
  return {
    queue: { sources: [], activeRoot: null, loaded: false, file: '', persistenceWarning: '', persistenceError: '', selectedPaths: [], runStates: {} },
    run: { id: '', starting: false, mode: 'check', stopAfterCurrent: false, aborting: false, lines: [], progress: null, startedAt: null, progressSeenAt: null, resources: null, message: '' },
    notice: { message: '', kind: 'info' },
  };
}

/** Selection is exact-path and stable-order, matching the persisted queue interface. */
export function selectQueuePaths(current: string[], paths: Iterable<string>, selected: boolean): string[] {
  const changed = new Set(paths);
  return selected ? [...new Set([...current, ...changed])] : current.filter(path => !changed.has(path));
}

/** Move one queued item; out-of-range indexes leave the list unchanged. */
export function moveQueueItem<T>(items: readonly T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= items.length || to >= items.length) return [...items];
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

/** Hide removed files and restore a saved order after a refresh. */
export function applyQueueFilePolicy<T extends { path: string }>(
  entries: readonly T[],
  excludedPaths: readonly string[],
  entryOrder: readonly string[],
  pathKey: (path: string) => string = (path) => path,
): T[] {
  const excluded = new Set(excludedPaths.map(pathKey));
  const visible = entries.filter((entry) => !excluded.has(pathKey(entry.path)));
  const order = new Map(entryOrder.map((path, index) => [pathKey(path), index]));
  return [...visible].sort((left, right) => {
    const leftIndex = order.get(pathKey(left.path)) ?? Number.MAX_SAFE_INTEGER;
    const rightIndex = order.get(pathKey(right.path)) ?? Number.MAX_SAFE_INTEGER;
    return leftIndex - rightIndex;
  });
}

/** Events can arrive before start_run resolves; only the returned run ID is accepted. */
export class RunEventBuffer {
  lines: RunLineEvent[] = [];
  progress: RunProgressEvent[] = [];
  resources: RunResourceEvent[] = [];
  finished: RunFinishedEvent[] = [];
  reset() { this.lines = []; this.progress = []; this.resources = []; this.finished = []; }
  line(event: RunLineEvent) { this.lines = [...this.lines, event].slice(-240); }
  update(event: RunProgressEvent) { this.progress = [...this.progress, event].slice(-8); }
  sample(event: RunResourceEvent) { this.resources = [...this.resources, event].slice(-8); }
  finish(event: RunFinishedEvent) { this.finished = [...this.finished, event].slice(-8); }
  claim(runId: string) {
    return {
      lines: this.lines.filter(event => event.runId === runId).map(event => event.line).slice(-240),
      progress: this.progress.filter(event => event.runId === runId).at(-1) ?? null,
      resources: this.resources.filter(event => event.runId === runId).at(-1)?.snapshot ?? null,
      finished: this.finished.filter(event => event.runId === runId).at(-1) ?? null,
    };
  }
}

/** A successful process exit alone cannot complete a file absent matching report evidence. */
export function reportOutcome(status: string | null | undefined, hasRow: boolean, finished: RunFinishedEvent): MediaQueueRunState['status'] {
  const normalized = status?.trim().toUpperCase() ?? null;
  if (['OK', 'METADATA_OK', 'TEST_OK', 'INFO', 'INTEGRITY_OK'].includes(normalized ?? '')) return 'completed';
  if (normalized === 'FAILED' || normalized === 'CORRUPT' || normalized === 'UNREADABLE') return 'failed';
  if (normalized === 'SKIPPED') return 'skipped';
  if (normalized === 'INTERRUPTED') return 'interrupted';
  if (!hasRow && finished.stopRequested) return 'queued';
  if (!hasRow && (finished.error || finished.exitCode !== 0)) return 'interrupted';
  return 'needs_review';
}
