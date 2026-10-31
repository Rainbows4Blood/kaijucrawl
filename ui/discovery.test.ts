import { afterEach, expect, it, vi } from 'vitest';
import { screen, waitFor } from '@testing-library/dom';
import userEvent from '@testing-library/user-event';
import { createDiscovery } from './discovery';
import { idleAnalysis, type Overview } from './analysis';
import { fixture } from './test/fixture';
import type { Invoke, Snapshot } from './types';

let dispose: (() => void) | undefined;
afterEach(() => dispose?.());
const totals = (): Overview => ({ catalogue_total: 100, analyses: {
  'weighted-rating-1': { eligible: 100, completed: 80, prerequisite_pending: 0, counts: { scored: 60, unknown: 20 } },
  'primary-book-role-1': { eligible: 100, completed: 50, prerequisite_pending: 0, counts: { adventure: 20, supplement: 20, core_rulebook: 5, none: 5 } },
  'solo-play-1': { eligible: 100, completed: 100, prerequisite_pending: 0, counts: { neither: 90, solo_adventure: 5, gamebook: 5 } },
  'adventure-players-1': { eligible: 20, completed: 10, prerequisite_pending: 50, counts: { known: 7, unknown: 3 } },
} });
function setup(patch: Partial<Snapshot> = {}) {
  const state = { ...structuredClone(fixture), ...patch };
  document.body.innerHTML = '<div id="discovery"></div>';
  const invoke = vi.fn(async (command: string): Promise<unknown> => {
    if (command === 'analysis_overview') return totals();
    if (command === 'snapshot') return structuredClone(state);
    if (command === 'start_discovery_analysis') state.analysis = { ...idleAnalysis(), state: 'running', total: 20, processed: 3, overview: totals(), discovery: { step: 0, completed_steps: 0, rerun_all: false, unsuccessful: 0 } };
    if (command === 'cancel_analysis') state.analysis.state = 'cancelled';
  });
  const notify = vi.fn();
  const view = createDiscovery(document.querySelector('#discovery')!, invoke as Invoke, notify);
  dispose = () => view.dispose(); view.setSnapshot(state);
  return { view, state, invoke, notify };
}
it('shows four system-independent coverage cards without a selector or automatic paid work', async () => {
  const { view, invoke } = setup();
  expect(invoke).not.toHaveBeenCalled();
  view.setVisible(true);
  await screen.findByText('80 of 100 eligible books analyzed');
  expect(screen.queryByRole('combobox')).toBeNull();
  expect(screen.getAllByRole('progressbar')).toHaveLength(4);
  expect(screen.getByText('50 books still need a primary role before eligibility is known.')).toBeTruthy();
  expect(document.querySelector('#discovery-ready')?.textContent).toBe('1 / 4');
  expect(document.querySelector('#discovery-saved')?.textContent).toBe('240');
  expect(screen.queryByText(/D&D/)).toBeNull();
  expect(screen.getAllByRole('button', { name: 'Run analyses' })).toHaveLength(1);
  expect(invoke.mock.calls.map(c => c[0])).toEqual(['analysis_overview']);
});
it('starts one backend sequence, keeps Cancel in place and retains the explicit rerun-all choice', async () => {
  const { view, state, invoke, notify } = setup(); view.setVisible(true);
  await screen.findByText('80 of 100 eligible books analyzed');
  const user = userEvent.setup(), action = screen.getByRole('button', { name: 'Run analyses' });
  await user.click(action);
  await screen.findByText('Step 1 of 4 · Weighted relevance');
  expect(invoke).toHaveBeenCalledWith('start_discovery_analysis', { rerunAll: false });
  expect(screen.getByRole('button', { name: 'Cancel analysis' })).toBe(action);
  expect(notify).toHaveBeenCalled();
  state.analysis.algorithm = 'primary-book-role-1';
  state.analysis.discovery!.step = 1; state.analysis.discovery!.completed_steps = 1;
  state.analysis.overview!.analyses['primary-book-role-1'].completed = 51;
  view.setSnapshot(structuredClone(state));
  expect(screen.getByText('Step 2 of 4 · Primary book role')).toBeTruthy();
  expect(screen.getByText('51 of 100 eligible books analyzed')).toBeTruthy();
  view.setVisible(false); view.setVisible(true);
  expect(invoke.mock.calls.filter(c => c[0] === 'start_discovery_analysis')).toHaveLength(1);
  await user.click(action);
  await screen.findByRole('button', { name: 'Run analyses' });
  expect(invoke).toHaveBeenCalledWith('cancel_analysis');
  await user.click(screen.getByRole('checkbox', { name: /Rerun all analyses/ }));
  await user.click(action);
  expect(invoke).toHaveBeenCalledWith('start_discovery_analysis', { rerunAll: true });
  expect(invoke.mock.calls.some(c => ['start_analysis','start_classification','start_player_analysis','start_dnd_classification'].includes(c[0]))).toBe(false);
});
it('uses live committed totals without aggregate polling and rejects an older read', async () => {
  const { view, state, invoke } = setup();
  let finish!: (value: Overview) => void;
  invoke.mockImplementation(async command => command === 'analysis_overview' ? new Promise<Overview>(resolve => { finish = resolve; }) : structuredClone(state));
  view.setVisible(true);
  state.analysis = { ...idleAnalysis(), algorithm: 'primary-book-role-1', state: 'running', total: 50, processed: 2, overview: totals(), discovery: { step: 1, completed_steps: 1, rerun_all: false, unsuccessful: 0 } };
  state.analysis.overview!.analyses['primary-book-role-1'].completed = 52;
  view.setSnapshot(structuredClone(state));
  finish(totals());
  await waitFor(() => expect((screen.getByRole('button', { name: 'Refresh totals' }) as HTMLButtonElement).disabled).toBe(false));
  expect(screen.getByText('52 of 100 eligible books analyzed')).toBeTruthy();
  state.analysis.overview!.analyses['primary-book-role-1'].completed = 53;
  view.setSnapshot(structuredClone(state)); view.setVisible(false); view.setVisible(true);
  await userEvent.click(screen.getByRole('button', { name: 'Refresh totals' }));
  expect(screen.getByText('53 of 100 eligible books analyzed')).toBeTruthy();
  expect(invoke.mock.calls.filter(c => c[0] === 'analysis_overview')).toHaveLength(1);
});
it('prevents duplicate starts and gates storage/crawling while keeping safe errors visible', async () => {
  const { view, state, invoke } = setup({ active: true }); view.setVisible(true);
  const action = screen.getByRole('button', { name: 'Run analyses' }) as HTMLButtonElement;
  expect(action.disabled).toBe(true);
  state.active = false; state.storage_ok = false; view.setSnapshot(structuredClone(state)); expect(action.disabled).toBe(true);
  state.storage_ok = true; view.setSnapshot(structuredClone(state));
  let reject!: (error: unknown) => void;
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(command => command === 'start_discovery_analysis' ? new Promise((_, fail) => { reject = fail; }) : original(command));
  await userEvent.click(action); await userEvent.click(action);
  expect(invoke.mock.calls.filter(c => c[0] === 'start_discovery_analysis')).toHaveLength(1);
  reject(new Error('Synthetic ownership conflict'));
  await screen.findByText(/Synthetic ownership conflict/);
  expect(action.disabled).toBe(false);
});
it('reports retry/escalation, terminal unsuccessful results and recoverable aggregate errors', async () => {
  const { view, state, invoke } = setup(); view.setVisible(true);
  await screen.findByText('80 of 100 eligible books analyzed');
  state.analysis = { ...idleAnalysis(), algorithm: 'adventure-players-1', state: 'running', total: 10, processed: 2, overview: totals(), discovery: { step: 3, completed_steps: 3, rerun_all: true, unsuccessful: 1 }, classification: { workers: 8, selected: 10, already_classified: 0, reclassify: true, requests: 2, input_tokens: 10, output_tokens: 0, cached_tokens: 0, cache_write_tokens: 0, core_rulebook: 0, supplement: 0, adventure: 0, none: 0, retrying: 1, retry_status: 503, retry_reason: 'Model capacity unavailable', retry_in_secs: 4, last_escalation: 'Book #42: advancing to Terra.' } };
  view.setSnapshot(structuredClone(state));
  expect(screen.getByText(/HTTP 503.*retrying automatically in 4s.*Most recent escalation/)).toBeTruthy();
  expect(screen.getByText(/rerunning saved results/)).toBeTruthy();
  state.analysis.state = 'completed'; state.analysis.discovery!.completed_steps = 4;
  view.setSnapshot(structuredClone(state));
  expect(screen.getByText(/Sequence finished.*1 unsuccessful results/)).toBeTruthy();
  invoke.mockImplementation(async () => { throw new Error('Synthetic totals unavailable'); });
  await userEvent.click(screen.getByRole('button', { name: 'Refresh totals' }));
  await screen.findByText(/Totals could not be refreshed/);
  expect(screen.queryByRole('progressbar')).toBeNull();
  invoke.mockImplementation(async () => totals());
  await userEvent.click(screen.getByRole('button', { name: 'Refresh totals' }));
  await screen.findByText('80 of 100 eligible books analyzed');
});
