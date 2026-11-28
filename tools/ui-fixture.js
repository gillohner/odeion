// Isolated browser fixtures for UI regressions. Real candidate assets, no live node,
// no credentials, no network egress, and no services or persistent data changed.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const ORIGIN = 'http://odeion-fixture.test';
const ROOT = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const STATIC = path.join(ROOT, 'crates/pm-web/static');
const ui = require('./ui-src'); // app.js is joined from its parts, as the node's build does
const SONG = 'https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69';
const LINK = 'https://example.com/liner-notes';
const artists = [{id: 11, name: 'Sefa'}, {id: 22, name: 'Dr. Peacock'}];
const track = {id: 1, title: 'Journey Through Time', artists, album: {id: 7, title: 'Journey'}, duration_ms: 240000, mbid: SONG.split('/').pop(), canonical_uri: SONG, file: {container: 'flac', format: {kind:'pcm',bits:16,rate_hz:44100,channels:2}}};
const summary = {id: 'c1', name: 'Music and notes', items: 2, covers: [], visibility: 'private', synced: false, layout:'list'};
const detail = {summary, playlist:{...summary,items:[{uri:SONG},{uri:LINK}]}, tracks:[track,null], entries:[{kind:'track',uri:SONG},{kind:'link',uri:LINK}], local_tracks:[]};
const pf = {order:{order:['library'],services:[{id:'library',name:'Your library',connected:true}]},own_order:false,items:[{index:0,uri:SONG,title:track.title,artist:'Sefa, Dr. Peacock',track_id:1,plays_from:{service:'library',label:'Your library'},sources:[]},{index:1,uri:LINK,plays_from:null,sources:[]}]};
// GET /v1/item/<kind>/<id>: a library song, album or artist page in the node's shape
// (crates/pm-api/src/item.rs library_song, library_album, library_artist) built from the fixture's
// songs, so those pages draw (the page reads its target, and artist/album are { name, id } links).
// 'lib-<n>' is song, album or artist n (none there: 404, as the node says). Another id (mb-…,
// deezer-…) is a catalogue page the fixture can't reach (a node would ask the service): a 404 in
// the same shape, never a library page in its place. No About facts (the page has no About section, only its hidden place at the foot).
// A test that needs another page passes its own answer in opts.fixtures.
const cover = (t) => (t.album ? `/v1/library/albums/${t.album.id}/cover` : null);
const itemRow = (t) => ({no:t.track_no??null,title:t.title,artist:(t.artists||[]).map((a)=>a.name).join(', ')||null,duration_ms:t.duration_ms??null,uri:null,track_id:t.id,page:null,image:cover(t),id:`lib-${t.id}`,artist_id:t.artists?.[0]?`lib-${t.artists[0].id}`:null,album:t.album?.title??null,album_id:t.album?`lib-${t.album.id}`:null,versions:[]});
function itemPage(kind, id, tracks) {
 if(!['song','album','artist'].includes(kind)) return {error:'kind is song, album or artist',status:400};
 const n=/^lib-\d+$/.test(id)?Number(id.slice(4)):null;
 const none={error:`no ${kind==='song'?'track':kind} ${n??id}`,status:404};
 if(n==null) return none;
 const base={kind,id,source:'Your library',source_id:'library',connect:[],reading:false,pending:false,versions:[],also_on:[],open_on:[],related:[],concerts:[],albums:[],about:[]};
 const from=(k,i,label='Your library')=>({service:'library',kind:k,id:String(i),label,url:null,mode:'full'});
 const total=(ts)=>ts.reduce((s,t)=>s+(t.duration_ms||0),0);
 const albumOf=(t)=>t.album?[{id:`lib-${t.album.id}`,title:t.album.title,year:t.album.year??null,image:cover(t),page:''}]:[];
 if(kind==='song'){
  const t=tracks.find((x)=>x.id===n);if(!t) return none;const a=t.artists?.[0];
  return {...base,id:`lib-${t.id}`,uri:t.canonical_uri||null,song_uri:t.canonical_uri||null,target:{track_id:t.id},title:t.title,artist:a?{name:(t.artists||[]).map((x)=>x.name).join(', '),id:`lib-${a.id}`}:null,album:t.album?{name:t.album.title,id:`lib-${t.album.id}`}:null,year:t.album?.year??null,image:cover(t),plays_from:from('track',t.id),total_ms:t.duration_ms||0,songs:[itemRow(t)],albums:albumOf(t),versions:[{page:null,source:'library',id:String(t.id),label:'Your library',url:'',uri:`track:${t.id}`}],section:'tracks'};
 }
 if(kind==='album'){
  const ts=tracks.filter((t)=>t.album&&t.album.id===n);if(!ts.length) return none;const al=ts[0].album;const a=ts[0].artists?.[0];
  return {...base,id:`lib-${al.id}`,target:{album_id:al.id},title:al.title,artist:a?{name:a.name,id:`lib-${a.id}`}:null,album:null,year:al.year??null,image:cover(ts[0]),plays_from:from('album',al.id),total_ms:total(ts),songs:ts.map(itemRow),section:'albums'};
 }
 if(kind==='artist'){
  const ts=tracks.filter((t)=>(t.artists||[]).some((a)=>a.id===n));if(!ts.length) return none;const ar=ts[0].artists.find((a)=>a.id===n);
  const albums=[...new Map(ts.flatMap(albumOf).map((a)=>[a.id,a])).values()];
  return {...base,id:`lib-${ar.id}`,target:{artist_id:ar.id},title:ar.name,artist:null,album:null,year:null,plays_from:from('artist',ar.id),total_ms:total(ts.slice(0,10)),songs:ts.slice(0,10).map(itemRow),albums,section:'artists'};
 }
}
const json = (r, body, status=200) => r.fulfill({status,contentType:'application/json',body:JSON.stringify(body)});
async function create(browser, width, opts={}) {
 const state = {role:'owner',token:'fixture-owner',offline:false,collections:[],favs:[],hits:[],errors:[],unhandled:[],...opts};
 // opts.mobile: a phone's viewport rules (a page wider than the screen is zoomed out to fit, as Android's WebView does)
 // opts.scale: a desktop screen's device pixels per CSS px (a 2x laptop screen: the mouse lands on half pixels)
 const context = await browser.newContext({viewport:{width,height:900},locale:'en-US',reducedMotion:'reduce',hasTouch:!!opts.touch||!!opts.mobile,...(opts.mobile?{isMobile:true,deviceScaleFactor:2.625}:opts.scale?{deviceScaleFactor:opts.scale}:{})});
 await context.addInitScript(([realAudio,token,zone]) => {localStorage.setItem('pm.token',token);localStorage.setItem('pm.zone',zone);if(realAudio){document.addEventListener('play',e=>{if(e.target instanceof HTMLMediaElement){e.target.muted=true;e.target.volume=0;}},true);}else{HTMLMediaElement.prototype.play=async function(){};HTMLMediaElement.prototype.pause=function(){};}},[!!state.realAudio,state.token,state.zone||'browser']);
 await context.route('**/*', async r => {
  const req=r.request(), u=new URL(req.url()), p=u.pathname;
  if(u.origin!==ORIGIN) return r.abort();
  if(!p.startsWith('/v1/')) {const f=path.join(STATIC, p==='/'||!path.extname(p)?'index.html':p.slice(1));return ui.has(STATIC,path.basename(f))&&(fs.existsSync(f)||p==='/app.js')?r.fulfill({body:ui.read(STATIC,path.basename(f)),contentType:({'.html':'text/html','.js':'text/javascript','.css':'text/css','.svg':'image/svg+xml'})[path.extname(f)]||'application/octet-stream'}):r.fulfill({status:404,body:''});}
  const v=p.slice(3), m=req.method();state.hits.push({v,m,q:u.search,body:req.postData(),at:Date.now()});
  // opts.delay: ms (or (path, method) => ms) before the answer, for loading states (ui-busy.js)
  const wait=typeof state.delay==='function'?state.delay(v,m):state.delay; if(wait) await new Promise(res=>setTimeout(res,wait));
  if(state.fixtures?.[v]) return json(r,state.fixtures[v].body,state.fixtures[v].status||200);
  if(v==='/info') return json(r,{name:'Fixture node',offline:state.offline,library:{tracks:state.tracks?.length??1,albums:1,artists:2,files:1,missing_files:0}});
  if(v==='/me') return json(r,{role:state.role,key:null,zones:null});
  if(v==='/pubky') return json(r,{user:null});
  if(['/outputs','/zones','/sources','/presence','/smart','/library/albums','/library/artists','/library/genres'].includes(v)) return json(r,[]);
  if(v==='/playlists') {if(m==='POST'){state.collections=[summary];return json(r,detail);}return json(r,state.collections);}
  if(v==='/playlists/c1') return json(r,detail);
  if(v==='/playlists/c1/play-from') return json(r,pf);
  if(v==='/bookmarks'||v==='/favourites') {
   if(m==='PUT') {state.favs=[SONG];return json(r,{uri:SONG});}
   if(m==='DELETE') {state.favs=[];return json(r,{});}
   return json(r,{uris:state.favs,tracks:state.favs.length?[track]:[],albums:[],artists:[],other:[]});
  }
  if(v==='/library/tracks') return json(r,{tracks:state.tracks||[track],next:null});
  if(v==='/library/tracks/1') return json(r,(state.tracks||[track])[0]);
  if(v==='/library/albums/7') return json(r,{...track.album,tracks:state.tracks||[track],artists});
  if(v.startsWith('/item/')) {const [,,kind,id,more]=v.split('/');if(more==='extras') return json(r,{related:[],concerts:[],similar:[]});const pg=itemPage(kind,decodeURIComponent(id),state.tracks||[track]);return pg.status?json(r,{error:pg.error},pg.status):json(r,pg);}
  if(v==='/node/settings') return json(r,{contact:'',discovery:false,token_replaceable:true});
  if(v==='/me/prefs') return json(r,{});
  if(v==='/play-order') return json(r,{order:['library'],services:[{id:'library',name:'Your library',connected:true}]});
  if(v==='/cards') return json(r,{uri:u.searchParams.get('uri'),status:'found',kind:'Article',name:'Liner notes',site:'example.com',description:'How it was made'});
  if(v==='/storage') return json(r,{homeserver:{mode:'none'}});
  if(v==='/smart/preview') return json(r,{tracks:[],matched:0});
  if(v==='/offline/downloads') return json(r,{tracks:state.tracks||[track],playlists:[{id:'c1',name:summary.name,track_ids:[1],total:1}],favourites:{track_ids:[1]}});
  if(/^\/library\/albums\/\d+\/cover$/.test(v)) return r.fulfill({status:404,body:''}); // no artwork kept
  // pm-offline's GET /v1/offline answer (crates/pm-offline/src/server.rs).
  if(v==='/offline') return json(r,{offline:true,device_id:'fixture-device',synced_at:null,bytes:0,queued:0,tracks:state.tracks||[track],albums:[{...track.album,track_ids:[1],cover:false}],playlists:[{id:'c1',name:summary.name,track_ids:[1],total:1}],favourites:{track_ids:[1]}});
  if(v==='/search') return r.fulfill({contentType:'application/x-ndjson',body:JSON.stringify({source:'library',status:'ok',hits:state.searchHits||[],counts:[]})+'\n'});
  if(v==='/discover/play') return json(r,{uri:SONG,title:track.title,artist:'Sefa, Dr. Peacock',track_id:1});
  state.unhandled.push(v);return json(r,{error:'Fixture endpoint unavailable'},404);
 });
 await context.routeWebSocket(/\/v1\/events/, ws=>{ws.onMessage(m=>{let j;try{j=JSON.parse(m);}catch{return;}if(j.type==='ping')ws.send(JSON.stringify({type:'pong',id:j.id}));});ws.send(JSON.stringify({type:'snapshot',seq:0,boot:'fixture-boot',library_rev:0,zones:[]}));});
 const page=await context.newPage();page.on('pageerror',e=>state.errors.push(e.message));
 page.setDefaultTimeout(7000);
 return {context,page,state};
}
const go = async (page,p) => {await page.goto(ORIGIN+p);};
async function cleanCopy(page) {
 const text = await page.evaluate(() => {
  const attrs=[...document.querySelectorAll('[aria-label],[title],[placeholder]')].filter(e=>e.getClientRects().length).flatMap(e=>['aria-label','title','placeholder'].map(a=>e.getAttribute(a)||''));
  return document.body.innerText+'\n'+attrs.join('\n');
 });
 assert.doesNotMatch(text,/\bundefined\b|\[object Object\]/,'rendered text and accessible labels');
 const overflow=await page.evaluate(()=>document.documentElement.scrollWidth-document.documentElement.clientWidth);assert.ok(overflow<=0,`no horizontal overflow: ${overflow}`);
}
module.exports={create,go,cleanCopy,itemPage,ORIGIN,track,artists,detail,summary,SONG,LINK};
