import assert from 'node:assert/strict';
import test from 'node:test';
import { initialWorkspaceState, selectQueuePaths, RunEventBuffer, reportOutcome } from '../src/lib/workspace-state.ts';
const finished = (override = {}) => ({ runId: 'run', exitCode: 0, stopRequested: false, logPath: null, reportPath: null, error: null, ...override });
test('independent empty workspace and stable populated selection preserve other sources', () => {
  const first = initialWorkspaceState(), second = initialWorkspaceState();
  first.queue.selectedPaths = selectQueuePaths([], ['/synthetic/a', '/synthetic/a'], true);
  first.queue.selectedPaths = selectQueuePaths(first.queue.selectedPaths, ['/synthetic/b'], true);
  assert.deepEqual(selectQueuePaths(first.queue.selectedPaths, ['/synthetic/a'], false), ['/synthetic/b']);
  assert.deepEqual(second.queue.selectedPaths, []);
  assert.equal(first.run.id, '');
});
test('active-run early events claim exact identity and ignore stale finishes', () => {
  const buffer = new RunEventBuffer();
  buffer.line({ runId: 'run', line: 'valid diagnostic' });
  buffer.line({ runId: 'stale', line: 'old diagnostic' });
  buffer.finish(finished());
  buffer.finish(finished({ runId: 'stale', error: 'old failure' }));
  const claimed = buffer.claim('run');
  assert.deepEqual(claimed.lines, ['valid diagnostic']);
  assert.equal(claimed.finished.error, null);
  assert.equal(buffer.claim('unrelated').finished, null);
  for (let i = 0; i < 1000; i++) buffer.line({ runId: 'run', line: `line ${i}` });
  assert.equal(buffer.claim('run').lines.length, 240);
  buffer.reset(); assert.deepEqual(buffer.claim('run').lines, []);
});
test('report evidence distinguishes successful, failed, cancelled and uncertain/recovered files', () => {
  for (const status of ['OK', 'METADATA_OK', 'TEST_OK', 'INFO']) assert.equal(reportOutcome(status, true, finished()), 'completed');
  assert.equal(reportOutcome('FAILED', true, finished()), 'failed');
  assert.equal(reportOutcome('SKIPPED', true, finished()), 'skipped');
  assert.equal(reportOutcome('INTERRUPTED', true, finished()), 'interrupted');
  assert.equal(reportOutcome(null, false, finished()), 'needs_review');
  assert.equal(reportOutcome(null, false, finished({ stopRequested: true })), 'queued');
  assert.equal(reportOutcome(null, false, finished({ exitCode: 1 })), 'interrupted');
  assert.equal(reportOutcome(null, false, finished({ error: 'lost engine' })), 'interrupted');
  assert.equal(reportOutcome('unknown', true, finished()), 'needs_review');
  assert.equal(reportOutcome('OK', true, finished({ error: 'later file failed' })), 'completed');
});
