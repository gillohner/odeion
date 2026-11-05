// Isolated browser decode regression: generated PCM WAV, real HTMLAudioElement,
// volume zero, real candidate UI. Never touches a node or a streaming provider.
// node tools/ui-browser-library.js [repo] [shots]
const {chromium}=require(require.resolve('playwright',{paths:[process.cwd(),__dirname]}));
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const {create,go,track,summary,SONG,LINK}=require('./ui-fixture');
const shots=path.resolve(process.argv[3]||path.join(process.env.TMPDIR||'.','browser-library-shots'));
function wav(){const rate=16000,n=rate*30,b=Buffer.alloc(44+n*2);b.write('RIFF');b.writeUInt32LE(b.length-8,4);b.write('WAVEfmt ',8);b.writeUInt32LE(16,16);b.writeUInt16LE(1,20);b.writeUInt16LE(1,22);b.writeUInt32LE(rate,24);b.writeUInt32LE(rate*2,28);b.writeUInt16LE(2,32);b.writeUInt16LE(16,34);b.write('data',36);b.writeUInt32LE(n*2,40);for(let i=0;i<n;i++)b.writeInt16LE(Math.round(Math.sin(i*2*Math.PI*220/rate)*1000),44+i*2);return b;}
const audio=wav();
const tracks=Array.from({length:9},(_,k)=>({...track,id:k+1,title:`Library ${k+1}`}));
const tidal={index:9,uri:'tidal:track:999',title:'Muzika',artist:'Sefa, Dr. Peacock',stream_uri:'tidal:track:999',plays_from:{service:'tidal',label:'TIDAL'},sources:[]};
async function setup(browser,width,mode='mixed',duplicate=false,token='fixture-owner'){
 const count=9,ts=tracks.map(t=>({...t})),items=ts.map((t,k)=>({uri:`https://musicbrainz.org/recording/00000000-0000-4000-8000-${String(k+1).padStart(12,'0')}`}));
 const ps=ts.map((t,k)=>({index:k,uri:items[k].uri,title:t.title,artist:'Sefa, Dr. Peacock',track_id:t.id,plays_from:{service:'library',label:'Your library'},sources:[]}));
 if(mode==='mixed'){items.push({uri:'https://tidal.com/track/999'});ts.push(null);ps.push(tidal);}
 if(duplicate){items.splice(2,0,items[0]);ts.splice(2,0,tracks[0]);ps.splice(2,0,{...ps[0],index:2});}
 items.splice(1,0,{uri:LINK});ts.splice(1,0,null);ps.splice(1,0,{uri:LINK,plays_from:null,sources:[]});
 const entries=items.map((it,i)=>({kind:it.uri===LINK?'link':'track',...it}));
 const d={summary:{...summary,items:items.length},playlist:{...summary,items},tracks:ts,entries,local_tracks:[]};
 const pf={order:{order:['library','tidal'],services:[{id:'library',connected:true},{id:'tidal',connected:true}]},own_order:false,items:ps};
 const fx={'/playlists/c1':{body:d},'/playlists/c1/play-from':{body:pf}};
 tracks.forEach(t=>fx[`/library/tracks/${t.id}`]={body:t});
 const run=await create(browser,width,{realAudio:true,token,tracks,collections:[d.summary],fixtures:fx});
 const streams=[];run.streams=streams;
 await run.context.route(/\/v1\/(?:library\/tracks\/[^/]+\/file|sources\/tidal\/stream\/999)/,r=>{
  const u=new URL(r.request().url()),range=r.request().headers().range||'',match=/bytes=(\d+)-(\d*)/.exec(range);
  const start=match?Number(match[1]):0,end=match&&match[2]?Math.min(Number(match[2]),audio.length-1):audio.length-1;
  streams.push({path:u.pathname,token:u.searchParams.get('token'),range,status:match?206:200,contentType:'audio/wav'});
  if(u.searchParams.get('token')!==token)return r.fulfill({status:401,body:'not audio'});
  if(run.failFiles&&u.pathname.includes('/library/'))return r.fulfill({status:503,contentType:'text/html',body:'Unavailable'});
  return r.fulfill({status:match?206:200,contentType:'audio/wav',headers:{'accept-ranges':'bytes','content-length':String(end-start+1),...(match?{'content-range':`bytes ${start}-${end}/${audio.length}`}:{})},body:audio.subarray(start,end+1)});
 });
 await go(run.page,'/playlist/c1');await run.page.locator('#plsongs [data-splay]').first().waitFor();
 run.expected=ps.filter(p=>p.plays_from).map(p=>p.track_id!=null?`track:${p.track_id}`:p.stream_uri);
 return run;
}
async function until(f,label){const t=Date.now();while(Date.now()-t<7000){if(await f())return;await new Promise(r=>setTimeout(r,100));}throw new Error('Timed out: '+label);}
async function queue(page){await page.waitForTimeout(2200);return page.evaluate(()=>JSON.parse(localStorage.getItem('pm.browserQueue')||'[]'));}
async function starts(run,id){await until(async()=>{const p=await run.page.evaluate(()=>pmPlayer.now());return p.time>0&&!p.paused;},'real muted audio starts');const now=await run.page.evaluate(()=>pmPlayer.now());assert.match(now.src,new RegExp(`/v1/library/tracks/${id}/file\\?`),'requested library song, not Muzika');assert.equal(now.index,0);assert.ok(run.streams.some(s=>s.path===`/v1/library/tracks/${id}/file`&&s.token===run.state.token&&s.status===206&&s.contentType==='audio/wav'),'authenticated, ranged audio response decoded');}
(async()=>{const browser=await chromium.launch();try{
 for(const width of [1280,390]){
  for(const action of (process.env.ACTION?[process.env.ACTION]:['direct-first','direct-middle','play','shuffle','list-play'])){
   const run=await setup(browser,width);const {page,state}=run;
   if(action==='list-play'){await go(page,'/playlists');await page.locator('[data-plplay="c1"]').click();}else if(action==='play')await page.locator('#plplay').click();else if(action==='shuffle')await page.locator('#plshuffle').click();else await page.locator(`#plsongs [data-splay="${action==='direct-middle'?5:0}"]`).click();
   if(action==='shuffle'){
    await until(async()=>{const p=await page.evaluate(()=>pmPlayer.now());return p.time>0&&!p.paused;},'Shuffle decodes first audio');
    const q=await queue(page);assert.deepEqual(q.map(x=>x.uri).sort(),[...run.expected].sort(),'Shuffle retains every playable item');
    assert.equal(q.filter(x=>x.uri.startsWith('track:')).length,9);assert.equal(q.filter(x=>x.uri==='tidal:track:999').length,1);
   }else{
    const id=action==='direct-middle'?5:1;await starts(run,id);const q=await queue(page);assert.deepEqual(q.map(x=>x.uri),run.expected.slice(action==='direct-middle'?4:0),'direct click and Play keep full ordered suffix');
   }
   assert.deepEqual(state.errors,[]);fs.mkdirSync(shots,{recursive:true});await page.screenshot({path:path.join(shots,`${width}-${action}.png`)});console.log(`PASS ${width} ${action}: actual muted ranged WAV decode, membership/order`);await run.context.close();
  }
 }
 {const run=await setup(browser,820,'library');await run.page.locator('#plplay').click();await starts(run,1);assert.deepEqual((await queue(run.page)).map(x=>x.uri),run.expected);console.log('PASS all-library');await run.context.close();}
 {const run=await setup(browser,820,'mixed',true);await run.page.locator('#plshuffle').click();await until(async()=>{const p=await run.page.evaluate(()=>pmPlayer.now());return p.time>0&&!p.paused;},'duplicate Shuffle audio');assert.deepEqual((await queue(run.page)).map(x=>x.uri).sort(),[...run.expected].sort());assert.equal((await queue(run.page)).filter(x=>x.uri==='track:1').length,2);await run.page.reload();await run.page.waitForTimeout(600);const q=await queue(run.page);assert.deepEqual(q.map(x=>x.uri).sort(),[...run.expected].sort());console.log('PASS Shuffle duplicates, restore full queue');await run.context.close();}
 {const run=await setup(browser,1280);run.failFiles=true;await run.page.locator('#plplay').click();await until(async()=>/503/.test(await run.page.locator('#toast').innerText()),'safe HTTP playback failure shown');await run.page.waitForTimeout(3000);assert.equal((await run.page.evaluate(()=>pmPlayer.now())).index,0,'error never advances through library to TIDAL');assert.ok(!run.streams.some(s=>s.path.includes('/tidal/')));assert.doesNotMatch(await run.page.locator('#toast').innerText(),/fixture-owner|token=|http:\/\//);console.log('PASS 503: useful safe error, bounded retry, no auto-skip');await run.context.close();}
 {const runs=await Promise.all(['fixture-person-A','fixture-person-B'].map(t=>setup(browser,820,'mixed',false,t)));for(const run of runs){await run.page.locator('#plplay').click();await starts(run,1);assert.ok(run.streams.every(s=>s.token===run.state.token));await run.page.evaluate(()=>pmPlayer.play([{uri:'tidal:track:999',title:'Muzika'}]));await until(()=>Promise.resolve(run.streams.some(s=>s.path.includes('/tidal/'))),'caller-bound provider stream');assert.ok(run.streams.every(s=>s.token===run.state.token));}for(const run of runs)await run.context.close();console.log('PASS separate browser callers: library and provider proxies carry only own token');}
 console.log('browser library playback: PASS');
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exit(1);});
