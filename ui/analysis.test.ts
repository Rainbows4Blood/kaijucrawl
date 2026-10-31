import { afterEach,beforeEach,expect,it,vi } from 'vitest';
import { screen,waitFor } from '@testing-library/dom';
import userEvent from '@testing-library/user-event';
import { createAnalysis,idleAnalysis,type Overview } from './analysis';
import { fixture } from './test/fixture';
import type { Invoke,Snapshot } from './types';
let dispose:()=>void;
let showAnalysis:(visible?:boolean)=>void;
beforeEach(()=>{document.body.innerHTML='<div id="app"></div>';vi.stubGlobal('scrollTo',vi.fn());});
afterEach(()=>{dispose?.();vi.unstubAllGlobals();});
it('keeps cancellation and essential retry errors in the totals block without run statistics',()=>{
  const root=document.querySelector<HTMLElement>('#app')!;
  const invoke=vi.fn();const view=createAnalysis(root,invoke);dispose=()=>view.dispose();
  const state:Snapshot={...structuredClone(fixture),analysis:{...idleAnalysis(),id:3,algorithm:'primary-book-role-1',state:'running',total:10,processed:2,scored:2,classification:{workers:8,in_flight:0,retries:3,retrying:4,retry_in_secs:16,retry_status:429,selected:10,already_classified:0,reclassify:false,requests:9,input_tokens:900,cached_tokens:0,cache_write_tokens:0,output_tokens:0,core_rulebook:0,supplement:0,adventure:2,none:0}}};
  view.setSnapshot(state);
  expect(screen.getByText(/Primary book role: HTTP 429 · Rate or quota limit · retrying automatically in 16s/)).toBeTruthy();
  expect(screen.queryByText(/requests dispatched/)).toBeNull();
  expect((screen.getByRole('button',{name:'Cancel analysis'}) as HTMLButtonElement).disabled).toBe(false);
  Object.assign(state.analysis.classification!,{retry_status:503,retry_in_secs:7200});view.setSnapshot(state);
  expect(screen.getByText(`Primary book role: HTTP 503 · Model capacity unavailable · retrying automatically in ${new Intl.NumberFormat().format(7200)}s.`)).toBeTruthy();
  Object.assign(state.analysis.classification!,{retry_status:502,retry_reason:'API request failed'});view.setSnapshot(state);
  expect(screen.getByText(/HTTP 502 · API request failed/)).toBeTruthy();
  for(const reason of ['Network connection failed','Request timed out','Invalid model response']){
    Object.assign(state.analysis.classification!,{retry_status:null,retry_reason:reason,retry_in_secs:2});view.setSnapshot(state);
    expect(root.querySelector<HTMLElement>('#analysis-status')!.hidden).toBe(false);
    expect(screen.getByText(`Primary book role: ${reason} · retrying automatically in 2s.`)).toBeTruthy();
    expect((screen.getByRole('button',{name:'Cancel analysis'}) as HTMLButtonElement).disabled).toBe(false);
  }
  state.analysis.state='cancelled';view.setSnapshot(state);
  expect(root.querySelector<HTMLElement>('#analysis-status')!.hidden).toBe(true);
  expect(screen.queryByText(/Invalid model response/)).toBeNull();
  state.analysis.state='failed';state.analysis.detail='Synthetic local storage failure';view.setSnapshot(state);
  expect(screen.getByText('Primary book role stopped: Synthetic local storage failure')).toBeTruthy();
  expect(screen.queryByText('Latest run activity')).toBeNull();
  expect(invoke).not.toHaveBeenCalled();
});
function overviewFixture():Overview{return {catalogue_total:120,analyses:{
 'weighted-rating-1':{eligible:120,completed:100,prerequisite_pending:0,counts:{scored:90,unknown:10}},
 'primary-book-role-1':{eligible:120,completed:100,prerequisite_pending:0,counts:{core_rulebook:20,supplement:30,adventure:35,none:15}},
 'adventure-players-1':{eligible:35,completed:35,prerequisite_pending:20,counts:{known:25,unknown:10}},
 'solo-play-1':{eligible:120,completed:120,prerequisite_pending:0,counts:{solo_adventure:10,gamebook:5,neither:105}},
 'dnd-compatibility-1':{eligible:120,completed:120,prerequisite_pending:0,counts:{dnd_only:40,dnd_and_others:10,no:70}},
 'dnd-editions-1':{eligible:50,completed:30,prerequisite_pending:0,counts:{identified:20,unknown:10,bx:10,'5e_2014':20}},
 'dnd-settings-1':{eligible:50,completed:35,prerequisite_pending:0,counts:{identified:20,any:5,unknown:10,forgotten_realms:12,al_qadim:4}},
 'dnd-levels-1':{eligible:50,completed:40,prerequisite_pending:0,counts:{known:25,unknown:15}},
 'dnd-locations-1':{eligible:50,completed:20,prerequisite_pending:0,counts:{known:10,unknown:10}},
 'dnd-date-1':{eligible:50,completed:10,prerequisite_pending:0,counts:{known:2,unknown:8}},
 'shadowrun-editions-1':{eligible:12,completed:8,prerequisite_pending:0,counts:{identified:6,unknown:2,anarchy:3,anarchy_2:4}},
 'coc-editions-1':{eligible:20,completed:12,prerequisite_pending:0,counts:{identified:9,unknown:3,'6e':7,'7e':8}},
 'shadowrun-location-1':{eligible:12,completed:8,prerequisite_pending:0,counts:{known:6,unknown:2}},
 'shadowrun-date-1':{eligible:12,completed:6,prerequisite_pending:0,counts:{known:4,unknown:2}},
 'coc-location-1':{eligible:20,completed:15,prerequisite_pending:0,counts:{known:5,unknown:10}},
 'coc-date-1':{eligible:20,completed:10,prerequisite_pending:0,counts:{known:8,unknown:2}}
}};}
it('runs four independent D&D analyses with saved totals, distinct fallbacks and per-analysis rerun choices',async()=>{
 const {invoke,state}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
 const original=invoke.getMockImplementation()!;
 invoke.mockImplementation(async(cmd,args)=>{
  if(cmd==='start_dnd_settings'||cmd==='start_dnd_extraction'){
   await original('start_classification',args);
   state.analysis.algorithm=cmd==='start_dnd_settings'?'dnd-settings-1':`dnd-${args!.kind}-1`;
   return;
  }
  return original(cmd,args);
 });
 showAnalysis();
 const select=screen.getByRole('combobox',{name:'Analysis type'});
 await user.selectOptions(select,'dnd_settings');
 expect(screen.getByText(/Al-Qadim is a subgroup of Forgotten Realms/)).toBeTruthy();
 await screen.findByText('35 of 50 eligible books have current saved results');
 expect(document.querySelector('[data-coverage=any]')!.textContent).toBe('5');
 expect(document.querySelector('[data-coverage=unknown]')!.textContent).toBe('10');
 expect(document.querySelector('[data-coverage=al_qadim]')!.textContent).toBe('4');
 expect(document.querySelector('[data-coverage=forgotten_realms]')!.textContent).toBe('12');
 expect(invoke.mock.calls.some(c=>c[0].startsWith('start_'))).toBe(false);
 const button=screen.getByRole('button',{name:'Classify books'});
 await user.click(button);await screen.findByText('D&D settings is running.');
 expect(invoke.mock.calls.find(c=>c[0]==='start_dnd_settings')![1]).toEqual({reclassify:false});
 expect(screen.getByRole('button',{name:'Cancel analysis'})).toBe(button);
 await user.click(button);await screen.findByRole('button',{name:'Classify books'});
 for(const [kind,completed] of [['levels',40],['locations',20],['date',10]] as const){
  await user.selectOptions(select,`dnd_${kind}`);
  await screen.findByText(`${completed} of 50 eligible books have current saved results`);
  expect((screen.getByRole('checkbox',{name:/Reanalyze saved results too/}) as HTMLInputElement).checked).toBe(false);
  expect(screen.getByText(/Shared instructions are cached/)).toBeTruthy();
  await user.click(screen.getByRole('button',{name:'Analyze books'}));
  await screen.findByRole('button',{name:'Cancel analysis'});
  expect(invoke.mock.calls.filter(c=>c[0]==='start_dnd_extraction').at(-1)![1]).toEqual({kind,reanalyze:false});
  expect(screen.getByRole('button',{name:'Cancel analysis'})).toBe(button);
  expect(screen.getAllByRole('progressbar')).toHaveLength(1);
  expect(screen.queryByText('Latest run activity')).toBeNull();
  await user.click(button);await screen.findByRole('button',{name:'Analyze books'});
  await user.click(screen.getByRole('checkbox',{name:/Reanalyze saved results too/}));
 }
 await user.selectOptions(select,'dnd_levels');
 expect((screen.getByRole('checkbox',{name:/Reanalyze saved results too/}) as HTMLInputElement).checked).toBe(true);
 await user.click(screen.getByRole('button',{name:'Analyze books'}));await screen.findByRole('button',{name:'Cancel analysis'});
 expect(invoke.mock.calls.filter(c=>c[0]==='start_dnd_extraction').at(-1)![1]).toEqual({kind:'levels',reanalyze:true});
});
it('selects four Rule System analyses without prerequisites and retains independent totals and rerun choices',async()=>{
 const {invoke,state}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
 const original=invoke.getMockImplementation()!;
 invoke.mockImplementation(async(cmd,args)=>{
  if(cmd==='start_earth_extraction'){await original('start_classification',args);state.analysis.algorithm=`${String(args!.kind).replace('_','-')}-1`;return;}
  return original(cmd,args);
 });
 showAnalysis();const select=screen.getByRole('combobox',{name:'Analysis type'});
 const button=screen.getByRole('button',{name:'Run analysis'});
 for(const [kind,done,total] of [['shadowrun_location',8,12],['shadowrun_date',6,12],['coc_location',15,20],['coc_date',10,20]] as const){
  await user.selectOptions(select,kind);await screen.findByText(`${done} of ${total} eligible books have current saved results`);
  expect(screen.getByText(/No previous analysis is required/)).toBeTruthy();
  expect(screen.queryAllByText(/Run D&D compatibility first/).every(el=>el.closest('[hidden]'))).toBe(true);
  expect((screen.getByRole('checkbox',{name:/Reanalyze saved results too/}) as HTMLInputElement).checked).toBe(false);
  if(kind==='coc_date')expect(screen.getByText(/Modern = 1980–2030; Dark Ages = 950–1050/)).toBeTruthy();
  if(kind.endsWith('location'))expect(screen.getByText(/Multiple supported places are joined with semicolons/)).toBeTruthy();
  if(kind==='shadowrun_location')expect(invoke.mock.calls.some(c=>c[0].startsWith('start_'))).toBe(false);
  await user.click(screen.getByRole('button',{name:'Analyze books'}));await screen.findByRole('button',{name:'Cancel analysis'});
  expect(screen.getByRole('button',{name:'Cancel analysis'})).toBe(button);expect(screen.getAllByRole('progressbar')).toHaveLength(1);
  expect(invoke.mock.calls.filter(c=>c[0]==='start_earth_extraction').at(-1)![1]).toEqual({kind,reanalyze:false});
  await user.click(button);await screen.findByRole('button',{name:'Analyze books'});
  await user.click(screen.getByRole('checkbox',{name:/Reanalyze saved results too/}));
 }
 await user.selectOptions(select,'shadowrun_date');expect((screen.getByRole('checkbox',{name:/Reanalyze saved results too/}) as HTMLInputElement).checked).toBe(true);
 await user.click(screen.getByRole('button',{name:'Analyze books'}));await screen.findByRole('button',{name:'Cancel analysis'});
 expect(invoke.mock.calls.filter(c=>c[0]==='start_earth_extraction').at(-1)![1]).toEqual({kind:'shadowrun_date',reanalyze:true});
 expect(invoke.mock.calls.some(c=>['start_dnd_extraction','start_dnd_classification','test_decisions'].includes(c[0]))).toBe(false);
});
function setup(patch:Partial<Snapshot>={}){
  const state={...structuredClone(fixture),...patch};
  const invoke=vi.fn(async(cmd:string,args?:Record<string,unknown>):Promise<unknown>=>{
    if(cmd==='snapshot')return structuredClone(state);
    if(cmd==='analysis_overview')return overviewFixture();
    if(cmd==='start_analysis'){state.analysis={...idleAnalysis(),id:1,state:'running',total:120,processed:50,scored:45,skipped:5};}
    if(cmd==='start_classification'){state.analysis={...idleAnalysis(),id:2,algorithm:'primary-book-role-1',state:'running',total:100,processed:40,scored:39,skipped:1,classification:{workers:8,in_flight:5,selected:120,already_classified:20,reclassify:Boolean(args?.reclassify),requests:45,input_tokens:20000,cached_tokens:0,cache_write_tokens:0,output_tokens:0,core_rulebook:10,supplement:9,adventure:15,none:5}};}
    if(cmd==='start_player_analysis'){state.analysis={...idleAnalysis(),id:3,algorithm:'adventure-players-1',scope:'adventures',state:'running',total:30,processed:10,scored:10,classification:{workers:8,in_flight:3,selected:35,already_classified:5,reclassify:Boolean(args?.reanalyze),requests:13,input_tokens:16000,cached_tokens:12000,cache_write_tokens:1500,output_tokens:200,core_rulebook:0,supplement:0,adventure:0,none:0,known_players:7,unknown_players:3}};}
    if(cmd==='cancel_analysis')state.analysis.state='cancelled';
  });
  // Retain the analysis workflows independently of the new crawler-only shell.
  const root=document.querySelector<HTMLElement>('#app')!;
  const view=createAnalysis(root,invoke as Invoke);root.hidden=true;
  showAnalysis=(visible=true)=>{root.hidden=!visible;view.setVisible(visible);};
  void invoke('snapshot').then(value=>view.setSnapshot(value as Snapshot));
  dispose=()=>view.dispose();return {state,invoke,view};
}
it('opens Analysis without calculating, explicitly runs the entire catalogue and keeps cancellation separate from crawling',async()=>{
  const {invoke}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
  showAnalysis();expect(screen.queryByRole('button',{name:/Start crawl/})).toBeNull();expect(invoke.mock.calls.map(c=>c[0]).filter(c=>c!=='analysis_overview')).toEqual(['snapshot']);
  const action=screen.getByRole('button',{name:'Run analysis'}),position=action.parentElement;
  await user.click(action);await screen.findByText('Weighted relevance is running.');
  expect(screen.getByRole('button',{name:'Cancel analysis'})).toBe(action);expect(action.parentElement).toBe(position);
  expect(position!.closest('.catalogue-coverage')).toBeNull();
  expect(invoke.mock.calls.find(c=>c[0]==='start_analysis')![1]).toBeUndefined();expect(screen.queryByRole('button',{name:'Run analysis'})).toBeNull();
  showAnalysis(false);
  showAnalysis();expect(screen.getByText('Weighted relevance is running.')).toBeTruthy();
  await user.click(screen.getByRole('button',{name:'Cancel analysis'}));await waitFor(()=>expect(screen.queryByRole('button',{name:'Cancel analysis'})).toBeNull());expect(screen.getByRole('button',{name:'Run analysis'})).toBe(action);expect(action.parentElement).toBe(position);
  expect(invoke.mock.calls.some(c=>['start','stop','test_access'].includes(c[0]))).toBe(false);
});
it('gates analysis during a crawl and retains failed start drafts with a visible error',async()=>{
  const {invoke,state,view}=setup({active:true});const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
  showAnalysis();expect((screen.getByRole('button',{name:'Run analysis'}) as HTMLButtonElement).disabled).toBe(true);expect(screen.getByText(/Stop crawling or reprocessing/)).toBeTruthy();
  state.active=false;view.setSnapshot(structuredClone(state));
  const original=invoke.getMockImplementation()!;invoke.mockImplementation(async(cmd,args)=>{if(cmd==='start_analysis')throw new Error('Synthetic ownership conflict');return original(cmd,args);});
  await user.click(screen.getByRole('button',{name:'Run analysis'}));await screen.findByText(/Synthetic ownership conflict/);expect((screen.getByRole('combobox',{name:'Analysis type'}) as HTMLSelectElement).value).toBe('weighted');
});
it('explicitly starts primary-role classification, shows cumulative categories, without a browsing dependency',async()=>{
  const {invoke}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
  showAnalysis();await user.selectOptions(screen.getByRole('combobox',{name:'Analysis type'}),'book_role');
  expect(invoke.mock.calls.map(c=>c[0]).filter(c=>c!=='analysis_overview')).toEqual(['snapshot']);expect(screen.getByText(/one book per paid request/)).toBeTruthy();
  expect((screen.getByRole('checkbox',{name:/Reclassify previously/}) as HTMLInputElement).checked).toBe(false);
  await user.click(screen.getByRole('button',{name:'Classify books'}));await screen.findByRole('button',{name:'Cancel analysis'});expect(screen.getByText('Primary book role is running.')).toBeTruthy();
  expect(invoke.mock.calls.find(c=>c[0]==='start_classification')![1]).toEqual({reclassify:false});
  await screen.findByText('100 of 120 eligible books have current saved results');expect(document.querySelector('[data-coverage=adventure]')!.textContent).toBe('35');expect(screen.queryByText(/requests dispatched/)).toBeNull();
  expect((screen.getByRole('button',{name:'Test Decisions API'}) as HTMLButtonElement).disabled).toBe(true);
  await user.click(screen.getByRole('button',{name:'Cancel analysis'}));await waitFor(()=>expect(screen.queryByRole('button',{name:'Cancel analysis'})).toBeNull());expect(screen.queryByText(/active \/ saving/)).toBeNull();
  await user.click(screen.getByRole('checkbox',{name:/Reclassify previously/}));await user.click(screen.getByRole('button',{name:'Classify books'}));await screen.findByRole('button',{name:'Cancel analysis'});expect(screen.getByText('Primary book role is running.')).toBeTruthy();
  expect(invoke.mock.calls.filter(c=>c[0]==='start_classification').at(-1)![1]).toEqual({reclassify:true});

});
it('tests Decisions only on click, prevents duplicate spending and reports usage or safe errors',async()=>{
  const {invoke}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
  showAnalysis();expect(invoke.mock.calls.map(c=>c[0]).filter(c=>c!=='analysis_overview')).toEqual(['snapshot']);
  const original=invoke.getMockImplementation()!;let finish!:(v:unknown)=>void;
  invoke.mockImplementation(async(cmd,args)=>cmd==='test_decisions'?new Promise(resolve=>{finish=resolve;}):original(cmd,args));
  const button=screen.getByRole('button',{name:'Test Decisions API'}) as HTMLButtonElement;
  await user.click(button);expect(button.disabled).toBe(true);await user.click(button);
  expect(invoke.mock.calls.filter(c=>c[0]==='test_decisions')).toHaveLength(1);
  finish({model:'gpt-6-luna',answers:[{type:'predicate'},{type:'refusal'},{type:'score'}],usage:{input_tokens:200,output_tokens:0,input_tokens_details:{cached_tokens:20,cache_write_tokens:0}}});
  await screen.findByText(/Connected · 2 answers · 1 refused · 200 input tokens/);expect(button.disabled).toBe(false);
  showAnalysis(false);showAnalysis();
  expect(invoke.mock.calls.filter(c=>c[0]==='test_decisions')).toHaveLength(1);
  invoke.mockImplementation(async(cmd,args)=>{if(cmd==='test_decisions')throw new Error('OPENAI_API_KEY is missing');return original(cmd,args);});
  await user.click(button);await screen.findByText(/OPENAI_API_KEY is missing/);expect(button.disabled).toBe(false);
  expect(invoke.mock.calls.filter(c=>c[0]==='test_decisions')).toHaveLength(2);
});

