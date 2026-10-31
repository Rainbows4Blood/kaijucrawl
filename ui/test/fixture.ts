import { gameLabels, type Game } from '../books';
import type { Invoke, Snapshot } from '../types';
import { idleAnalysis, type Overview } from '../analysis';
import { discoveryComplete, workspaces, type AnalysisWorkspace } from '../workspaces';
export const fixture: Snapshot = {
  analysis: idleAnalysis(),
  state: 'idle', detail: 'Ready to continue your catalogue',
  metrics: { successful: 12840, last_saved: '2026-10-27T12:30:00Z', saved_run: 832, pending: 246000,
    active: 12, persistence: 4, retry: 3, failed: 7, sitemaps_pending: 1, sitemaps_active: 0,
    sitemaps_retry: 0, sitemaps_done: 14, sitemaps_failed: 0, seen: 18000, discovered: 420, next_due: null },
  events: ['14:30:00 Saved 8 products with verified archives', '14:29:59 Saved 5 products with verified archives', '14:29:58 Discovered 124 product IDs', '14:29:57 Saved 12 products with verified archives'],
  active: false, busy: false, closing: false, storage_ok: true, selected: true, elapsed_secs: 600,
  storage_checking: false, catalogue_loaded: true,
  downloads: 12, parsers: 2, workers: 16, website_delay_ms: 5000, api_delay_ms: 0, cooldown_secs: 0,
  reprocessed: 0, reprocess_failed: 0, outdated: 0, can_resume: false,
  auth_method: 'browser', browser_name: 'Firefox · Synthetic profile · container 0', has_manual_cookies: false,
  remembered: false, browser_container: 0, root: 'C:\\Users\\Example\\.kaijucrawl',
  version: '0.1.0', catalogue_version: 16, record_version: 2, parser_version: 'drivethru-rust-1',
};
const previewAlgorithms:Record<string,string>={start_analysis:'weighted-rating-1',start_classification:'primary-book-role-1',start_player_analysis:'adventure-players-1',start_solo_classification:'solo-play-1',start_dnd_classification:'dnd-compatibility-1',start_dnd_editions:'dnd-editions-1',start_dnd_settings:'dnd-settings-1',osr:'osr-systems-1',osr_levels:'osr-levels-1',levels:'dnd-levels-1',locations:'dnd-locations-1',date:'dnd-date-1',shadowrun_location:'shadowrun-location-1',shadowrun_date:'shadowrun-date-1',coc_location:'coc-location-1',coc_date:'coc-date-1',shadowrun_editions:'shadowrun-editions-1',coc_editions:'coc-editions-1'};
const previewOverview:Overview={catalogue_total:fixture.metrics.successful,analyses:Object.fromEntries(Object.values(workspaces).flatMap(w=>w.stages.map(s=>s.algorithm)).map(algorithm=>{
  const dependent=(algorithm.startsWith('warhammer-')&&!algorithm.endsWith('-games-1'))||['osr-levels-1','adventure-players-1','dnd-editions-1','dnd-settings-1','dnd-levels-1','dnd-locations-1','dnd-date-1'].includes(algorithm);
  return [algorithm,{eligible:dependent?0:algorithm.startsWith('warhammer-')?30:algorithm==='osr-systems-1'?125:algorithm.startsWith('shadowrun-')?20:algorithm.startsWith('coc-')?40:fixture.metrics.successful,completed:0,prerequisite_pending:dependent?(algorithm.startsWith('warhammer-')?30:fixture.metrics.successful):0,counts:{}}];
}))};
export const previewInvoke: Invoke = async <T>(command: string, args?: Record<string, unknown>) => {
  if(command==='warhammer_books'||command==='warhammer_book'){
    const game=(command==='warhammer_books'?(args?.query as {game:Game}).game:args?.game) as Game;
    const book={target:'drivethru',product_id:'42',name:`Fictional ${gameLabels[game]} chronicle`,release_date:'2024-03-10',book_type:'Adventure',in_universe_date:'late M41',locations:['Fictional Sector'],date_analyzed:true,locations_analyzed:true};
    if(command==='warhammer_books')return {books:[book],total:1,next:null} as T;
    return {book,description:'Synthetic book detail preview. No catalogue data or network requests are used.',description_truncated:false,publisher:'Example Press',authors:['Example Author'],page_count:32,rule_systems:['Warhammer 40k Roleplay'],source_product_types:['Adventures'],games:[game],captured_at:'2026-01-01'} as T;
  }
  if(command==='test_decisions')return {model:'gpt-6-luna',answers:[{type:'predicate'},{type:'choice'},{type:'score'}],usage:{input_tokens:180,input_tokens_details:{cached_tokens:0,cache_write_tokens:0},output_tokens:0}} as T;
  if(command==='analysis_overview')return structuredClone(previewOverview) as T;
  const selected=command==='start_earth_editions'?`${String(args?.kind)}-editions-1`:(command==='start_dnd_extraction'||command==='start_earth_extraction')?previewAlgorithms[String(args?.kind)]:previewAlgorithms[command];
  if(command.startsWith('start_')&&selected){
    const algorithm=selected,coverage=previewOverview.analyses[algorithm];
    coverage.completed=coverage.eligible;
    coverage.counts=algorithm==='weighted-rating-1'?{scored:coverage.eligible}:algorithm==='primary-book-role-1'?{adventure:coverage.eligible}:algorithm==='solo-play-1'?{neither:coverage.eligible}:algorithm==='dnd-compatibility-1'?{dnd_only:coverage.eligible}:algorithm==='osr-systems-1'?{not_osr:coverage.eligible}:{unknown:coverage.eligible};
    if(algorithm==='primary-book-role-1')Object.assign(previewOverview.analyses['adventure-players-1'],{eligible:coverage.eligible,prerequisite_pending:0});
    if(algorithm==='dnd-compatibility-1')for(const a of ['dnd-editions-1','dnd-settings-1','dnd-levels-1','dnd-locations-1','dnd-date-1'])Object.assign(previewOverview.analyses[a],{eligible:coverage.eligible,prerequisite_pending:0});
    fixture.analysis={...idleAnalysis(),id:1,algorithm,state:'completed',total:coverage.eligible,processed:coverage.eligible,scored:coverage.eligible,overview:structuredClone(previewOverview)};
  }
  if(command==='start_discovery_analysis'||command==='start_system_analysis'){
    const workspace:AnalysisWorkspace=command==='start_discovery_analysis'?'discovery':args?.workspace as AnalysisWorkspace;
    if(workspace!=='discovery'&&!discoveryComplete(previewOverview))throw new Error('Complete Discovery first');
    const algorithms=workspaces[workspace].stages.map(s=>s.algorithm);
    for(const algorithm of algorithms){
      const coverage=previewOverview.analyses[algorithm];
      if(algorithm==='osr-levels-1'){coverage.eligible=0;coverage.prerequisite_pending=0;}
      if(algorithm==='adventure-players-1'){coverage.eligible=previewOverview.catalogue_total;coverage.prerequisite_pending=0;}
      coverage.completed=coverage.eligible;
      coverage.counts=algorithm==='weighted-rating-1'?{scored:coverage.eligible}:algorithm==='primary-book-role-1'?{adventure:coverage.eligible}:algorithm==='solo-play-1'?{neither:coverage.eligible}:algorithm==='dnd-compatibility-1'?{dnd_only:coverage.eligible}:algorithm==='osr-systems-1'?{not_osr:coverage.eligible}:{unknown:coverage.eligible};
      if(algorithm.startsWith('warhammer-')&&algorithm.endsWith('-games-1'))for(const stage of workspaces[workspace].stages.slice(1))Object.assign(previewOverview.analyses[stage.algorithm],{eligible:coverage.eligible,prerequisite_pending:0});
      if(algorithm==='dnd-compatibility-1')for(const stage of workspaces.dnd.stages.slice(1))Object.assign(previewOverview.analyses[stage.algorithm],{eligible:coverage.eligible,prerequisite_pending:0});
    }
    fixture.analysis={...idleAnalysis(),id:1,algorithm:algorithms.at(-1)!,state:'completed',overview:structuredClone(previewOverview),discovery:{workspace,step:algorithms.length-1,completed_steps:algorithms.length,rerun_all:Boolean(args?.rerunAll),unsuccessful:0}};
  }
  if(command==='cancel_analysis')fixture.analysis.state='cancelled';
  if (command === 'snapshot') return structuredClone(fixture) as T;
  if (command === 'browser_profiles') return [{ id: 0, browser: 'Firefox', name: 'Synthetic profile', containers: [[0, 'Default'], [2, 'Personal']] }] as T;
  if (command === 'failure_page') return { rows: ['#17 Product 42 · Restricted (HTTP 403) · synthetic fixture'], next: null } as T;
  if (command === 'test_access') return { passed: true, text: 'Synthetic preview only.\nCookies loaded: yes\nAuthenticated access: unverified\nWebsite robots: HTTP 200\nAPI robots: HTTP 200\nProduct response: identity and completeness verified' } as T;
  if (command === 'select_target') fixture.selected = Boolean(args?.selected);
  if (command === 'start') { fixture.active = true; fixture.state = 'crawling'; }
  if (command === 'stop') { fixture.active = false; fixture.state = 'stopped'; }
  return undefined as T;
};
