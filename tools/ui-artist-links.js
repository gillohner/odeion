// Artist navigation uses structured identity, not punctuation in a display name.
// Isolated real candidate assets, fixture-only API, no node/media/provider calls.
const {chromium}=require(require.resolve('playwright',{paths:[process.cwd(),__dirname]}));
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const {create,go,track,detail,summary}=require('./ui-fixture');
const shots=path.resolve(process.argv[3]||path.join(process.env.TMPDIR||'.','artist-link-shots'));
const cases=[
 {name:'duet',credits:[{id:11,name:'Sefa'},{id:22,name:'Dr. Peacock'}]},
 {name:'single',credits:[{id:11,name:'Sefa'}]},
 {name:'comma',credits:[{id:33,name:'Earth, Wind & Fire'}]},
 {name:'missing-ids',credits:[{name:'Sefa'},{name:'Dr. Peacock'}]},
];
const badPlayback=h=>h.m!=='GET'&&(/\/library\/history|\/queue|\/transport|\/discover\/play/.test(h.v));
const target=(c)=>c.id!=null?'/artist/lib-'+c.id:'/search?q='+encodeURIComponent(c.name);
async function assertTarget(page,c){const u=new URL(page.url());if(c.id!=null)assert.equal(decodeURIComponent(u.pathname),'/artist/lib-'+c.id);else{assert.equal(u.pathname,'/search');assert.equal(u.searchParams.get('q'),c.name);}}
(async()=>{const browser=await chromium.launch();try{
 for(const width of [1280,820,390])for(const test of cases){
  const t={...track,artists:test.credits};const d={...detail,tracks:[t,null]};
  const {page,context,state}=await create(browser,width,{tracks:[t],collections:[summary],fixtures:{'/playlists/c1':{body:d}}});
  for(const screen of ['tracks','collection','player']){
   for(let step=0;step<test.credits.length*2;step++){const n=Math.floor(step/2);
    await go(page,screen==='collection'?'/playlist/c1':'/tracks');await page.locator(screen==='collection'?'#plsongs .srow':'.track').first().waitFor();
    let root=screen==='collection'?page.locator('#plsongs .srow .grow .sub').first():page.locator('.track .by').first();
    if(screen==='player'){await page.locator('.track [data-act="play"]').first().click();await page.locator('#np').click();root=page.locator('#nowplaying .np-titles .artist');await root.waitFor();}
    const expected=test.credits.map(c=>c.name);const names=await root.locator('a:not([data-salbum]):not([data-np-album]),button:not([data-salbum]):not([data-np-album])').allTextContents();
    assert.deepEqual(names,expected,`${width} ${test.name} ${screen}: independent exact credit names`);
    const before=state.hits.length,now=await page.evaluate(()=>pmPlayer.now());const link=root.locator('[data-artist-credit]').nth(n);
    assert.equal(await link.getAttribute('href'),target(test.credits[n]));
    // (a phone's song row is one tap target that plays: its links answer the keyboard, the ⋯ menu has the pages)
    const inert=await link.evaluate(a=>getComputedStyle(a).pointerEvents==='none');
    if(step%2||inert){await link.focus();await page.keyboard.press('Enter');}else await link.click();
    await page.waitForTimeout(150);await assertTarget(page,test.credits[n]);
    assert.ok(!state.hits.slice(before).some(badPlayback),'artist navigation never starts row playback');
    assert.equal((await page.evaluate(()=>pmPlayer.now())).index,now.index);
   }
  }
  await go(page,'/tracks');await page.locator('.track').first().waitFor();
  const links=page.locator('.track .by [data-artist-credit]');await links.first().waitFor();
  fs.mkdirSync(shots,{recursive:true});await page.screenshot({path:path.join(shots,`${width}-${test.name}-rows.png`)});
  // The album remains a distinct target, not part of the artist credit.
  const album=page.locator('.track [data-act="album"]').first();if(await album.isVisible()){await album.click();await page.waitForTimeout(150);assert.equal(new URL(page.url()).pathname,'/album/lib-7');}
  assert.deepEqual(state.errors,[]);console.log(`PASS ${width} ${test.name}: row/collection/player identity, click+keyboard, no playback`);await context.close();
 }
 {const {page,context,state}=await create(browser,390,{fixtures:{'/playlists/c1':{body:{...detail,tracks:[null,null]}}}});await go(page,'/playlist/c1');await page.waitForFunction(()=>document.querySelectorAll('#plsongs [data-artist-credit]').length===2);assert.deepEqual(await page.locator('#plsongs [data-artist-credit]').allTextContents(),['Sefa','Dr. Peacock']);assert.equal(await page.locator('#plsongs [data-artist-credit]').nth(1).getAttribute('href'),'/artist/lib-22');assert.equal(state.hits.filter(h=>h.v==='/library/tracks/1').length,1,'flattened legacy row coalesces one local metadata read');assert.deepEqual(state.errors,[]);console.log('PASS legacy flattened row: local structured hydration, distinct targets');await context.close();}
 {const pf={order:{order:['tidal'],services:[]},items:[{uri:'tidal:track:999',stream_uri:'tidal:track:999',artist:'Earth, Wind & Fire',title:'Unknown recording',plays_from:{service:'tidal'},sources:[]},{uri:'https://example.com/liner-notes',plays_from:null,sources:[]}]};const {page,context,state}=await create(browser,390,{fixtures:{'/playlists/c1':{body:{...detail,tracks:[null,null]}},'/playlists/c1/play-from':{body:pf}}});await go(page,'/playlist/c1');await page.locator('#plsongs [data-artist-credit]').waitFor();assert.deepEqual(await page.locator('#plsongs [data-artist-credit]').allTextContents(),['Earth, Wind & Fire']);assert.equal(await page.locator('#plsongs [data-artist-credit]').getAttribute('href'),'/search?q=Earth%2C%20Wind%20%26%20Fire');assert.equal(state.hits.filter(h=>h.v.startsWith('/library/tracks/')).length,0,'no invented library identity for a provider display name');console.log('PASS unstructured comma-bearing credit: one intact safe search link');assert.deepEqual(state.errors,[]);await context.close();}
 console.log('artist links: PASS');
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exit(1);});