it('runs Adventure player extraction explicitly, preserves unknown results, and retains catalogue totals',async()=>{
  const {invoke}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
  showAnalysis();await user.selectOptions(screen.getByRole('combobox',{name:'Analysis type'}),'players');
  expect(invoke.mock.calls.map(c=>c[0]).filter(c=>c!=='analysis_overview')).toEqual(['snapshot']);expect(screen.getByText(/Run Primary book role first/)).toBeTruthy();expect(screen.getByText(/Prompt caching is enabled/)).toBeTruthy();
  await user.click(screen.getByRole('button',{name:'Analyze player counts'}));await screen.findByText('Adventure player counts is running.');
  expect(invoke.mock.calls.find(c=>c[0]==='start_player_analysis')![1]).toEqual({reanalyze:false});expect(document.querySelector('[data-coverage=known]')!.textContent).toBe('25');
  expect(document.querySelector('[data-coverage=unknown]')!.textContent).toBe('10');expect(screen.getByText('35 of 35 eligible books have current saved results')).toBeTruthy();
  await user.click(screen.getByRole('button',{name:'Cancel analysis'}));await waitFor(()=>expect(screen.queryByRole('button',{name:'Cancel analysis'})).toBeNull());
  await user.click(screen.getByRole('checkbox',{name:/Reanalyze saved player counts too/}));await user.click(screen.getByRole('button',{name:'Analyze player counts'}));await screen.findByText('Adventure player counts is running.');
  expect(invoke.mock.calls.filter(c=>c[0]==='start_player_analysis').at(-1)![1]).toEqual({reanalyze:true});

});

