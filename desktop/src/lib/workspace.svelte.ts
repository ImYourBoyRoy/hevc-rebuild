import { initialWorkspaceState } from './workspace-state.js';
/** One reactive owner; the route coordinates native calls and durable persistence. */
export function createWorkspaceState() { const state = $state(initialWorkspaceState()); return state; }
