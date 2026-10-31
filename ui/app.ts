import type { AccessResult, Draft, FailurePage, Invoke, Profile, Snapshot } from './types';
import { analysisActive } from './analysis';
import { createAnalysisDashboard } from './discovery';
import { analysisWorkspaces, workspaces, workspaceIcon, type AnalysisWorkspace } from './workspaces';
import './themes.css';
import { createBookLibrary, gameLabels, type Game } from './books';

const escape = (value: unknown) => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
const number = (value: number) => new Intl.NumberFormat().format(value);
const date = (value: string | null) => value ? new Date(value).toLocaleString() : 'Never';
const duration = (secs: number) => secs < 60 ? `${secs}s` : secs < 3600 ? `${Math.floor(secs / 60)}m ${secs % 60}s` : `${Math.floor(secs / 3600)}h ${Math.floor(secs % 3600 / 60)}m`;
const delay = (ms: number | null) => ms === null ? 'Not loaded' : ms === 0 ? 'No delay' : `${number(ms)} ms · robots`;
const icons: Record<string, string> = { dashboard: '◫', auth: '◇', failures: '≡', storage: '▤', about: 'ⓘ' };
const crawlerTabs = [
  ['dashboard', 'Overview'], ['auth', 'Authentication'], ['failures', 'Unsuccessful jobs'],
  ['storage', 'Local storage'], ['about', 'About'],
] as const;
type View = 'dashboard' | 'auth' | 'access' | 'failures' | 'storage' | 'about';
type Step = 'method' | 'credentials' | 'profile' | 'retention' | 'review';