it('chooses solo play from a compact dropdown, runs independently and retains the selection',async()=>{
  const {invoke,state}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
  showAnalysis(false);
  showAnalysis();
  expect(screen.queryByRole('radio')).toBeNull();
  const select=screen.getByRole('combobox',{name:'Analysis type'}) as HTMLSelectElement;
  expect(select.options).toHaveLength(16);
  await user.selectOptions(select,'book_role');await user.click(screen.getByRole('checkbox',{name:/Reclassify previously/}));
  await user.selectOptions(select,'solo_play');expect((screen.getByRole('checkbox',{name:/Reclassify previously/}) as HTMLInputElement).checked).toBe(false);
  expect(screen.getByText(/Previous analysis results are never supplied/)).toBeTruthy();
  expect(invoke.mock.calls.some(c=>c[0]==='start_solo_classification')).toBe(false);
  const original=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(cmd,args)=>{
    if(cmd==='start_solo_classification'){
      await original('start_classification',args);
      Object.assign(state.analysis,{algorithm:'solo-play-1'});
      Object.assign(state.analysis.classification!,{solo_adventure:3,gamebook:2,neither:35});return;
    }
    return original(cmd,args);
  });
  await user.click(screen.getByRole('button',{name:'Classify books'}));await screen.findByRole('button',{name:'Cancel analysis'});
  expect(invoke.mock.calls.find(c=>c[0]==='start_solo_classification')![1]).toEqual({reclassify:false});
  expect(select.disabled).toBe(false);expect(select.value).toBe('solo_play');
  expect(document.querySelector('[data-coverage=neither]')!.textContent).toBe('105');
  expect(screen.getByText('Solo Adventure / Gamebook is running.')).toBeTruthy();
  showAnalysis(false);showAnalysis();expect(select.value).toBe('solo_play');
  await user.click(screen.getByRole('button',{name:'Cancel analysis'}));await waitFor(()=>expect(screen.queryByRole('button',{name:'Cancel analysis'})).toBeNull());expect(select.disabled).toBe(false);
  await user.click(screen.getByRole('checkbox',{name:/Reclassify previously/}));await user.click(screen.getByRole('button',{name:'Classify books'}));await screen.findByRole('button',{name:'Cancel analysis'});
  expect(invoke.mock.calls.filter(c=>c[0]==='start_solo_classification').at(-1)![1]).toEqual({reclassify:true});

});

