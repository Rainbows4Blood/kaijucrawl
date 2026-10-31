import type { Invoke } from './types';
import './books.css';
export const gameLabels = {dark_heresy:'Dark Heresy',dark_heresy_2e:'Dark Heresy 2nd Edition',rogue_trader:'Rogue Trader',only_war:'Only War',deathwatch:'Deathwatch',black_crusade:'Black Crusade',imperium_maledictum:'Imperium Maledictum',wrath_glory:'Wrath and Glory'} as const;
export type Game = keyof typeof gameLabels;
export type Sort = 'release_date'|'name'|'book_type'|'in_universe_date'|'in_universe_location';
interface Cursor { game:Game;sort:Sort;descending:boolean;key:string|null;target:string;product_id:string }
export interface Book {target:string;product_id:string;name:string;release_date:string|null;book_type:string|null;in_universe_date:string|null;locations:string[];date_analyzed:boolean;locations_analyzed:boolean}
export interface BookPage {books:Book[];total:number;next:Cursor|null}
export interface BookDetails {book:Book;description:string;description_truncated:boolean;publisher:string|null;page_count:number|null;authors:string[];rule_systems:string[];source_product_types:string[];games:string[];captured_at:string}
const columns: [Sort,string][]=[['release_date','Release Date'],['name','Name'],['book_type','Book Type'],['in_universe_date','In Universe Date'],['in_universe_location','In Universe Location']];
const escape=(v:unknown)=>String(v??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'})[c]!);
const date=(v:string|null)=>v?.match(/^\d{4}-\d{2}-\d{2}(?:T|\s|$)/)?.[0].trim().slice(0,10)??v??'—';
const number=(v:number)=>new Intl.NumberFormat().format(v);
interface TableState {sort:Sort;descending:boolean;page:BookPage|null;cursors:(Cursor|null)[];index:number}
export function createBookLibrary(root:HTMLElement,invoke:Invoke) {
  const states=new Map<Game,TableState>();
  let game:Game='dark_heresy',visible=false,disposed=false,epoch=0,pending=false,error='',detail:BookDetails|null=null,detailMode=false,selected:Book|null=null,scroll=0;
  const state=()=>{let s=states.get(game);if(!s){s={sort:'name',descending:false,page:null,cursors:[null],index:0};states.set(game,s);}return s;};
  const el=<T extends HTMLElement=HTMLElement>(id:string)=>root.querySelector<T>(`#books-${id}`)!;
  function back() {epoch++;pending=false;detail=null;detailMode=false;error='';render();const button=root.querySelector<HTMLButtonElement>(`[data-book="${state().page?.books.findIndex(b=>b.target===selected?.target&&b.product_id===selected?.product_id)}"]`);button?.focus({preventScroll:true});root.ownerDocument.defaultView?.scrollTo?.(0,scroll);}
  async function load(reset=false) {
    if(!visible||disposed)return;
    const s=state();if(reset){s.index=0;s.cursors=[null];}const focus=(root.ownerDocument.activeElement as HTMLElement|undefined)?.dataset.sort;s.page=null;const token=++epoch;pending=true;error='';detailMode=false;detail=null;render();
    try {const page=await invoke<BookPage>('warhammer_books',{query:{game,sort:s.sort,descending:s.descending,after:s.cursors[s.index]}});if(token!==epoch||disposed)return;s.page=page;}
    catch(e){if(token!==epoch||disposed)return;error=String(e);s.page=null;}
    finally {if(token===epoch&&!disposed){pending=false;render();if(focus)root.querySelector<HTMLButtonElement>(`[data-sort="${focus}"]`)?.focus({preventScroll:true});}}
  }
  async function open(book:Book) {
    scroll=root.ownerDocument.defaultView?.scrollY??0;selected=book;detailMode=true;detail=null;pending=true;error='';const token=++epoch;render();el<HTMLButtonElement>('back').focus();
    try {const result=await invoke<BookDetails|null>('warhammer_book',{game,target:book.target,productId:book.product_id});if(token!==epoch||disposed)return;detail=result;if(!result)error='This book is no longer in this category. Return to the list and refresh it.';}
    catch(e){if(token!==epoch||disposed)return;error=String(e);}
    finally {if(token===epoch&&!disposed){pending=false;render();el<HTMLButtonElement>('back').focus({preventScroll:true});}}
  }
  function render() {
    const s=state();
    if(detailMode){
      const b=detail?.book??selected;
      root.innerHTML=`<div class="book-detail"><button class="text-button" id="books-back">← Back to ${escape(gameLabels[game])}</button><header><p class="eyebrow">WARHAMMER 40K / ${escape(gameLabels[game])}</p><h1>${escape(b?.name)}</h1></header>${error?`<p class="form-error" role="alert">${escape(error)}</p>`:''}${pending?'<p role="status">Loading saved book details…</p>':''}${detail?`<section class="panel" aria-label="Book details"><dl class="book-facts">${[
        ['Release Date',b!.release_date??'—'],['Book Type',b!.book_type??'Not analyzed'],['In Universe Date',b!.in_universe_date??(b!.date_analyzed?'Unknown':'Not analyzed')],['In Universe Location',b!.locations.join('; ')||(b!.locations_analyzed?'Unknown':'Not analyzed')],['Games',detail.games.map(g=>gameLabels[g as Game]??g).join(' · ')],['Publisher',detail.publisher??'—'],['Authors',detail.authors.join(', ')||'—'],['Pages',detail.page_count??'—'],['Rule System',detail.rule_systems.join(', ')||'—'],['Storefront product types',detail.source_product_types.join(', ')||'—'],['Catalogue identity',`${b!.target} · ${b!.product_id}`],['Captured',detail.captured_at]
      ].map(([label,value])=>`<div><dt>${label}</dt><dd>${escape(value)}</dd></div>`).join('')}</dl></section><section class="panel book-description"><h2>Description</h2><div>${escape(detail.description)}</div>${detail.description_truncated?'<p class="muted-text">This unusually long description is truncated in this view.</p>':''}</section>`:''}</div>`;
      el('back').onclick=back;return;
    }
    const rows=s.page?.books??[];
    root.innerHTML=`<header class="book-list-heading"><div><p class="eyebrow">WARHAMMER 40K / COLLECTION</p><h1>${escape(gameLabels[game])}</h1><p class="subtitle">Books with a saved ${escape(gameLabels[game])} classification.</p></div><button class="secondary" id="books-refresh" ${pending?'disabled':''}>Refresh books</button></header>
      <section class="panel book-list" aria-label="${escape(gameLabels[game])} books" aria-busy="${pending}"><div class="book-list-status"><p role="status">${pending?'Loading saved books…':s.page?`${number(s.page.total)} ${s.page.total===1?'book':'books'} · ${rows.length?`${number(s.index*50+1)}–${number(s.index*50+rows.length)}`:'0'} shown`:''}</p><span class="muted-text">Select a book for details</span></div>
      ${error?`<p class="form-error" role="alert">${escape(error)}</p>`:''}<div class="book-table-scroll" tabindex="0" aria-label="Book table; scroll horizontally if needed"><table><caption class="sr-only">${escape(gameLabels[game])} books; select a column heading to sort</caption><thead><tr>${columns.map(([key,label])=>`<th scope="col" aria-sort="${s.sort===key?(s.descending?'descending':'ascending'):'none'}"><button data-sort="${key}" ${pending?'disabled':''}>${label}<span aria-hidden="true">${s.sort===key?(s.descending?'↓':'↑'):'↕'}</span></button></th>`).join('')}</tr></thead><tbody>${rows.map((b,i)=>`<tr data-row="${i}"><td>${escape(date(b.release_date))}</td><td><button class="book-name" data-book="${i}">${escape(b.name)}</button></td><td>${escape(b.book_type??'—')}</td><td title="${b.date_analyzed?'Saved analysis':'Not analyzed'}">${escape(b.in_universe_date??'—')}</td><td title="${b.locations_analyzed?'Saved analysis':'Not analyzed'}">${escape(b.locations.join('; ')||'—')}</td></tr>`).join('')}</tbody></table></div>
      ${!pending&&!error&&!rows.length?`<div class="books-empty"><h2>${s.index?'No more books on this page.':'No books classified here yet.'}</h2><p>${s.index?'Refresh the list to see the current category.':'Run the 40k analyses to populate this category. Books can appear in more than one game tab.'}</p></div>`:''}
      <div class="book-pagination"><button class="secondary" id="books-prev" ${pending||s.index===0?'disabled':''}>← Previous</button><span>Page ${s.index+1}</span><button class="secondary" id="books-next" ${pending||!s.page?.next?'disabled':''}>Next →</button></div></section><p class="page-note">Book Type uses Primary Book Role. Missing values appear last in either sort direction. In-universe dates and locations sort as text. Refresh to include newly saved analysis results.</p>`;
    el('refresh').onclick=()=>void load(true);
    el('prev').onclick=()=>{if(s.index){s.index--;void load();}};
    el('next').onclick=()=>{if(s.page?.next){s.cursors[s.index+1]=s.page.next;s.index++;void load();}};
    root.querySelectorAll<HTMLButtonElement>('[data-sort]').forEach(b=>b.onclick=()=>{const key=b.dataset.sort as Sort;s.descending=s.sort===key?!s.descending:false;s.sort=key;void load(true);});
    root.querySelectorAll<HTMLTableRowElement>('[data-row]').forEach(row=>row.onclick=()=>{if(!pending)void open(rows[Number(row.dataset.row)]);});
    // Native buttons make each row's detail action keyboard-accessible.
  }
  return {
    show(next:Game|null){epoch++;pending=false;visible=next!==null;if(next===null)return;const changed=game!==next;game=next;if(changed){detailMode=false;detail=null;error='';}render();if(detailMode&&!detail&&!error&&selected)void open(selected);else if(!state().page&&!detailMode)void load();},
    dispose(){disposed=true;epoch++;root.innerHTML='';},
  };
}