export function createApp(root: HTMLElement, invoke: Invoke, pollMs = 1000) {
  let state: Snapshot | null = null;
  let view: View = 'dashboard';
  let workspace: 'crawlers' | AnalysisWorkspace = 'crawlers';
  let warhammerView: 'analysis'|Game = 'analysis';
  let operation = false;
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let step: Step = 'method';
  let profiles: Profile[] = [];
  let access: AccessResult | null = null;
  let failureNext: number | null = null;
  let failureAfter = 0;
  let draft: Draft = { method: 'anonymous', profile_id: null, container: 0, website_cookie: '', api_cookie: '', keep_saved: false, remember: false, product_id: null };
  root.dataset.theme = 'default';
  root.innerHTML = `<div class="shell">
    <div class="app-frame">
      <header class="app-header">
        <div class="app-identity">
          <button id="menu-toggle" class="icon-button" aria-label="Open main menu" aria-haspopup="dialog" aria-expanded="false" aria-controls="main-menu"><span aria-hidden="true">☰</span></button>
          <div class="brand"><span class="mark" aria-hidden="true">K<span>◆</span></span><span>KaijuCrawl</span></div>
        </div>
        <div class="app-status"><div class="storage-dot"><i aria-hidden="true"></i><span id="storage-label">Opening storage</span></div><button class="quiet quit" id="quit">Close application <span aria-hidden="true">↗</span></button></div>
      </header>
      <main>
        <section id="storage-progress" class="startup-progress" role="status" hidden><strong id="storage-progress-title"></strong><p id="storage-progress-detail"></p></section>
        <div id="banner" role="alert" hidden></div>
        <section id="crawler-workspace">
        <div class="workspace-heading"><div><p class="eyebrow">WORKSPACE</p><p class="workspace-title">Crawlers</p></div><span id="crawler-state" class="status-pill">Idle</span></div>
        <div class="crawler-tabs" role="tablist" aria-label="Crawler views">${crawlerTabs.map(([key,label]) => `<button id="${key}-tab" role="tab" data-view="${key}" aria-selected="${key==='dashboard'}" aria-controls="content" tabindex="${key==='dashboard'?0:-1}"><span aria-hidden="true">${icons[key]}</span>${label}</button>`).join('')}</div>
        <div id="content" role="tabpanel" aria-labelledby="dashboard-tab" tabindex="0"></div>
        </section>
        ${analysisWorkspaces.map(key=>`<section id="${key}-workspace" hidden>
          <div class="workspace-heading"><div><p class="eyebrow">WORKSPACE</p><p class="workspace-title">${escape(workspaces[key].name)}</p></div><span id="${key}-workspace-state" class="status-pill">Idle</span></div>
          <div class="crawler-tabs ${key==='warhammer_40k'?'game-tabs':''}" role="tablist" aria-label="${escape(workspaces[key].name)} views"><button id="${key}-analysis-tab" role="tab" aria-selected="true" aria-controls="${key}-content" tabindex="0"><span aria-hidden="true">◈</span>Analysis</button>${key==='warhammer_40k'?Object.entries(gameLabels).map(([game,label])=>`<button id="warhammer_40k-${game}-tab" role="tab" aria-selected="false" aria-controls="warhammer_40k-library" tabindex="-1">${escape(label)}</button>`).join(''):''}</div>
          <div id="${key}-content" role="tabpanel" aria-labelledby="${key}-analysis-tab" tabindex="0"></div>${key==='warhammer_40k'?'<div id="warhammer_40k-library" role="tabpanel" tabindex="0" hidden></div>':''}
        </section>`).join('')}
      </main>
    </div>
    <div id="menu-backdrop" class="menu-backdrop" aria-hidden="true" hidden></div>
    <aside id="main-menu" class="main-menu" role="dialog" aria-modal="true" aria-labelledby="menu-title" hidden>
      <div class="menu-heading"><h2 id="menu-title">Main menu</h2><button id="menu-close" class="icon-button" aria-label="Close main menu"><span aria-hidden="true">×</span></button></div>
      <nav aria-label="Main navigation"><button id="menu-crawlers" aria-current="page"><span class="menu-icon" aria-hidden="true">◫</span><span>Crawlers<small>Collect and maintain your catalogue</small></span><span class="menu-arrow" aria-hidden="true">→</span></button>${analysisWorkspaces.map(key=>`<button id="menu-${key}"><span class="menu-icon">${workspaceIcon(key)}</span><span class="menu-copy">${escape(workspaces[key].name)}<small>${escape(workspaces[key].subtitle)}</small><small id="menu-state-${key}" class="menu-run-state"></small></span><span class="menu-arrow" aria-hidden="true">→</span></button>`).join('')}</nav>
      <p class="menu-note">KaijuCrawl<span>Your catalogue, on this computer.</span></p>
    </aside>
  </div>`;
  const content = root.querySelector<HTMLElement>('#content')!;
  const banner = root.querySelector<HTMLElement>('#banner')!;
  const el = <T extends HTMLElement = HTMLElement>(id: string) => root.querySelector<T>(`#${id}`)!;
  const text = (id: string, value: unknown) => { const element = root.querySelector(`#${id}`); if (element) element.textContent = String(value); };
  const button = (id: string, handler: () => void) => { const b = root.querySelector<HTMLButtonElement>(`#${id}`); if (b) b.onclick = handler; };
  const frame = root.querySelector<HTMLElement>('.app-frame')!;
  const menu = el('main-menu');
  const menuToggle = el<HTMLButtonElement>('menu-toggle');
  const dashboards = Object.fromEntries(analysisWorkspaces.map(key=>[key, createAnalysisDashboard(el(`${key}-content`), invoke, next=>{state=next;update();}, key, ()=>selectWorkspace('discovery'))])) as Record<AnalysisWorkspace, ReturnType<typeof createAnalysisDashboard>>;
  const books=createBookLibrary(el('warhammer_40k-library'),invoke);
  function setMenu(open: boolean, returnFocus = true) {
    menu.hidden = !open;
    el('menu-backdrop').hidden = !open;
    frame.inert = open;
    root.classList.toggle('menu-open', open);
    menuToggle.setAttribute('aria-expanded', String(open));
    if (open) el('menu-close').focus();
    else if (returnFocus) menuToggle.focus({ preventScroll: true });
  }
  button('menu-toggle', () => setMenu(true));
  button('menu-close', () => setMenu(false));
  el('menu-backdrop').onclick = () => setMenu(false);
  function selectWorkspace(next: typeof workspace) {
    workspace = next;
    el('crawler-workspace').hidden = next !== 'crawlers';
    for (const key of analysisWorkspaces) el(`${key}-workspace`).hidden = next !== key;
    root.dataset.theme = next === 'crawlers' || next === 'discovery' ? 'default' : next;
    for (const key of ['crawlers', ...analysisWorkspaces]) {
      if (key === next) el(`menu-${key}`).setAttribute('aria-current', 'page');
      else el(`menu-${key}`).removeAttribute('aria-current');
    }
    for (const key of analysisWorkspaces) dashboards[key].setVisible(next === key && (key!=='warhammer_40k'||warhammerView==='analysis'));
    books.show(next==='warhammer_40k'&&warhammerView!=='analysis'?warhammerView:null);
    setMenu(false, false);
    el(next !== 'crawlers' ? (next==='warhammer_40k'?`warhammer_40k-${warhammerView}-tab`:`${next}-analysis-tab`) : `${view === 'access' ? 'auth' : view}-tab`).focus({ preventScroll: true });
  }
  button('menu-crawlers', () => selectWorkspace('crawlers'));
  for (const key of analysisWorkspaces) {
    button(`menu-${key}`, ()=>selectWorkspace(key));
    el(`${key}-analysis-tab`).onkeydown = e=>{if (['ArrowLeft','ArrowRight','Home','End'].includes(e.key)) e.preventDefault();};
  }
  const fortyKTabs=['analysis',...Object.keys(gameLabels)] as ('analysis'|Game)[];
  function select40k(next:'analysis'|Game) {
    warhammerView=next;
    for(const key of fortyKTabs){const tab=el<HTMLButtonElement>(`warhammer_40k-${key}-tab`);tab.setAttribute('aria-selected',String(key===next));tab.tabIndex=key===next?0:-1;}
    el('warhammer_40k-content').hidden=next!=='analysis';el('warhammer_40k-library').hidden=next==='analysis';
    el('warhammer_40k-library').setAttribute('aria-labelledby',`warhammer_40k-${next}-tab`);
    dashboards.warhammer_40k.setVisible(workspace==='warhammer_40k'&&next==='analysis');
    books.show(workspace==='warhammer_40k'&&next!=='analysis'?next:null);
  }
  for(const key of fortyKTabs){
    button(`warhammer_40k-${key}-tab`,()=>select40k(key));
    el(`warhammer_40k-${key}-tab`).onkeydown=e=>{
      if(!['ArrowLeft','ArrowRight','Home','End'].includes(e.key))return;
      e.preventDefault();const i=fortyKTabs.indexOf(key),n=e.key==='Home'?0:e.key==='End'?fortyKTabs.length-1:(i+(e.key==='ArrowRight'?1:-1)+fortyKTabs.length)%fortyKTabs.length;
      select40k(fortyKTabs[n]);el(`warhammer_40k-${fortyKTabs[n]}-tab`).focus();
    };
  }
  menu.onkeydown = e => {
    if (e.key === 'Escape') { e.preventDefault(); setMenu(false); }
    if (e.key === 'Tab') {
      const first = el('menu-close'), last = el(`menu-${analysisWorkspaces[analysisWorkspaces.length - 1]}`);
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
  };
  function selectTab() {
    const selected = view === 'access' ? 'auth' : view;
    for (const [key] of crawlerTabs) {
      const tab = el<HTMLButtonElement>(`${key}-tab`);
      tab.setAttribute('aria-selected', String(key === selected));
      tab.tabIndex = key === selected ? 0 : -1;
    }
    content.setAttribute('aria-labelledby', `${selected}-tab`);
  }
  for (const [key] of crawlerTabs) {
    button(`${key}-tab`, () => go(key, false));
    el(`${key}-tab`).onkeydown = e => {
      if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
      e.preventDefault();
      const index = crawlerTabs.findIndex(([value]) => value === key);
      const next = e.key === 'Home' ? 0 : e.key === 'End' ? crawlerTabs.length - 1 : (index + (e.key === 'ArrowRight' ? 1 : -1) + crawlerTabs.length) % crawlerTabs.length;
      const [target] = crawlerTabs[next];
      go(target, false);
      el(`${target}-tab`).focus();
    };
  }
  const heading = (eyebrow: string, title: string, subtitle: string) => `<header><p class="eyebrow">${eyebrow}</p><h1 tabindex="-1">${title}</h1><p class="subtitle">${subtitle}</p></header>`;
  function alert(error: unknown) { banner.textContent = String(error); banner.hidden = false; }
  async function refresh() {
    const first = state === null;
    state = await invoke<Snapshot>('snapshot');
    if (disposed) return;
    if (first) render();
    update();
  }
  async function perform(work: () => Promise<void>) {
    if (operation) return;
    operation = true; banner.hidden = true; update();
    try { await work(); } catch (error) { alert(error); }
    finally { operation = false; await refresh().catch(alert); update(); }
  }
  async function poll() {
    try { await refresh(); } catch { alert('Cannot reach the desktop backend. Launch KaijuCrawl to use this interface.'); }
    if (!disposed) timer = setTimeout(poll, pollMs);
  }
  function go(next: View, focusHeading = true) {
    if (view === next) return;
    if (view === 'auth' && next !== 'auth' && next !== 'access') { draft.website_cookie = ''; draft.api_cookie = ''; }
    view = next; banner.hidden = true;
    if (next === 'auth' && state) {
      draft = { method: state.auth_method, profile_id: null, container: state.browser_container, website_cookie: '', api_cookie: '', keep_saved: state.has_manual_cookies, remember: state.remembered, product_id: null };
      step = 'method';
    }
    selectTab(); render(); update();
    if (focusHeading) content.querySelector<HTMLElement>('h1')?.focus();
    if (next === 'failures') void loadFailures(0);
  }
  button('quit', () => { void invoke('quit').catch(alert); });
  function update() {
    if (!state) return;
    const s = state, m = s.metrics;
    const analyzing=analysisActive(s.analysis);
    for (const key of analysisWorkspaces) {
      dashboards[key].setSnapshot(s);
      const own = s.analysis.discovery && (s.analysis.discovery.workspace ?? 'discovery') === key;
      text(`${key}-workspace-state`, own ? s.analysis.state : 'Idle');
      text(`menu-state-${key}`, own && analyzing ? s.analysis.state : '');
    }
    text('crawler-state', s.state);
    text('storage-label', s.storage_checking ? 'Opening local storage' : s.storage_ok ? 'Local storage ready' : 'Storage needs attention');
    root.querySelector('.storage-dot')?.classList.toggle('ready', s.storage_ok);
    root.querySelector('.storage-dot')?.classList.toggle('checking', s.storage_checking);
    el('storage-progress').hidden=!s.storage_checking;
    text('storage-progress-title','Opening local storage…');
    text('storage-progress-detail',s.detail);
    content.querySelectorAll<HTMLButtonElement>('[data-operation]').forEach(b => b.disabled = operation || s.busy || s.closing);
    if (s.closing) { banner.textContent = 'Closing safely… finishing pending operations and releasing catalogue ownership.'; banner.hidden = false; }
    if (view === 'dashboard') {
      text('state', s.state); text('detail', s.detail); text('lifetime', s.catalogue_loaded ? number(m.successful) : '—');
      text('last-save', s.catalogue_loaded ? `Last save · ${date(m.last_saved)}` : 'Loading saved catalogue…'); text('saved', number(m.saved_run));
      text('elapsed', `Elapsed · ${duration(s.elapsed_secs)}`);
      text('rate', s.elapsed_secs ? (m.saved_run * 60 / s.elapsed_secs).toFixed(1) : '—');
      text('failed', number(m.failed));
      for (const key of ['pending', 'active', 'persistence', 'retry', 'discovered', 'seen'] as const) text(key, number(m[key]));
      text('website-delay', delay(s.website_delay_ms)); text('api-delay', delay(s.api_delay_ms));
      text('workers', `${s.workers} logical CPU workers`); text('cooldown', s.cooldown_secs ? duration(s.cooldown_secs) : 'None');
      text('downloads', `${s.downloads} downloads / pacing`); text('parsers', `${s.parsers} isolated parsers`);
      text('discovery', `${m.sitemaps_pending} queued · ${m.sitemaps_active} active · ${m.sitemaps_retry} retrying · ${m.sitemaps_done} processed · ${m.sitemaps_failed} failed`);
      text('next-due', m.next_due ? date(m.next_due) : 'None');
      text('reprocess', `${number(s.outdated)} outdated · ${number(s.reprocessed)} processed · ${number(s.reprocess_failed)} failed`);
      const target = el<HTMLInputElement>('target'); target.checked = s.selected; target.disabled = operation || s.busy || s.closing;
      const start = el<HTMLButtonElement>('start'); start.disabled = operation || s.busy || s.active || analyzing || !s.storage_ok || !s.selected || s.closing;
      text('start-hint', s.storage_checking ? 'Start becomes available when local preparation finishes.' : !s.storage_ok ? 'Check local storage to enable Start.' : !s.selected ? 'Select DriveThruRPG to enable Start.' : analyzing ? 'Finish or cancel analysis before starting a crawl.' : s.active ? 'Your current run is active.' : 'New runs recover unfinished work and skip successful products.');
      el<HTMLButtonElement>('stop').disabled = !s.active && (s.storage_checking || !s.busy);
      el<HTMLButtonElement>('resume').hidden = !s.state.includes('paused');
      el<HTMLButtonElement>('resume').disabled = operation || s.busy || !s.can_resume;
      text('pause-hint', s.state === 'authentication paused' && !s.can_resume ? 'Open Authentication, save credentials and pass Test Access before resuming.' : '');
      const events = el('events');
      const rows = s.events.slice().reverse();
      const signature = rows.join('\n');
      if (events.dataset.signature !== signature) {
        events.dataset.signature = signature;
        events.innerHTML = rows.length ? rows.map(e => `<li>${escape(e)}</li>`).join('') : '<li class="empty">Your next chapter starts here.<small>Events will appear when a crawl begins.</small></li>';
      }
    }
    if (view === 'storage') { text('storage-state', s.storage_checking ? 'Preparation in progress' : s.storage_ok ? 'Validated' : 'Needs attention'); text('storage-detail', s.detail); el<HTMLButtonElement>('retry-storage').disabled ||= s.active || analyzing; }
    if (view === 'about') el<HTMLButtonElement>('reprocess-button').disabled ||= !s.storage_ok || s.active || analyzing;
  }
  function render() {
    if (!state) { content.innerHTML = heading('WORKSPACE', 'Opening your catalogue…', 'Loading local settings and storage.'); return; }
    if (view === 'auth') { renderWizard(); return; }
    if (view === 'access') { renderAccess(); return; }
    if (view === 'dashboard') {
      content.innerHTML = heading('DRIVETHRURPG / OVERVIEW', 'Your catalogue, growing.', 'A permanent home for every first successful capture.') + `
      <section class="runbar"><div><span class="status-pill" id="state">Idle</span><p id="detail"></p></div><div class="actions"><button id="resume" class="secondary" hidden>Resume run</button><button id="stop" class="secondary">Stop</button><button id="start" class="primary"><span aria-hidden="true">▶</span> Start crawl</button></div></section>
      <div class="target-row"><label><input type="checkbox" id="target" /> DriveThruRPG</label><span id="start-hint"></span></div><p id="pause-hint" class="warning"></p>
      <div class="stats"><article class="stat hero-stat"><span>CATALOGUE</span><strong id="lifetime">—</strong><small id="last-save"></small></article><article class="stat"><span>SAVED THIS RUN</span><strong id="saved">—</strong><small id="elapsed"></small></article><article class="stat"><span>SAVED / MINUTE</span><strong id="rate">—</strong><small>Average for this run</small></article><article class="stat"><span>UNSUCCESSFUL</span><strong id="failed">—</strong><button class="text-button" id="inspect">Inspect jobs ↗</button></article></div>
      <div class="dashboard-grid"><section class="panel"><div class="panel-title"><h2>Work queue</h2><span class="eyebrow">THIS RUN</span></div><div class="queue"><div><i class="dot muted"></i><span>Pending</span><b id="pending"></b></div><div><i class="dot cyan"></i><span>Active</span><b id="active"></b></div><div><i class="dot violet"></i><span>Persisting</span><b id="persistence"></b></div><div><i class="dot amber"></i><span>Retry scheduled</span><b id="retry"></b></div></div><p class="footnote">Next eligible <span id="next-due"></span></p><div class="panel-divider"></div><h3>Discovery</h3><p id="discovery" class="muted-text"></p><div class="mini-stats"><div><b id="discovered"></b><span>newly discovered</span></div><div><b id="seen"></b><span>distinct seen</span></div></div></section>
      <section class="panel"><div class="panel-title"><h2>Network & processing</h2><span class="live-mark">● LIVE</span></div><dl class="details"><div><dt>Website pacing</dt><dd id="website-delay"></dd></div><div><dt>API pacing</dt><dd id="api-delay"></dd></div><div><dt>Server cooldown</dt><dd id="cooldown"></dd></div><div><dt>Capacity</dt><dd id="workers"></dd></div></dl><div class="panel-divider"></div><div class="processing"><span id="downloads"></span><span id="parsers"></span></div><p class="footnote">Parser updates <span id="reprocess"></span></p></section></div>
      <section class="panel activity"><div class="panel-title"><h2>Recent activity</h2><span class="eyebrow">LATEST 100 EVENTS</span></div><ol id="events"></ol></section><p class="page-note">Successful products are first captures. Discovery does not measure storefront completeness.</p>`;
      button('start', () => void perform(() => invoke('start', { reprocess: false })));
      button('stop', () => { void invoke('stop').then(refresh).catch(alert); });
      button('resume', () => void perform(() => invoke('resume')));
      button('inspect', () => go('failures'));
      el<HTMLInputElement>('target').onchange = e => { const selected = (e.target as HTMLInputElement).checked; void perform(() => invoke('select_target', { selected })); };
    } else if (view === 'storage') {
      content.innerHTML = heading('WORKSPACE / STORAGE', 'One folder. Your entire catalogue.', 'The desktop app uses the same files as the terminal app.') + `<section class="panel reading"><div class="panel-title"><h2>Local storage</h2><span class="status-pill" id="storage-state"></span></div><code class="path">${escape(state.root)}</code><dl class="details files"><div><dt>catalogue.sqlite3</dt><dd>Products, queue, runs and recovery intents</dd></div><div><dt>archives/</dt><dd>Verified successful responses</dd></div><div><dt>settings.toml</dt><dd>Target and browser preferences</dd></div><div><dt>secrets.json</dt><dd>Explicitly remembered cookies</dd></div><div><dt>webview/</dt><dd>Desktop interface cache</dd></div></dl><p id="storage-detail" class="muted-text"></p><button class="secondary" data-operation id="retry-storage">Check storage again</button></section><section class="note"><h3>Continue where you left off</h3><p>Existing successful products are preserved and skipped. Every new crawl recovers pending saves, retries unfinished products and runs sitemap discovery again.</p><p>To back up, close the application and copy the complete .kaijucrawl folder, including the database and archives.</p></section>`;
      button('retry-storage', () => void perform(() => invoke('check_storage')));
    } else if (view === 'about') {
      content.innerHTML = heading('WORKSPACE / ABOUT', 'Built for the long run.', 'KaijuCrawl · a desktop catalogue of first successful captures.') + `<section class="panel reading"><dl class="details"><div><dt>Application</dt><dd>${escape(state.version)} · Tauri desktop</dd></div><div><dt>Catalogue schema</dt><dd>${state.catalogue_version}</dd></div><div><dt>Product record schema</dt><dd>${state.record_version}</dd></div><div><dt>Parser</dt><dd>${escape(state.parser_version)}</dd></div><div><dt>Storage</dt><dd>SQLite + immutable local archives</dd></div></dl></section><section class="panel reading"><h2>Reprocess saved responses</h2><p class="muted-text">Apply parser updates to saved captures. No storefront requests are made. Observation times and archives remain intact; a failed parse preserves the previous good record.</p><p>${number(state.outdated)} captures use an older parser.</p><button id="reprocess-button" data-operation class="secondary">Reprocess saved responses</button></section>`;
      button('reprocess-button', () => void perform(async () => { await invoke('start', { reprocess: true }); go('dashboard'); }));
    } else if (view === 'failures') {
      content.innerHTML = heading('WORKSPACE / UNSUCCESSFUL JOBS', 'Every outcome, accounted for.', 'Unsuccessful products become eligible again on the next run.') + `<section class="panel"><div class="panel-title"><h2>Current run</h2><button class="quiet" id="refresh-failures">Refresh</button></div><ol class="failure-list" id="failure-rows"><li>Loading…</li></ol><div class="actions"><button class="secondary" id="first-failures">First page</button><button class="secondary" id="next-failures">Next 100</button></div></section>`;
      button('first-failures', () => void loadFailures(0)); button('refresh-failures', () => void loadFailures(failureAfter));
      button('next-failures', () => { if (failureNext !== null) void loadFailures(failureNext); });
    }
  }
  async function loadFailures(after: number) {
    try {
      const page = await invoke<FailurePage>('failure_page', { after });
      if (view !== 'failures') return;
      failureAfter = after; failureNext = page.next;
      el('failure-rows').innerHTML = page.rows.length ? page.rows.map(row => `<li>${escape(row)}</li>`).join('') : '<li class="empty">No unsuccessful jobs to show for this run.</li>';
      el<HTMLButtonElement>('next-failures').disabled = page.next === null;
      el<HTMLButtonElement>('first-failures').disabled = after === 0;
    } catch (e) { alert(e); }
  }
  function steps(): Step[] { return draft.method === 'manual' ? ['method', 'credentials', 'retention', 'review'] : draft.method === 'browser' ? ['method', 'profile', 'review'] : ['method', 'review']; }
  function renderWizard() {
    const route = steps(), index = route.indexOf(step);
    const titles: Record<Step, string> = { method: 'How would you like to connect?', credentials: 'Bring your session with you.', profile: 'Choose your browser session.', retention: 'Remember this session?', review: 'Ready to check access.' };
    content.innerHTML = heading('WORKSPACE / AUTHENTICATION', 'A connection on your terms.', 'Configure DriveThruRPG access, one step at a time.') + `<div class="wizard-layout"><section class="panel wizard"><div class="wizard-progress"><span>STEP ${index + 1} OF ${route.length}</span><span>${route.map((_, n) => `<i class="${n <= index ? 'done' : ''}"></i>`).join('')}</span></div><h2>${titles[step]}</h2><div id="question"></div><p class="form-error" role="alert" id="form-error"></p><div class="wizard-actions"><button class="quiet" id="back">${index ? '← Back' : 'Cancel'}</button><div>${step === 'review' ? '<button class="secondary" data-operation id="save-only">Save only</button><button class="primary" data-operation id="save-test">Save & test access</button>' : '<button class="primary" data-operation id="next">Continue →</button>'}</div></div></section><aside class="wizard-aside"><span class="eyebrow">YOUR SESSION</span><h3>Private by design.</h3><p>Cookies are used only for matching DriveThruRPG requests. Imported cookies stay in memory.</p><p>Test Access checks reachability without saving products or changing the crawl queue.</p><div class="aside-rule"></div><p>Browser access and a public HTTP 200 do not prove authenticated account access.</p><button class="text-button danger" data-operation id="clear-auth">Clear saved authentication</button></aside></div>`;
    const q = el('question');
    if (step === 'method') {
      q.innerHTML = `<p class="muted-text">Choose the source of your session cookies.</p><div class="choices" role="radiogroup" aria-label="Authentication method">${([['browser', 'Import from browser', 'Use a selected Firefox, Chrome, Edge or Brave profile.'], ['manual', 'Paste cookie headers', 'Copy the request header for each exact host.'], ['anonymous', 'No cookies', 'Try public access without a saved session.']] as const).map(([value, title, description]) => `<label class="choice"><input type="radio" name="method" value="${value}" ${draft.method === value ? 'checked' : ''}/><span><b>${title}</b><small>${description}</small></span></label>`).join('')}</div>`;
      q.querySelectorAll<HTMLInputElement>('[name=method]').forEach(input => input.onchange = () => {
        draft.method = input.value as Draft['method'];
        const progress = content.querySelector('.wizard-progress')!;
        progress.innerHTML = `<span>STEP 1 OF ${steps().length}</span><span>${steps().map((_, i) => `<i class="${i === 0 ? 'done' : ''}"></i>`).join('')}</span>`;
      });
    } else if (step === 'credentials') {
      q.innerHTML = `<p class="muted-text">Paste the complete Cookie request header for each exact host. Values stay masked.</p>${state?.has_manual_cookies ? `<label class="check"><input id="keep-saved" type="checkbox" ${draft.keep_saved ? 'checked' : ''}/> Keep the currently loaded cookie headers</label>` : ''}<div id="cookie-fields"><label class="field">Website · www.drivethrurpg.com<input id="website-cookie" type="password" autocomplete="off" spellcheck="false" maxlength="1048576" value="${escape(draft.website_cookie)}" /></label><label class="field">API · api.drivethrurpg.com<input id="api-cookie" type="password" autocomplete="off" spellcheck="false" maxlength="1048576" value="${escape(draft.api_cookie)}" /></label><label class="check"><input type="checkbox" id="reveal"/> Show these inputs temporarily</label></div>`;
      const fields = el('cookie-fields'); fields.hidden = draft.keep_saved;
      const keep = root.querySelector<HTMLInputElement>('#keep-saved');
      if (keep) keep.onchange = () => { draft.keep_saved = keep.checked; fields.hidden = keep.checked; };
      el<HTMLInputElement>('website-cookie').oninput = e => draft.website_cookie = (e.target as HTMLInputElement).value;
      el<HTMLInputElement>('api-cookie').oninput = e => draft.api_cookie = (e.target as HTMLInputElement).value;
      el<HTMLInputElement>('reveal').onchange = e => { for (const id of ['website-cookie', 'api-cookie']) el<HTMLInputElement>(id).type = (e.target as HTMLInputElement).checked ? 'text' : 'password'; };
    } else if (step === 'profile') {
      q.innerHTML = `<p class="muted-text">Only cookies for DriveThruRPG are read. Your browser files are never changed.</p><label class="field">Browser profile<select id="profile"><option value="">Select a profile…</option>${profiles.map(p => `<option value="${p.id}" ${draft.profile_id === p.id ? 'selected' : ''}>${escape(p.browser)} · ${escape(p.name)}</option>`).join('')}</select></label><label class="field">Firefox container<select id="container"></select></label><p class="muted-text">${profiles.length ? 'Encrypted or unsupported browser contexts offer manual paste instead.' : 'No supported profiles found. Go Back and choose manual paste.'}</p>`;
      const containers = () => {
        const options = profiles.find(p => p.id === draft.profile_id)?.containers ?? [[0, 'Default'] as [number, string]];
        if (!options.some(([id]) => id === draft.container)) draft.container = options[0]?.[0] ?? 0;
        el<HTMLSelectElement>('container').innerHTML = options.map(([id, name]) => `<option value="${id}" ${draft.container === id ? 'selected' : ''}>${escape(name)}</option>`).join('');
      }; containers();
      el<HTMLSelectElement>('profile').onchange = e => { const value = (e.target as HTMLSelectElement).value; draft.profile_id = value === '' ? null : Number(value); containers(); };
      el<HTMLSelectElement>('container').onchange = e => draft.container = Number((e.target as HTMLSelectElement).value);
    } else if (step === 'retention') {
      q.innerHTML = `<p class="muted-text">Choose how long manual cookies stay available.</p><div class="choices"><label class="choice"><input type="radio" name="remember" value="no" ${!draft.remember ? 'checked' : ''}/><span><b>This session only</b><small>Keep cookies in memory. Clear previously remembered cookies on save.</small></span></label><label class="choice"><input type="radio" name="remember" value="yes" ${draft.remember ? 'checked' : ''}/><span><b>Remember on this computer</b><small>Save in .kaijucrawl/secrets.json. This file is not encrypted.</small></span></label></div>`;
      q.querySelectorAll<HTMLInputElement>('[name=remember]').forEach(input => input.onchange = () => draft.remember = input.value === 'yes');
    } else {
      q.innerHTML = `<p class="muted-text">Save these settings, then check the connection. A failed test keeps this wizard available for corrections.</p><dl class="details"><div><dt>Method</dt><dd>${escape({ anonymous: 'No cookies', manual: 'Manual headers', browser: 'Browser import' }[draft.method])}</dd></div>${draft.method === 'manual' ? `<div><dt>Cookie headers</dt><dd>${draft.keep_saved ? 'Keep currently loaded headers' : '•••••••• · masked'}</dd></div><div><dt>Remember</dt><dd>${draft.remember ? 'Local secrets file' : 'Session only'}</dd></div>` : draft.method === 'browser' ? `<div><dt>Profile</dt><dd>${escape(profiles.find(p => p.id === draft.profile_id)?.name ?? state?.browser_name ?? '')}</dd></div><div><dt>Container</dt><dd>${draft.container}</dd></div>` : ''}</dl><label class="field">Product ID or URL <span class="optional">optional</span><input id="product-id" value="${escape(draft.product_id)}" placeholder="Use a known product to check parsing" maxlength="2048"/></label><p class="footnote">If empty, the diagnostic uses an existing known product when available. This never creates a seed.</p>`;
      el<HTMLInputElement>('product-id').oninput = e => draft.product_id = (e.target as HTMLInputElement).value;
    }
    const focusQuestion = () => { const h = content.querySelector<HTMLElement>('.wizard h2'); if (h) { h.tabIndex = -1; h.focus(); } };
    button('back', () => { if (!index) go('dashboard'); else { step = route[index - 1]; renderWizard(); update(); focusQuestion(); } });
    button('next', () => { void perform(async () => {
      if (step === 'credentials' && !draft.keep_saved) for (const value of [draft.website_cookie, draft.api_cookie]) {
        if (/[\r\n]/.test(value)) throw 'Paste a single Cookie header value, without line breaks.';
        if (value && !value.includes('=')) throw 'Cookie headers must contain name=value pairs.';
      }
      if (step === 'profile' && draft.profile_id === null) throw 'Choose a browser profile.';
      const next = steps()[steps().indexOf(step) + 1];
      if (next === 'profile') {
        profiles = await invoke<Profile[]>('browser_profiles');
        if (draft.profile_id === null && state?.browser_name && profiles.length) draft.profile_id = profiles[0].id;
      }
      step = next; renderWizard(); focusQuestion();
    }); });
    button('clear-auth', () => void perform(async () => { await invoke('clear_auth'); await refresh(); go('auth'); }));
    button('save-only', () => void perform(async () => { await invoke('save_auth', { draft }); draft.website_cookie = ''; draft.api_cookie = ''; go('dashboard'); }));
    button('save-test', () => void perform(async () => {
      await invoke('save_auth', { draft });
      // Values now live in Rust. Back keeps their presence, not a second DOM copy.
      draft.website_cookie = ''; draft.api_cookie = ''; draft.keep_saved = draft.method === 'manual';
      access = null; view = 'access'; renderAccess();
      try { access = await invoke<AccessResult>('test_access', { productId: draft.product_id }); }
      catch (error) { access = { passed: false, text: String(error) }; }
      if (view === 'access') renderAccess();
    }));
  }
  function renderAccess() {
    content.innerHTML = heading('AUTHENTICATION / TEST ACCESS', access ? (access.passed ? 'Connection checked.' : 'Let’s check the connection.') : 'Checking your connection…', 'No products, queue entries or archives are written by this diagnostic.') + `<section class="panel reading"><span class="status-pill ${access?.passed ? '' : 'warning'}">${access ? access.passed ? 'Access checks passed' : 'Action needed' : 'In progress'}</span><pre class="access-report" id="access-report">${escape(access?.text ?? 'Loading scoped cookies and checking website / API robots policies…')}</pre><div class="actions"><button id="back-auth" class="secondary">← Back to authentication</button><button id="access-done" class="primary">Return to overview</button></div></section>`;
    button('back-auth', () => { view = 'auth'; step = 'review'; renderWizard(); update(); });
    button('access-done', () => go('dashboard'));
  }
  render(); void poll();
  return { dispose() { disposed = true;books.dispose();for (const dashboard of Object.values(dashboards)) dashboard.dispose();delete root.dataset.theme;if (timer) clearTimeout(timer); draft.website_cookie = ''; draft.api_cookie = ''; root.classList.remove('menu-open'); root.innerHTML = ''; } };
}