it('shows lifetime coverage for the selected analysis while another run is active and handles refresh errors',async()=>{
 const {invoke}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
 showAnalysis();await screen.findByText('100 of 120 eligible books have current saved results');
 expect(screen.queryByText('Latest run activity')).toBeNull();expect(screen.getAllByRole('progressbar')).toHaveLength(1);
 await user.click(screen.getByRole('button',{name:'Run analysis'}));await screen.findByText('Weighted relevance is running.');
 const select=screen.getByRole('combobox',{name:'Analysis type'});await user.selectOptions(select,'solo_play');
 await screen.findByText('120 of 120 eligible books have current saved results');expect(document.querySelector('[data-coverage="neither"]')!.textContent).toBe('105');
 expect(screen.getByRole('button',{name:'Cancel analysis'})).toBeTruthy();expect(screen.getByText('Weighted relevance is running.')).toBeTruthy();
 await user.selectOptions(select,'dnd_editions');await screen.findByText('30 of 50 eligible books have current saved results');
 expect(screen.getAllByRole('progressbar')).toHaveLength(1);expect(screen.queryByText('50 of 120 processed')).toBeNull();expect(screen.queryByText(/requests dispatched/)).toBeNull();
 const bar=screen.getByRole('progressbar',{name:'Catalogue analysis coverage'}) as HTMLProgressElement;expect(bar.max).toBe(50);expect(bar.value).toBe(30);
 showAnalysis(false);showAnalysis();expect((select as HTMLSelectElement).value).toBe('dnd_editions');
 const original=invoke.getMockImplementation()!;invoke.mockImplementation(async(cmd,args)=>{if(cmd==='analysis_overview')throw new Error('Synthetic unavailable totals');return original(cmd,args);});
 await user.click(screen.getByRole('button',{name:'Refresh totals'}));await screen.findByText(/Totals could not be refreshed/);expect(document.querySelector<HTMLElement>('#analysis-progress')!.hidden).toBe(true);
 invoke.mockImplementation(async(cmd,args)=>{if(cmd==='analysis_overview'){const t=overviewFixture();t.analyses['dnd-editions-1'].completed=50;return t;}return original(cmd,args);});
 await user.click(screen.getByRole('button',{name:'Refresh totals'}));await screen.findByText('50 of 50 eligible books have current saved results');
 expect(invoke.mock.calls.filter(c=>c[0].startsWith('start_')).map(c=>c[0])).toEqual(['start_analysis']);
});
it('applies committed snapshot totals immediately and rejects stale reads without polling aggregates during analysis',async()=>{
 const root=document.querySelector<HTMLElement>('#app')!;
 let finish!:(v:Overview)=>void;
 const invoke=vi.fn(()=>new Promise<Overview>(resolve=>{finish=resolve;}));
 const view=createAnalysis(root,invoke as Invoke);dispose=()=>view.dispose();
 const state:Snapshot={...structuredClone(fixture),analysis:{...idleAnalysis(),id:20,algorithm:'dnd-editions-1',state:'running',total:20,processed:0,overview:overviewFixture()}};
 view.setSnapshot({...state,analysis:{...state.analysis,state:'idle',overview:null}});view.setVisible(true);
 expect(invoke).toHaveBeenCalledTimes(1);
 view.setSnapshot(state);
 const bar=screen.getByRole('progressbar',{name:'Catalogue analysis coverage'}) as HTMLProgressElement;
 expect(bar.value).toBe(30);expect(bar.max).toBe(50);
 state.analysis.overview!.analyses['dnd-editions-1'].completed=31;state.analysis.processed=1;view.setSnapshot(structuredClone(state));
 expect(bar.value).toBe(31);
 const stale=overviewFixture();stale.analyses['dnd-editions-1'].completed=0;finish(stale);
 await waitFor(()=>expect((screen.getByRole('button',{name:'Refresh totals'}) as HTMLButtonElement).disabled).toBe(false));
 expect(bar.value).toBe(31);
 state.analysis.overview!.analyses['dnd-editions-1'].completed=32;state.analysis.processed=2;view.setSnapshot(structuredClone(state));
 expect(bar.value).toBe(32);expect(invoke).toHaveBeenCalledTimes(1);
 const user=userEvent.setup();await user.selectOptions(screen.getByRole('combobox',{name:'Analysis type'}),'solo_play');
 expect(bar.value).toBe(120);expect(screen.getByText('D&D editions is running.')).toBeTruthy();
 view.setVisible(false);view.setSnapshot(structuredClone(state));view.setVisible(true);
 await user.click(screen.getByRole('button',{name:'Refresh totals'}));expect(invoke).toHaveBeenCalledTimes(1);
 expect(screen.getAllByRole('progressbar')).toHaveLength(1);
});
it('runs the two D&D analyses explicitly with their built-in scopes',async()=>{
 const {invoke,state}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
 const original=invoke.getMockImplementation()!;invoke.mockImplementation(async(cmd,args)=>{
  if(cmd==='start_dnd_classification'||cmd==='start_dnd_editions'){await original('start_classification',args);state.analysis.algorithm=cmd==='start_dnd_editions'?'dnd-editions-1':'dnd-compatibility-1';return;}
  return original(cmd,args);
 });
 showAnalysis();const select=screen.getByRole('combobox',{name:'Analysis type'});
 await user.selectOptions(select,'dnd');expect(screen.getByText(/OSR, retroclones, Pathfinder/)).toBeTruthy();await user.click(screen.getByRole('button',{name:'Classify books'}));await screen.findByRole('button',{name:'Cancel analysis'});expect(invoke.mock.calls.find(c=>c[0]==='start_dnd_classification')![1]).toEqual({reclassify:false});
 await user.click(screen.getByRole('button',{name:'Cancel analysis'}));await waitFor(()=>expect(screen.queryByRole('button',{name:'Cancel analysis'})).toBeNull());
 await user.selectOptions(select,'dnd_editions');expect(screen.getByText(/Nine independent checks/)).toBeTruthy();await user.click(screen.getByRole('button',{name:'Classify books'}));await screen.findByRole('button',{name:'Cancel analysis'});expect(invoke.mock.calls.find(c=>c[0]==='start_dnd_editions')![1]).toEqual({reclassify:false});

});

