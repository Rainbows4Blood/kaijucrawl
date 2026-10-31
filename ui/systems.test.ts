import { afterEach, expect, it, vi } from 'vitest';
import { screen, waitFor } from '@testing-library/dom';
import userEvent from '@testing-library/user-event';
import { createAnalysisDashboard } from './discovery';
import { createApp } from './app';
import { idleAnalysis, type Overview } from './analysis';
import { workspaces, type AnalysisWorkspace } from './workspaces';
import { fixture } from './test/fixture';
import type { Invoke } from './types';

let dispose: (() => void) | undefined;
afterEach(() => dispose?.());
function totals(): Overview {
  return { catalogue_total: 10, analyses: Object.fromEntries(Object.values(workspaces).flatMap(w=>w.stages).map(s=>[s.algorithm,{eligible:10,completed:10,prerequisite_pending:0,counts:{unknown:10}}])) };
}
function setup(workspace: AnalysisWorkspace, overview = totals()) {
  document.body.innerHTML = '<div id="test"></div>';
  const state = structuredClone(fixture), openDiscovery = vi.fn();
  const invoke = vi.fn(async (command: string) => {
    if (command === 'analysis_overview') return structuredClone(overview);
    if (command === 'snapshot') return structuredClone(state);
    if (command === 'start_system_analysis') state.analysis = { ...idleAnalysis(), state:'running', algorithm:workspaces[workspace].stages[0].algorithm, total:10, processed:2, overview:structuredClone(overview), discovery:{workspace,step:0,completed_steps:0,rerun_all:false,unsuccessful:0} };
    if (command === 'cancel_analysis') state.analysis.state = 'cancelled';
  });
  const view = createAnalysisDashboard(document.querySelector('#test')!, invoke as Invoke, vi.fn(), workspace, openDiscovery);
  dispose=()=>view.dispose(); view.setSnapshot(state);view.setVisible(true);
  return { view, state, invoke, overview, openDiscovery };
}

it.each(['dnd','osr','shadowrun','coc','warhammer_fantasy','warhammer_40k'] as const)('%s gates runs on all current Discovery results and accepts completed unknowns', async workspace=>{
  const coverage = totals();coverage.analyses['solo-play-1'].completed=9;
  const { view, state, invoke, openDiscovery } = setup(workspace,coverage);
  await screen.findByText('1 remaining');
  const action=screen.getByRole('button',{name:'Run analyses'}) as HTMLButtonElement;
  expect(action.disabled).toBe(true);
  expect(screen.getByRole('region',{name:'Discovery prerequisite'})).toBeTruthy();
  await userEvent.click(screen.getByRole('button',{name:'Open Discovery'}));
  expect(openDiscovery).toHaveBeenCalledOnce();
  expect(invoke.mock.calls.every(c=>c[0]==='analysis_overview')).toBe(true);
  coverage.analyses['solo-play-1'].completed=10;
  coverage.analyses['adventure-players-1'].prerequisite_pending=1;
  await userEvent.click(screen.getByRole('button',{name:'Refresh totals'}));
  await screen.findByText(/eligibility pending/);
  expect(action.disabled).toBe(true);
  coverage.analyses['adventure-players-1'].prerequisite_pending=0;
  await userEvent.click(screen.getByRole('button',{name:'Refresh totals'}));
  await waitFor(()=>expect(action.disabled).toBe(false));
  expect(screen.queryByRole('region',{name:'Discovery prerequisite'})).toBeNull();
  expect(screen.getAllByRole('progressbar')).toHaveLength(workspaces[workspace].stages.length);
  // A committed source change must relock the page, even after prior completion.
  state.analysis={...idleAnalysis(),state:'running',overview:totals()};
  state.analysis.overview!.analyses['weighted-rating-1'].completed=9;
  view.setSnapshot(structuredClone(state));
  state.analysis.state='completed';view.setSnapshot(structuredClone(state));
  coverage.analyses['weighted-rating-1'].completed=9;
  await userEvent.click(screen.getByRole('button',{name:'Refresh totals'}));
  await waitFor(()=>expect(action.disabled).toBe(true));
});

it.each(['dnd','osr','shadowrun','coc','warhammer_fantasy','warhammer_40k'] as const)('%s runs one sequence with optional rerun and the same Cancel button',async workspace=>{
  const { view, state, invoke }=setup(workspace);
  const action=screen.getByRole('button',{name:'Run analyses'}) as HTMLButtonElement;
  await waitFor(()=>expect(action.disabled).toBe(false));
  await userEvent.click(screen.getByRole('checkbox',{name:/Rerun all analyses/}));
  await userEvent.click(action);
  expect(invoke).toHaveBeenCalledWith('start_system_analysis',{workspace,rerunAll:true});
  expect(screen.getByRole('button',{name:'Cancel analysis'})).toBe(action);
  await screen.findByText(`Step 1 of ${workspaces[workspace].stages.length} · ${workspaces[workspace].stages[0].name}`);
  const step=workspaces[workspace].stages.length>1?1:0;
  state.analysis.discovery!.step=step;state.analysis.discovery!.completed_steps=step;
  state.analysis.algorithm=workspaces[workspace].stages[step].algorithm;
  view.setSnapshot(structuredClone(state));
  expect(screen.getByText(`Step ${step+1} of ${workspaces[workspace].stages.length} · ${workspaces[workspace].stages[step].name}`)).toBeTruthy();
  expect(invoke.mock.calls.filter(c=>c[0]==='analysis_overview')).toHaveLength(1);
  await userEvent.click(action);
  expect(invoke).toHaveBeenCalledWith('cancel_analysis');
  await waitFor(()=>expect(action.textContent).toBe('Run analyses'));
  state.analysis.state='running';state.analysis.discovery!.workspace='discovery';
  view.setSnapshot(structuredClone(state));
  expect(action.disabled).toBe(true);
  expect(screen.queryByRole('button',{name:'Cancel analysis'})).toBeNull();
  expect(screen.getByText('Discovery analysis is running.')).toBeTruthy();
});

