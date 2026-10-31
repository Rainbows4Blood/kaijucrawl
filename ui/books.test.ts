import { afterEach,expect,it,vi } from 'vitest';
import { screen,waitFor,within } from '@testing-library/dom';
import userEvent from '@testing-library/user-event';
import { createBookLibrary,gameLabels,type Book,type BookDetails,type BookPage } from './books';
import { createApp } from './app';
import type { Invoke } from './types';
import { fixture } from './test/fixture';
import { workspaces } from './workspaces';
let dispose:(()=>void)|undefined;
afterEach(()=>{dispose?.();vi.restoreAllMocks();});
const book:Book={target:'drivethru',product_id:'42',name:'The <script>Archive</script>',release_date:'2024-02-03T00:00:00Z',book_type:'Adventure',in_universe_date:'late M41',locations:['Scintilla','Calixis Sector'],date_analyzed:true,locations_analyzed:true};
const detail:BookDetails={book,description:'Saved description\n<img src=x onerror=alert(1)>',description_truncated:false,publisher:'Fictional Press',page_count:32,authors:['Example Author'],rule_systems:['Warhammer 40k Roleplay'],source_product_types:['Source tag'],games:['dark_heresy','rogue_trader'],captured_at:'2026-01-01'};
const page:BookPage={books:[book],total:1,next:null};
function root(){document.body.innerHTML='<div id="test"></div>';return document.querySelector<HTMLElement>('#test')!;}
it('loads only on a game tab, sorts all five columns through the backend, and keeps details isolated',async()=>{
  const element=root();vi.spyOn(window,'scrollTo').mockImplementation(()=>{});
  const invoke=vi.fn(async(command:string)=>command==='warhammer_book'?structuredClone(detail):structuredClone(page));
  const view=createBookLibrary(element,invoke as Invoke);dispose=()=>view.dispose();expect(invoke).not.toHaveBeenCalled();view.show('dark_heresy');
  await screen.findByRole('button',{name:book.name});
  expect(screen.getByText('2024-02-03')).toBeTruthy();expect(element.querySelector('script')).toBeNull();
  for(const [label,sort] of [['Release Date','release_date'],['Name','name'],['Book Type','book_type'],['In Universe Date','in_universe_date'],['In Universe Location','in_universe_location']]){
    await userEvent.click(screen.getByRole('button',{name:new RegExp(`^${label}`)}));
    await waitFor(()=>expect(element.querySelector('[aria-busy=true]')).toBeNull());
    expect(invoke).toHaveBeenLastCalledWith('warhammer_books',{query:{game:'dark_heresy',sort,descending:false,after:null}});
    await userEvent.click(screen.getByRole('button',{name:new RegExp(`^${label}`)}));
    await waitFor(()=>expect(element.querySelector('[aria-busy=true]')).toBeNull());
    expect(invoke).toHaveBeenLastCalledWith('warhammer_books',{query:{game:'dark_heresy',sort,descending:true,after:null}});
    expect(element.querySelector('th[aria-sort=descending]')?.textContent).toContain(label);
  }
  const requests=invoke.mock.calls.length;
  await userEvent.click(screen.getByRole('button',{name:book.name}));await screen.findByRole('heading',{name:'Description'});
  expect(screen.queryByRole('table')).toBeNull();expect(element.querySelector('.book-description>div')?.textContent).toBe(detail.description);expect(element.querySelector('img')).toBeNull();
  expect(screen.getByText('Dark Heresy · Rogue Trader')).toBeTruthy();
  await userEvent.click(screen.getByRole('button',{name:/Back to Dark Heresy/}));
  expect(screen.getByRole('table')).toBeTruthy();expect(invoke.mock.calls.length).toBe(requests+1);
  expect(document.activeElement).toBe(screen.getByRole('button',{name:book.name}));
});
it('keeps paging and sorting per game, and returning from details retains the current page',async()=>{
  vi.spyOn(window,'scrollTo').mockImplementation(()=>{});
  const cursor={game:'dark_heresy',sort:'name',descending:false,key:'a',target:'drivethru',product_id:'1'};
  const invoke=vi.fn(async(command:string,args?:Record<string,any>)=>command==='warhammer_book'?detail:args?.query.after?{...page,books:[{...book,name:'Second page'}],total:51}:{...page,total:51,next:cursor});
  const view=createBookLibrary(root(),invoke as Invoke);dispose=()=>view.dispose();view.show('dark_heresy');await screen.findByRole('button',{name:book.name});
  await userEvent.click(screen.getByRole('button',{name:'Next →'}));await screen.findByRole('button',{name:'Second page'});expect(screen.getByText('Page 2')).toBeTruthy();
  await userEvent.click(screen.getByRole('button',{name:'Second page'}));await screen.findByRole('heading',{name:'Description'});await userEvent.click(screen.getByRole('button',{name:/Back to/}));expect(screen.getByText('Page 2')).toBeTruthy();
  view.show('rogue_trader');await screen.findByRole('button',{name:book.name});expect(screen.getByText('Page 1')).toBeTruthy();
  view.show('dark_heresy');expect(screen.getByText('Page 2')).toBeTruthy();
  await userEvent.click(screen.getByRole('button',{name:'← Previous'}));await screen.findByRole('button',{name:book.name});expect(screen.getByText('Page 1')).toBeTruthy();
});
it('rejects late pages from another game and handles empty, error and missing-detail states',async()=>{
  let finish!:(value:BookPage)=>void;
  const invoke=vi.fn(async(command:string,args?:Record<string,any>)=>command==='warhammer_book'?null:args?.query.game==='dark_heresy'?new Promise<BookPage>(r=>finish=r):{books:[],total:0,next:null});
  const element=root(),view=createBookLibrary(element,invoke as Invoke);dispose=()=>view.dispose();view.show('dark_heresy');view.show('only_war');
  await screen.findByText('No books classified here yet.');finish(page);await new Promise(r=>setTimeout(r,0));expect(screen.queryByRole('button',{name:book.name})).toBeNull();
  invoke.mockImplementation(async()=>{throw new Error('Synthetic read error');});await userEvent.click(screen.getByRole('button',{name:'Refresh books'}));await screen.findByRole('alert');expect(screen.getByRole('alert').textContent).toContain('Synthetic read error');
  invoke.mockImplementation(async(command:string)=>command==='warhammer_book'?null:page);await userEvent.click(screen.getByRole('button',{name:'Refresh books'}));await screen.findByRole('button',{name:book.name});await userEvent.click(screen.getByRole('button',{name:book.name}));await screen.findByText(/no longer in this category/);
});
it('retains Analysis and exposes only eight named game tabs with arrow navigation, without paid work',async()=>{
  const element=root();const totals={catalogue_total:1,analyses:Object.fromEntries(Object.values(workspaces).flatMap(w=>w.stages).map(s=>[s.algorithm,{eligible:1,completed:1,prerequisite_pending:0,counts:{}}]))};
  const invoke=vi.fn(async(command:string)=>command==='snapshot'?structuredClone(fixture):command==='analysis_overview'?totals:page);
  const app=createApp(element,invoke as Invoke,60000);dispose=()=>app.dispose();await screen.findByRole('tab',{name:'Overview'});
  await userEvent.click(screen.getByRole('button',{name:'Open main menu'}));await userEvent.click(element.querySelector('#menu-warhammer_40k')!);
  expect(screen.getAllByRole('tab').map(t=>t.textContent?.replace('◈',''))).toEqual(['Analysis',...Object.values(gameLabels)]);
  expect(invoke.mock.calls.some(c=>c[0]==='warhammer_books')).toBe(false);
  await userEvent.click(screen.getByRole('tab',{name:'Dark Heresy'}));await screen.findByRole('button',{name:book.name});
  expect(screen.queryByRole('button',{name:'Run analyses'})).toBeNull();expect(screen.getByRole('tabpanel',{name:'Dark Heresy'})).toBeTruthy();
  screen.getByRole('tab',{name:'Dark Heresy'}).focus();await userEvent.keyboard('{End}');await waitFor(()=>expect(screen.getByRole('tab',{name:'Wrath and Glory'}).getAttribute('aria-selected')).toBe('true'));
  await userEvent.keyboard('{Home}');expect(screen.getByRole('tab',{name:'Analysis'}).getAttribute('aria-selected')).toBe('true');expect(screen.queryByRole('table')).toBeNull();
  expect(invoke.mock.calls.every(c=>['snapshot','analysis_overview','warhammer_books'].includes(c[0]))).toBe(true);
  expect(within(element.querySelector('#warhammer_40k-workspace')!).getAllByRole('tab')).toHaveLength(9);
});