it('selects both edition analyses with metadata scopes, overlapping totals and same-position cancellation',async()=>{
 const {invoke,state}=setup();const user=userEvent.setup();await waitFor(()=>expect(invoke).toHaveBeenCalledWith('snapshot'));
 const original=invoke.getMockImplementation()!;
 invoke.mockImplementation(async(cmd,args)=>{
  if(cmd==='start_earth_editions'){await original('start_classification',args);state.analysis.algorithm=`${args!.kind}-editions-1`;return;}
  return original(cmd,args);
 });
 showAnalysis();const select=screen.getByRole('combobox',{name:'Analysis type'});
 const button=screen.getByRole('button',{name:'Run analysis'});
 for(const [kind,done,total] of [['shadowrun',8,12],['coc',12,20]] as const){
  await user.selectOptions(select,`${kind}_editions`);await screen.findByText(`${done} of ${total} eligible books have current saved results`);
  expect(screen.getByText(/Edition counts overlap/)).toBeTruthy();
  expect(screen.getByText(/No previous analysis is required; no prior results are supplied/)).toBeTruthy();
  expect((screen.getByRole('checkbox',{name:/Reclassify previously analyzed books too/}) as HTMLInputElement).checked).toBe(false);
  if(kind==='shadowrun') {expect(document.querySelector('[data-coverage=anarchy_2]')!.textContent).toBe('4');expect(invoke.mock.calls.some(c=>c[0].startsWith('start_'))).toBe(false);}
  else expect(document.querySelector('[data-coverage="7e"]')!.textContent).toBe('8');
  await user.click(screen.getByRole('button',{name:'Classify books'}));await screen.findByRole('button',{name:'Cancel analysis'});
  expect(screen.getByRole('button',{name:'Cancel analysis'})).toBe(button);
  expect(invoke.mock.calls.filter(c=>c[0]==='start_earth_editions').at(-1)![1]).toEqual({kind,reclassify:false});
  await user.click(button);await screen.findByRole('button',{name:'Classify books'});
  await user.click(screen.getByRole('checkbox',{name:/Reclassify previously analyzed books too/}));
 }
 await user.selectOptions(select,'shadowrun_editions');await user.click(button);await screen.findByRole('button',{name:'Cancel analysis'});
 expect(invoke.mock.calls.filter(c=>c[0]==='start_earth_editions').at(-1)![1]).toEqual({kind:'shadowrun',reclassify:true});
 expect(screen.getAllByRole('progressbar')).toHaveLength(1);
 expect(invoke.mock.calls.some(c=>['start_dnd_classification','start_earth_extraction','test_decisions'].includes(c[0]))).toBe(false);
});

