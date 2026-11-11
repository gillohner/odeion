// Regressions for the independent playback/artist-link review (M1–M3).
// Isolated real candidate assets on a fixture origin: no node, no provider,
// no external network (every other origin is aborted and recorded here).
// node tools/ui-review-fixes.js [repo] [shots]
const {chromium}=require(require.resolve('playwright',{paths:[process.cwd(),__dirname]}));
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const {create,go,track,summary,LINK}=require('./ui-fixture');
const shots=path.resolve(process.argv[3]||path.join(process.env.TMPDIR||'.','review-fixes-shots'));
const only=process.env.CASE;
const mb=(n)=>`https://musicbrainz.org/recording/00000000-0000-4000-8000-${String(n).padStart(12,'0')}`;
const lib=(id,k)=>({...track,id,title:`Library ${id}`,artists:[{id:10+id,name:`Artist ${id}`},{id:20+id,name:'Earth, Wind & Fire'}]});
const sleep=(ms)=>new Promise((r)=>setTimeout(r,ms));
async function until(f,label,ms=9000){const t=Date.now();while(Date.now()-t<ms){if(await f())return;await sleep(100);}throw new Error('Timed out: '+label);}
const queue=(page)=>page.evaluate(()=>JSON.parse(localStorage.getItem('pm.browserQueue')||'[]'));
function external(run){run.external=[];run.page.on('request',(r)=>{const u=new URL(r.url());if(u.origin!=='http://odeion-fixture.test')run.external.push(r.url());});}
// A collection whose play-from rows are: library, a link card, the same library song again,
// a Spotify Connect pick (plays_from but no stream: only its canonical page), TIDAL, a
// second library song and an unavailable row; plus a local-only library track.
function mixed(connectFirst){
 const t1=lib(1),t2=lib(2),t3=lib(3);
 let rows=[
  {uri:mb(1),t:t1,p:{track_id:1,stream_uri:'track:1',plays_from:{service:'library',label:'Your library'}}},
  {uri:LINK,link:true,p:{plays_from:null}},
  {uri:mb(1),t:t1,p:{track_id:1,stream_uri:'track:1',plays_from:{service:'library',label:'Your library'}}},
  {uri:mb(50),t:null,p:{title:'Connect only',artist:'Sefa, Dr. Peacock',stream_uri:null,plays_from:{service:'spotify',label:'Spotify · on your Spotify devices',mode:'connect'}}},
  {uri:'https://tidal.com/track/999',t:null,p:{title:'Muzika',artist:'Sefa, Dr. Peacock',stream_uri:'tidal:track:999',plays_from:{service:'tidal',label:'TIDAL'}}},
  {uri:mb(2),t:t2,p:{track_id:2,stream_uri:'track:2',plays_from:{service:'library',label:'Your library'}}},
  {uri:mb(60),t:null,p:{title:'Nowhere',plays_from:null,reason:'Not on your connected services'}},
 ];
 // A Connect pick first: Play must still start the rest, never the Connect page alone
 if(connectFirst)rows=[rows[3],...rows.slice(0,3),...rows.slice(4)];
 const items=rows.map((r)=>({uri:r.uri}));
 const d={summary:{...summary,items:items.length},playlist:{...summary,items},tracks:rows.map((r)=>r.t||null),entries:rows.map((r)=>({kind:r.link?'link':'track',uri:r.uri})),local_tracks:[t3]};
 const pf={order:{order:['library','tidal','spotify'],services:[]},own_order:false,items:rows.map((r,k)=>({index:k,uri:r.uri,sources:[],...r.p}))};
 const fx={'/playlists/c1':{body:d},'/playlists/c1/play-from':{body:pf}};[t1,t2,t3].forEach((t)=>fx[`/library/tracks/${t.id}`]={body:t});
 return {d,pf,fx,tracks:[t1,t2,t3],expected:['track:1','track:1','tidal:track:999','track:2','track:3']};
}
// A collection of n library songs whose page only has flattened credits (an older answer).
function legacy(n){
 const ts=Array.from({length:n},(_,k)=>lib(k+1));
 const items=ts.map((t,k)=>({uri:mb(k+1)}));
 const d={summary:{...summary,items:n},playlist:{...summary,items},tracks:ts.map(()=>null),entries:items.map((it)=>({kind:'track',...it})),local_tracks:[]};
 const pf={order:{order:['library'],services:[]},own_order:false,items:ts.map((t,k)=>({index:k,uri:items[k].uri,title:t.title,artist:`Artist ${t.id}, Earth, Wind & Fire`,track_id:t.id,stream_uri:'track:'+t.id,plays_from:{service:'library',label:'Your library'},sources:[]}))};
 return {ts,fx:{'/playlists/c1':{body:d},'/playlists/c1/play-from':{body:pf}}};
}
// Serve /library/tracks/{id} with a scripted answer per read; count reads and peak overlap.
async function trackReads(run,ts,answer){
 const by=new Map(ts.map((t)=>[String(t.id),t]));run.reads=[];run.active=0;run.peak=0;
 await run.context.route(/\/v1\/library\/tracks\/\d+$/,async(r)=>{
  const id=new URL(r.request().url()).pathname.split('/').pop();const n=run.reads.filter((x)=>x.id===id).length;
  const rec={id,at:Date.now(),n};run.reads.push(rec);run.active++;run.peak=Math.max(run.peak,run.active);
  try{const a=await answer(id,n,rec);if(a==='hang'){await sleep(20000);}else{if(a.delay)await sleep(a.delay);await r.fulfill({status:a.status||200,contentType:'application/json',body:JSON.stringify(a.body!==undefined?a.body:by.get(id))});}}
  catch{/* the page aborted it */}finally{run.active--;}
 });
}
const credits=(page,sel='#plsongs [data-artist-credit]')=>page.locator(sel).allTextContents();
const cases={
 // A skipped Connect notice must not replace a failed local lookup's error.
 async 'm1-play-error-visible'(browser){
  const l=legacy(1);l.fx['/playlists/c1/play-from'].body.items.push({uri:mb(50),stream_uri:null,plays_from:{service:'spotify',mode:'connect'},title:'Connect only'});
  l.fx['/playlists/c1'].body.tracks.push(null);l.fx['/playlists/c1'].body.entries.push({kind:'track',uri:mb(50)});
  const run=await create(browser,1280,{tracks:[],collections:[summary],fixtures:l.fx});
  await trackReads(run,l.ts,()=>({status:503,body:{error:'Track lookup failed'}}));
  await go(run.page,'/playlists');await run.page.locator('[data-plplay="c1"]').click();
  await until(()=>Promise.resolve(run.reads.length>0),'local lookup attempted');await sleep(300);
  assert.match(await run.page.locator('#toast').innerText(),/Track lookup failed/,'actual playback failure remains visible, not replaced by Connect omission');
  assert.deepEqual(run.state.errors,[]);await run.context.close();console.log('PASS list Play keeps failed lookup error visible when Connect rows are omitted');
 },
 // M1: a Connect pick has no stream; its canonical page must never become audio.
 async 'm1-connect-omitted'(browser){
  for(const width of [1280,820,390])for(const how of ['list','detail','detail-connect-first']){
   const m=mixed(how==='detail-connect-first');const run=await create(browser,width,{tracks:m.tracks,collections:[m.d.summary],fixtures:m.fx});external(run);const {page,state}=run;
   if(how==='list'){await go(page,'/playlists');await page.locator('[data-plplay="c1"]').click();}else{await go(page,'/playlist/c1');await page.locator('#plsongs .srow').first().waitFor();await page.locator('#plplay').click();}
   await until(async()=>(await queue(page)).length>0,'browser queue written');await sleep(400);
   const q=await queue(page);const src=(await page.evaluate(()=>pmPlayer.now())).src;
   assert.deepEqual(q.map((x)=>x.uri),m.expected,`${width} ${how}: every library/provider stream kept in order with duplicates; Connect and unavailable omitted`);
   assert.ok(q.every((x)=>!/^https?:/i.test(x.uri||'')),`${width} ${how}: no canonical web page queued as audio`);
   assert.doesNotMatch(src,/musicbrainz|^https?:\/\/(?!odeion-fixture\.test)/,`${width} ${how}: audio element never loads a canonical page`);
   assert.deepEqual(run.external,[],`${width} ${how}: no third-party request`);
   if(how==='list')assert.match(await page.locator('#toast').innerText(),/1 song plays only through Spotify Connect/,'list Play says what it left out');
   if(how==='list')assert.match(await page.locator('#toast').innerText(),/1 song plays nowhere you are connected/,'list Play also counts songs with no source');
   else assert.match(await page.locator('#toast').innerText(),/left out of this browser's queue/,'detail Play says what it left out');
   assert.deepEqual(state.errors,[]);console.log(`PASS ${width} ${how} Play: Connect row omitted, streams/duplicates/order kept, no canonical audio`);
   await run.context.close();
  }
 },
 // M2: rows already carrying structured credits never re-read the track.
 async 'm2-structured-no-refetch'(browser){
  for(const width of [1280,820,390]){
   const m=mixed();const run=await create(browser,width,{tracks:m.tracks,collections:[m.d.summary],fixtures:m.fx});await trackReads(run,m.tracks,()=>({}));const {page,state}=run;
   await go(page,'/playlist/c1');await page.locator('#plsongs .srow').first().waitFor();await sleep(1200);
   assert.deepEqual(run.reads,[],`${width}: structured collection rows need no track reads`);
   const names=await credits(page,'#plsongs .srow:first-child [data-artist-credit]');assert.deepEqual(names,['Artist 1','Earth, Wind & Fire'],'exact structured credit order and comma name');
   assert.equal(await page.locator('#plsongs .srow:first-child [data-artist-credit]').nth(1).getAttribute('href'),'/artist/lib-21');
   await page.locator('#plsongs .srow').first().evaluate((r)=>{const b=r.querySelector('[data-splay]');(b&&b.offsetParent?b:r).click();});await page.locator('#np').click();await page.locator('#nowplaying .np-titles .artist [data-artist-credit]').first().waitFor();await sleep(800);
   // (Now playing reads its song once for the bookmark heart; that is not credit hydration.)
   assert.equal(await page.locator('[data-credit-track]').count(),0,`${width}: player credits use the queued structured artists, nothing to hydrate`);
   assert.deepEqual(await credits(page,'#nowplaying .np-titles .artist [data-artist-credit]'),['Artist 1','Earth, Wind & Fire']);
   assert.deepEqual(state.errors,[]);console.log(`PASS ${width}: structured rows/player: zero hydration reads, exact IDs/order/comma names`);await run.context.close();
  }
 },
 // M2: a page of flattened rows reads a bounded number of tracks, two at a time.
 async 'm2-bounded'(browser){
  const l=legacy(80);const run=await create(browser,1280,{tracks:l.ts,collections:[summary],fixtures:l.fx});await trackReads(run,l.ts,()=>({delay:40}));const {page,state}=run;
  await go(page,'/playlist/c1');await page.locator('#plsongs .srow').first().waitFor();
  await until(async()=>(await credits(page,'#plsongs .srow:first-child [data-artist-credit]')).length===2,'first row hydrated');await sleep(2500);
  const ids=new Set(run.reads.map((r)=>r.id));
  assert.ok(run.reads.length<=48&&ids.size===run.reads.length,`bounded distinct reads per visit: ${run.reads.length}`);assert.ok(run.peak<=2,`peak overlap ${run.peak}`);
  assert.deepEqual(await credits(page,'#plsongs .srow:first-child [data-artist-credit]'),['Artist 1','Earth, Wind & Fire']);
  assert.deepEqual(await credits(page,'#plsongs .srow:last-child [data-artist-credit]'),['Artist 80, Earth, Wind & Fire'],'unread rows keep one intact safe search link');
  assert.deepEqual(state.errors,[]);console.log(`PASS bounded hydration: ${run.reads.length} reads for 80 flattened rows, peak ${run.peak}`);await run.context.close();
 },
 // M2: leaving the page stops queued reads and aborts those in flight.
 async 'm2-navigation-cancel'(browser){
  const l=legacy(30);const run=await create(browser,820,{tracks:l.ts,collections:[summary],fixtures:l.fx});await trackReads(run,l.ts,()=>({delay:700}));const {page,state}=run;
  const aborted=[];page.on('requestfailed',(r)=>{if(/\/library\/tracks\/\d+$/.test(r.url()))aborted.push(r.url());});
  await go(page,'/playlist/c1');await until(()=>Promise.resolve(run.reads.length>=2),'reads started');
  await page.locator('#plsongs [data-artist-credit]').first().click();const left=Date.now();await page.waitForURL(/\/search/);await sleep(2500);
  const after=run.reads.filter((r)=>r.at>left+50);
  assert.deepEqual(after,[],'no read starts after navigating away');assert.ok(run.reads.length<=3,`only the in-flight pair: ${run.reads.length}`);
  assert.ok(aborted.length>=1,'in-flight reads aborted on navigation');
  assert.deepEqual(state.errors,[]);console.log(`PASS navigation cancels hydration: ${run.reads.length} started, ${aborted.length} aborted, none after`);await run.context.close();
 },
 // M3: a transient failure recovers with a bounded retry, on the same page and after navigating back.
 async 'm3-transient-retry'(browser){
  for(const width of [1280,390]){
   const l=legacy(1);const run=await create(browser,width,{tracks:l.ts,collections:[summary],fixtures:l.fx});await trackReads(run,l.ts,(id,n)=>n===0?{status:503,body:{error:'busy'}}:{});const {page,state}=run;
   await go(page,'/playlist/c1');
   await until(async()=>(await credits(page)).length===2,'retry splits the credit without navigation');
   assert.deepEqual(await credits(page),['Artist 1','Earth, Wind & Fire']);assert.equal(run.reads.length,2,'one failure, one retry');
   assert.equal(await page.locator('#plsongs [data-artist-credit]').nth(1).getAttribute('href'),'/artist/lib-21');
   assert.deepEqual(state.errors,[]);await run.context.close();
  }
  // Persistent 503s stop after a bounded number of tries; returning to the page recovers.
  {const l=legacy(1);let healthy=false;const run=await create(browser,820,{tracks:l.ts,collections:[summary],fixtures:l.fx});await trackReads(run,l.ts,()=>healthy?{}:{status:503,body:{error:'busy'}});const {page,state}=run;
   await go(page,'/playlist/c1');await page.locator('#plsongs [data-artist-credit]').first().waitFor();await sleep(9000);
   const tries=run.reads.length;assert.ok(tries>=2&&tries<=3,`bounded retries: ${tries}`);assert.deepEqual(await credits(page),['Artist 1, Earth, Wind & Fire'],'safe unsplit fallback while failing');
   healthy=true;await page.locator('#plsongs [data-artist-credit]').first().click();await page.waitForURL(/\/search/);await page.goBack();
   await until(async()=>(await credits(page)).length===2,'navigating back retries');assert.equal(run.reads.length,tries+1);
   assert.deepEqual(state.errors,[]);await run.context.close();}
  // A 404 is not transient: no retry.
  {const l=legacy(1);const run=await create(browser,820,{tracks:l.ts,collections:[summary],fixtures:l.fx});await trackReads(run,l.ts,()=>({status:404,body:{error:'gone'}}));const {page,state}=run;
   await go(page,'/playlist/c1');await page.locator('#plsongs [data-artist-credit]').first().waitFor();await sleep(4000);assert.equal(run.reads.length,1,'permanent failure is not retried');assert.deepEqual(state.errors,[]);await run.context.close();}
  console.log('PASS transient 503 recovers (same page and after navigation), bounded tries, 404 not retried');
 },
 // M3: a read that never answers times out and is retried.
 async 'm3-timeout'(browser){
  const l=legacy(1);const run=await create(browser,820,{tracks:l.ts,collections:[summary],fixtures:l.fx});await trackReads(run,l.ts,(id,n)=>n===0?'hang':{});const {page,state}=run;
  await go(page,'/playlist/c1');await until(async()=>(await credits(page)).length===2,'timed-out read retried',16000);
  assert.equal(run.reads.length,2);assert.deepEqual(state.errors,[]);fs.mkdirSync(shots,{recursive:true});await page.screenshot({path:path.join(shots,'820-timeout-recovered.png')});
  console.log('PASS hung read times out and the retry hydrates');await run.context.close();
 },
 // Structured credits with an explicit empty join phrase are structured: no read (review note 1).
 async 'm2-empty-join'(browser){
  const t={...lib(1),artists:[{id:11,name:'Artist 1',join_phrase:''}]};const m=mixed();m.d.tracks[0]=t;m.d.tracks[2]=t;
  const run=await create(browser,820,{tracks:m.tracks,collections:[m.d.summary],fixtures:m.fx});await trackReads(run,m.tracks,()=>({}));const {page,state}=run;
  await go(page,'/playlist/c1');await page.locator('#plsongs .srow').first().waitFor();await sleep(1200);
  assert.deepEqual(run.reads,[],'no hydration read');assert.deepEqual(await credits(page,'#plsongs .srow:first-child [data-artist-credit]'),['Artist 1']);
  assert.equal(await page.locator('#plsongs [data-artist-credit]').first().getAttribute('href'),'/artist/lib-11');assert.deepEqual(state.errors,[]);
  console.log('PASS structured credit with empty join phrase: zero reads, exact link');await run.context.close();
 },
 // A slow read for the page being left never writes the next page's credits (review note 2).
 async 'm2-stale-generation'(browser){
  const l=legacy(1);const run=await create(browser,1280,{tracks:l.ts,collections:[summary],fixtures:l.fx});const {page,state}=run;
  await trackReads(run,l.ts,(id,n)=>n===0?{delay:1500,body:{...l.ts[0],artists:[{id:91,name:'Stale A'},{id:92,name:'Stale B'}]}}:{delay:300});
  await go(page,'/playlist/c1');await until(()=>Promise.resolve(run.reads.length===1),'first read started');
  await page.locator('#plsongs [data-artist-credit]').first().click();await page.waitForURL(/\/search/);await page.goBack();
  const seen=new Set();const t0=Date.now();while(Date.now()-t0<3000){for(const n of await credits(page))seen.add(n);await sleep(50);}
  assert.ok(!seen.has('Stale A')&&!seen.has('Stale B'),'stale answer never shown');assert.deepEqual(await credits(page),['Artist 1','Earth, Wind & Fire']);
  assert.deepEqual(state.errors,[]);console.log('PASS stale same-ID read never writes the returned page');await run.context.close();
 },
 // Hydrated links replace the fallback without losing keyboard focus, and an empty
 // structured answer keeps the fallback text.
 async 'm2-focus-empty'(browser){
  {const l=legacy(1);const run=await create(browser,1280,{tracks:l.ts,collections:[summary],fixtures:l.fx});await trackReads(run,l.ts,()=>({delay:600}));const {page,state}=run;
   await go(page,'/playlist/c1');const a=page.locator('#plsongs [data-artist-credit]').first();await a.waitFor();await a.focus();
   await until(async()=>(await credits(page)).length===2,'hydrated');assert.equal(await page.evaluate(()=>document.activeElement&&document.activeElement.dataset.artistCredit),'Artist 1','focus stays on the credit');
   assert.deepEqual(state.errors,[]);await run.context.close();}
  {const l=legacy(1);const run=await create(browser,1280,{tracks:l.ts,collections:[summary],fixtures:l.fx});await trackReads(run,l.ts,()=>({body:{...l.ts[0],artists:[]}}));const {page,state}=run;
   await go(page,'/playlist/c1');await page.locator('#plsongs [data-artist-credit]').first().waitFor();await until(()=>Promise.resolve(run.reads.length===1),'read');await sleep(500);
   assert.deepEqual(await credits(page),['Artist 1, Earth, Wind & Fire'],'empty structured answer keeps the fallback');assert.deepEqual(state.errors,[]);await run.context.close();}
  {const l=legacy(1);const run=await create(browser,1280,{tracks:l.ts,collections:[summary],fixtures:l.fx});await trackReads(run,l.ts,()=>({body:{...l.ts[0],artists:[{id:5},{id:6,name:''}]}}));const {page,state}=run;
   await go(page,'/playlist/c1');await page.locator('#plsongs [data-artist-credit]').first().waitFor();await until(()=>Promise.resolve(run.reads.length===1),'read');await sleep(500);
   assert.deepEqual(await credits(page),['Artist 1, Earth, Wind & Fire'],'nameless credits keep the fallback, no blank links');assert.deepEqual(state.errors,[]);await run.context.close();}
  console.log('PASS hydration keeps focus; empty or nameless answer keeps fallback');
 },
};
(async()=>{const browser=await chromium.launch();let n=0;try{
 for(const [name,f] of Object.entries(cases)){if(only&&!only.split(',').includes(name))continue;console.log(`-- ${name}`);await f(browser);n++;}
 assert.ok(n>0,'zero cases ran');console.log(`review fixes: PASS (${n} cases)`);
}finally{await browser.close();}})().catch((e)=>{console.error(e);process.exit(1);});