it('fails closed if coverage cannot be read',async()=>{
  const { invoke }=setup('shadowrun');
  const action=screen.getByRole('button',{name:'Run analyses'}) as HTMLButtonElement;
  await waitFor(()=>expect(action.disabled).toBe(false));
  invoke.mockImplementation(async()=>{throw new Error('Synthetic totals failure');});
  await userEvent.click(screen.getByRole('button',{name:'Refresh totals'}));
  await screen.findByText(/Totals could not be refreshed/);
  expect(action.disabled).toBe(true);
});

it('shows OSR subset coverage and overlapping categories without inflating analyzed book totals',async()=>{
  const coverage=totals();coverage.catalogue_total=1000;
  coverage.analyses['osr-systems-1']={eligible:12,completed:10,prerequisite_pending:0,counts:{identified:7,not_osr:3,osric:5,old_school_essentials:6,other:4}};
  coverage.analyses['osr-levels-1']={eligible:5,completed:3,prerequisite_pending:2,counts:{known:2,unknown:2,not_applicable:1}};
  const { view,state }=setup('osr',coverage);
  await screen.findByText('10 of 12 eligible books analyzed');
  expect(document.querySelector('#osr-books')?.textContent).toBe('12');
  expect(document.querySelector('#osr-saved')?.textContent).toBe('13');
  expect(document.querySelector('#osr-0-osric')?.textContent).toBe('5');
  expect(document.querySelector('#osr-0-old_school_essentials')?.textContent).toBe('6');
  expect(document.querySelector('#osr-0-other')?.textContent).toBe('4');
  expect(document.querySelector('#osr-0-not_osr')?.textContent).toBe('3');
  expect(screen.getByText(/Only books with the saved Rule System label/)).toBeTruthy();
  expect(screen.getByText('3 of 5 eligible books analyzed')).toBeTruthy();
  expect(document.querySelector('#osr-1-known')?.textContent).toBe('2');
  expect(document.querySelector('#osr-1-unknown')?.textContent).toBe('2');
  expect(document.querySelector('#osr-1-not_applicable')?.textContent).toBe('1');
  expect(screen.getByText(/2 books still need an Adventure role and OSR system classification/)).toBeTruthy();
  state.analysis={...idleAnalysis(),state:'running',algorithm:'osr-systems-1',overview:coverage,discovery:{workspace:'osr',step:0,completed_steps:0,rerun_all:false,unsuccessful:0}};
  view.setSnapshot(structuredClone(state));
  state.analysis.state='completed';state.analysis.discovery!.completed_steps=2;
  view.setSnapshot(structuredClone(state));
  expect(screen.getByText('Sequence finished. All 2 analyses have been checked.')).toBeTruthy();
});

it('provides themed system workspaces, icons, isolated tabs and unique IDs without starting work',async()=>{
  document.body.innerHTML='<div id="app"></div>';
  const invoke=vi.fn(async (command:string)=>command==='snapshot'?structuredClone(fixture):command==='analysis_overview'?totals():undefined);
  const root=document.querySelector<HTMLElement>('#app')!;
  const app=createApp(root,invoke as Invoke,60000);dispose=()=>app.dispose();
  await screen.findByRole('tab',{name:'Overview'});
  const user=userEvent.setup();
  for (const workspace of ['dnd','osr','shadowrun','coc','warhammer_fantasy','warhammer_40k','discovery'] as const) {
    await user.click(screen.getByRole('button',{name:'Open main menu'}));
    const button=document.querySelector<HTMLButtonElement>(`#menu-${workspace}`)!;
    expect(button.querySelector('svg[aria-hidden=true]')).toBeTruthy();
    await user.click(button);
    expect(root.dataset.theme).toBe(workspace==='discovery'?'default':workspace);
    expect(screen.getAllByRole('tab')).toHaveLength(workspace==='warhammer_40k'?9:1);
    const tab=screen.getByRole('tab',{name:'Analysis'});
    expect(document.activeElement).toBe(tab);
    expect(screen.getByRole('tabpanel',{name:'Analysis'})).toBeTruthy();
    expect(screen.getByRole('heading',{name:workspaces[workspace].heading})).toBeTruthy();
    await waitFor(()=>expect((screen.getByRole('button',{name:'Run analyses'}) as HTMLButtonElement).disabled).toBe(false));
  }
  const ids=[...root.querySelectorAll('[id]')].map(e=>e.id);
  expect(new Set(ids).size).toBe(ids.length);
  await user.click(screen.getByRole('button',{name:'Open main menu'}));
  await user.click(document.querySelector('#menu-crawlers')!);
  expect(root.dataset.theme).toBe('default');
  expect(screen.getAllByRole('tab')).toHaveLength(5);
  expect(invoke.mock.calls.every(c=>['snapshot','analysis_overview'].includes(c[0]))).toBe(true);
});