it('shows model escalation and exhausted-book feedback inside the single totals block',()=>{
 const root=document.getElementById('app')!;const invoke=vi.fn(async()=>overviewFixture()) as Invoke;
 const view=createAnalysis(root,invoke);dispose=()=>view.dispose();
 const state=structuredClone(fixture);state.analysis={...idleAnalysis(),algorithm:'shadowrun-date-1',state:'running',overview:overviewFixture(),classification:{selected:12,already_classified:0,reclassify:false,requests:2,input_tokens:1600,cached_tokens:1200,cache_write_tokens:0,output_tokens:20,core_rulebook:0,supplement:0,adventure:0,none:0,model_escalations:1,exhausted_responses:0,last_escalation:'Book #42: invalid output from gpt-6-luna; escalating to gpt-6.1-terra.'}};
 view.setSnapshot(state);view.setVisible(true);
 expect(screen.getByText(/Most recent escalation: Book #42/)).toBeTruthy();expect(screen.getByRole('button',{name:'Cancel analysis'})).toBeTruthy();expect(screen.getAllByRole('progressbar')).toHaveLength(1);
 state.analysis.state='completed';state.analysis.classification!.exhausted_responses=1;view.setSnapshot(structuredClone(state));
 expect(screen.getByText(/1 books remained unsuccessful after Astra/)).toBeTruthy();expect(screen.getByRole('button',{name:'Analyze books'})).toBeTruthy();expect(screen.queryByText(/requests dispatched/)).toBeNull();
});
