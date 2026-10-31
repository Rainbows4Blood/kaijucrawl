import { analysisActive, type Overview } from './analysis';
import type { Invoke, Snapshot } from './types';
import './discovery.css';
import { workspaces, discoveryComplete, workspaceIcon, type AnalysisWorkspace } from './workspaces';

const number = (value: number) => new Intl.NumberFormat().format(value);

export function createDiscovery(root: HTMLElement, invoke: Invoke, notify: (state: Snapshot) => void) {
  return createAnalysisDashboard(root, invoke, notify, 'discovery');
}
export function createAnalysisDashboard(root: HTMLElement, invoke: Invoke, notify: (state: Snapshot) => void, workspace: AnalysisWorkspace, openDiscovery?: () => void) {
  const config = workspaces[workspace], stages = config.stages, system = workspace !== 'discovery';
  let snapshot: Snapshot | null = null, overview: Overview | null = null;
  let visible = false, disposed = false, pending = false, cancelling = false;
  let loading = false, queued = false, epoch = 0, totalsError = '';
  root.innerHTML = `<header class="discovery-heading"><p class="eyebrow">CATALOGUE / ANALYSIS</p><div class="workspace-emblem">${workspaceIcon(workspace)}</div><h1>${config.heading}</h1><p class="subtitle">${config.intro}</p></header>
    <section class="panel discovery-summary" aria-label="${config.name} analysis">
      <div class="discovery-summary-top"><div><h2>${system ? config.name + " collection" : "Your catalogue at a glance"}</h2><p>Current saved results across all runs.</p></div><button id="discovery-run" class="primary">Run analyses</button></div>
      <div class="discovery-stats"><div><strong id="discovery-books">—</strong><span>${config.countLabel}</span></div><div><strong id="discovery-ready">—</strong><span>${stages.length === 1 ? "analysis" : "analyses"} up to date</span></div><div><strong id="discovery-saved">—</strong><span>saved analysis results</span></div></div>
      <div class="discovery-controls"><label class="check"><input type="checkbox" id="discovery-rerun"/> Rerun all analyses, including current results</label><button id="discovery-refresh" class="text-button">Refresh totals</button></div>
      <p class="discovery-cost">${system ? "These analyses use OpenAI credits." : "Book classifications and player counts use OpenAI credits."} Current results, including ${workspace === "osr" ? "Not OSR, unknown and level-free results" : "unknowns"}, are kept by default. Rerunning includes them again and uses additional credits.</p>
      <p id="discovery-gate" class="muted-text"></p>
      <div id="discovery-sequence" class="discovery-sequence" hidden><p id="discovery-phase" role="status"></p><div class="discovery-meter" role="progressbar" aria-label="Analysis sequence progress" aria-valuemin="0" aria-valuemax="100"><span></span></div><p id="discovery-step-detail"></p></div>
      <p id="discovery-status" class="discovery-status" role="status" hidden></p>
      <p id="discovery-error" class="form-error" role="alert"></p>
      <p id="discovery-totals-error" class="form-error" role="alert"></p>
    </section>
    <section id="discovery-prerequisite" class="panel prerequisite-panel" aria-label="Discovery prerequisite" hidden><h2>Complete Discovery first</h2><p>All four Discovery analyses must have current saved results before system analyses can run. Unknown results count as complete.</p><ul>${workspaces.discovery.stages.map((stage,index)=>`<li><span>${stage.name}</span><strong id="discovery-prerequisite-${index}">Checking…</strong></li>`).join('')}</ul><button id="discovery-open-prerequisite" class="secondary">Open Discovery</button></section>
    <section class="discovery-grid" aria-label="Analysis coverage">${stages.map((stage,index) => `
      <article class="panel discovery-card" id="discovery-card-${index}">
        <div class="discovery-card-top"><span class="discovery-step" aria-hidden="true">0${index+1}</span><span class="eyebrow">${stage.tag}</span><span id="discovery-card-state-${index}" class="discovery-card-state">Loading</span></div>
        <h2>${stage.name}</h2><p class="discovery-description">${stage.description}</p><p class="discovery-scope">${stage.scope}</p>
        <div class="discovery-meter" id="discovery-meter-${index}" role="progressbar" aria-label="${stage.name} coverage" aria-valuemin="0" aria-valuemax="100" hidden><span></span></div>
        <p id="discovery-count-${index}" class="discovery-count">Loading saved totals…</p>
        <div class="discovery-categories">${Object.entries(stage.categories).map(([key,label]) => `<span>${label}<b id="discovery-${index}-${key}">—</b></span>`).join('')}</div>
        <p id="discovery-note-${index}" class="discovery-note"></p>
      </article>`).join('')}</section>
    <p class="page-note">The sequence runs in the order shown. ${config.note} Cancel keeps completed work; run again to continue missing results.</p>`.replaceAll('id="discovery-', `id="${workspace}-`);
  const el = <T extends HTMLElement = HTMLElement>(id: string) => root.querySelector<T>(`#${id.replace(/^discovery-/, workspace + '-')}`)!;
  const set = (id: string, value: string) => { el(id).textContent = value; };
  function meter(element: HTMLElement, fraction: number, label: string) {
    const percent = Math.max(0, Math.min(100, fraction * 100));
    element.setAttribute('aria-valuenow', String(Math.round(percent)));
    element.setAttribute('aria-valuetext', label);
    element.querySelector<HTMLElement>('span')!.style.width = `${percent}%`;
  }
  function update() {
    const s = snapshot, a = s?.analysis, sequence = a?.discovery;
    const pipeline = sequence && (sequence.workspace ?? 'discovery') === workspace ? sequence : null;
    const active = Boolean(a && analysisActive(a)), running = active && Boolean(pipeline);
    const unlocked = !system || (!totalsError && discoveryComplete(overview));
    const action = el<HTMLButtonElement>('discovery-run');
    action.textContent = running ? 'Cancel analysis' : pending ? 'Starting…' : 'Run analyses';
    action.disabled = running ? cancelling || a?.state === 'stopping' || Boolean(s?.closing) : pending || !s?.storage_ok || s.busy || s.closing || s.active || active || !unlocked;
    el<HTMLInputElement>('discovery-rerun').disabled = pending || active || Boolean(s?.closing);
    el<HTMLButtonElement>('discovery-refresh').disabled = loading || !s?.storage_ok || Boolean(s?.closing);
    set('discovery-gate', !s?.storage_ok ? 'Available once local storage is ready.' : s.closing ? 'Closing safely…' : s.active ? 'Stop the crawler before running analyses.' : active && !pipeline ? `${sequence ? workspaces[sequence.workspace ?? 'discovery'].name : 'Another'} analysis is running.` : s.busy ? 'Waiting for the current operation.' : !unlocked ? 'Complete Discovery to unlock this system’s analyses.' : '');
    const valid = overview !== null && !totalsError;
    set('discovery-books', valid ? number(config.countAlgorithm ? overview!.analyses[config.countAlgorithm]?.eligible ?? 0 : overview!.catalogue_total) : '—');
    let saved = 0, ready = 0;
    stages.forEach((stage,index) => {
      const totals = valid ? overview!.analyses[stage.algorithm] : undefined;
      const complete = Boolean(totals && totals.completed >= totals.eligible && totals.prerequisite_pending === 0);
      if (totals) saved += totals.completed;
      if (complete) ready++;
      const current = running && pipeline!.step === index && pipeline!.completed_steps <= index;
      el(`discovery-card-${index}`).classList.toggle('is-active', current);
      set(`discovery-card-state-${index}`, current ? a?.state === 'stopping' ? 'Stopping' : 'Analyzing' : running && pipeline!.completed_steps > index ? 'Checked' : complete ? 'Up to date' : totals ? 'Remaining' : 'Unavailable');
      const bar = el(`discovery-meter-${index}`); bar.hidden = !totals;
      if (totals) meter(bar, totals.eligible ? totals.completed / totals.eligible : complete ? 1 : 0, `${number(totals.completed)} of ${number(totals.eligible)} eligible books`);
      set(`discovery-count-${index}`, totals ? `${number(totals.completed)} of ${number(totals.eligible)} eligible books analyzed` : totalsError ? 'Saved totals unavailable' : 'Loading saved totals…');
      for (const key of Object.keys(stage.categories)) set(`discovery-${index}-${key}`, totals ? number(totals.counts[key] ?? 0) : '—');
      set(`discovery-note-${index}`, totals?.prerequisite_pending ? `${number(totals.prerequisite_pending)} books still need ${stage.prerequisite ?? "a prerequisite classification"} before eligibility is known.` : stage.algorithm === 'osr-levels-1' ? 'Ranges stay separate by system. Status counts may overlap; unknown and level-free results count as analyzed.' : workspace === 'osr' ? 'Not OSR is a completed result. Multiple system memberships count as one analyzed book.' : 'Unknown results count as analyzed.');
    });
    set('discovery-ready', valid ? `${ready} / ${stages.length}` : '—');
    set('discovery-saved', valid ? number(saved) : '—');
    el('discovery-sequence').hidden = !running;
    let status = '';
    if (running && a && pipeline) {
      const step = stages[pipeline.step], checked = pipeline.completed_steps > pipeline.step;
      set('discovery-phase', a.state === 'stopping' ? 'Stopping safely…' : checked ? pipeline.completed_steps < stages.length ? 'Preparing the next analysis…' : 'Finishing the sequence…' : `Step ${pipeline.step+1} of ${stages.length} · ${step?.name ?? 'Analysis'}`);
      const fraction = checked ? pipeline.completed_steps / stages.length : (pipeline.completed_steps + (a.total ? a.processed / a.total : 0)) / stages.length;
      meter(el('discovery-sequence').querySelector<HTMLElement>('[role=progressbar]')!, fraction, `${pipeline.completed_steps} of ${stages.length} steps finished`);
      set('discovery-step-detail', a.state === 'preparing' ? 'Checking current results…' : `${number(a.processed)} of ${number(a.total)} selected books processed in this step${pipeline.rerun_all ? ' · rerunning saved results' : ''}.`);
      const p = a.classification;
      if (p && ((p.retrying ?? 0) > 0 || (p.retry_in_secs ?? 0) > 0)) status = `${p.retry_status ? `HTTP ${p.retry_status} · ` : ''}${p.retry_reason ?? 'API request failed'} · retrying automatically in ${number(p.retry_in_secs ?? 0)}s.`;
      if (p?.last_escalation) status += `${status ? ' ' : ''}Most recent escalation: ${p.last_escalation}`;
    } else if (pipeline && a) {
      status = a.state === 'completed' ? stages.length === 1 ? 'Sequence finished. The analysis has been checked.' : `Sequence finished. All ${stages.length === 4 ? 'four' : stages.length} analyses have been checked.` : a.detail;
      const failures = pipeline.unsuccessful + (pipeline.completed_steps <= pipeline.step && (system || pipeline.step > 0) ? a.skipped : 0);
      if (failures) status += ` ${number(failures)} unsuccessful results; previous good results were kept. Run again to retry missing results.`;
    }
    el('discovery-prerequisite').hidden = !system || unlocked || running;
    if (system) workspaces.discovery.stages.forEach((stage,index)=>{
      const c = !totalsError ? overview?.analyses[stage.algorithm] : undefined;
      set(`discovery-prerequisite-${index}`, c ? c.completed >= c.eligible && c.prerequisite_pending === 0 ? 'Complete' : `${number(Math.max(0,c.eligible-c.completed))} remaining${c.prerequisite_pending ? ' · eligibility pending' : ''}` : 'Checking…');
    });
    set('discovery-status', status); el('discovery-status').hidden = !status;
    set('discovery-totals-error', totalsError ? `Totals could not be refreshed: ${totalsError}` : '');
  }
  function useLiveTotals() {
    const a = snapshot?.analysis;
    if (!a?.overview || !analysisActive(a)) return false;
    epoch++; overview = a.overview; totalsError = ''; return true;
  }
  async function refreshTotals(force = false) {
    if (disposed || !visible || !snapshot?.storage_ok || snapshot.closing) return;
    if (useLiveTotals()) { update(); return; }
    if (!force && totalsError) return;
    if (loading) { queued = true; return; }
    loading = true; const startEpoch = epoch; update();
    try { const totals = await invoke<Overview>('analysis_overview'); if (!disposed && startEpoch === epoch) { overview = totals; totalsError = ''; } }
    catch (error) { if (!disposed && startEpoch === epoch) { totalsError = String(error); queued = false; } }
    finally { loading = false; if (!disposed) { update(); if (queued) { queued = false; void refreshTotals(); } } }
  }
  function setSnapshot(s: Snapshot) {
    const previous = snapshot;
    const finished = Boolean(previous && analysisActive(previous.analysis) && !analysisActive(s.analysis));
    const changed = !previous || previous.storage_ok !== s.storage_ok || previous.metrics.successful !== s.metrics.successful || previous.reprocessed !== s.reprocessed || previous.active !== s.active;
    snapshot = s;
    if (s.analysis.overview && (analysisActive(s.analysis) || finished)) { epoch++; overview = s.analysis.overview; totalsError = ''; }
    update();
    if (!analysisActive(s.analysis) && (finished || changed || !overview)) void refreshTotals(finished);
  }
  el('discovery-open-prerequisite').onclick = () => openDiscovery?.();
  el('discovery-refresh').onclick = () => { void refreshTotals(true); };
  el('discovery-run').onclick = async () => {
    if (disposed || !snapshot || pending || cancelling) return;
    const active = analysisActive(snapshot.analysis);
    if (active && (!snapshot.analysis.discovery || (snapshot.analysis.discovery.workspace ?? 'discovery') !== workspace)) return;
    if (!active && system && (totalsError || !discoveryComplete(overview))) return;
    if (active) cancelling = true; else pending = true;
    set('discovery-error', ''); update();
    try {
      if (active) await invoke('cancel_analysis');
      else await invoke(system ? 'start_system_analysis' : 'start_discovery_analysis', { ...(system ? { workspace } : {}), rerunAll: el<HTMLInputElement>('discovery-rerun').checked });
      const latest = await invoke<Snapshot>('snapshot');
      if (!disposed) { setSnapshot(latest); notify(latest); }
    } catch (error) { if (!disposed) set('discovery-error', String(error)); }
    finally { pending = false; cancelling = false; if (!disposed) update(); }
  };
  update();
  return { setSnapshot, setVisible(value: boolean) { visible = value; if (value) void refreshTotals(true); }, dispose() { disposed = true; queued = false; root.innerHTML = ''; } };
}
