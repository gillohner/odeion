// Isolated candidate assets; synthetic sharing writes only, no live node.
const {chromium}=require(require.resolve('playwright',{paths:[process.cwd(),__dirname]}));
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const {create,go,cleanCopy,detail}=require('./ui-fixture');
(async()=>{const browser=await chromium.launch();try{for(const width of [1280,820,390]){
 const {page,context,state}=await create(browser,width);let pub=false;const changes=[];
 await context.route('**/v1/pubky',r=>r.fulfill({json:{user:'fixture-key'}}));
 await context.route('**/v1/playlists/c1',r=>r.fulfill({json:{...detail,summary:{...detail.summary,visibility:pub?'public':'private',synced:!pub}}}));
 await context.route('**/v1/playlists/c1/visibility',r=>{pub=JSON.parse(r.request().postData()).public;changes.push(pub);return r.fulfill({json:{}});});
 await go(page,'/playlist/c1');await page.locator('#plplay').waitFor();
 assert.equal(await page.locator('#plorder,#plownorder,#plorderopen,#plpubky').count(),0,'no collection order editor or lower sharing block');
 await page.locator('.hero #plshare').waitFor();assert.deepEqual(changes,[],'opening never publishes');
 await page.locator('#plshare').click();await page.getByRole('button',{name:'Share',exact:true}).waitFor();assert.deepEqual(changes,[],'confirmation required');await page.keyboard.press('Escape');assert.deepEqual(changes,[]);
 await page.locator('#plshare').click();await page.getByRole('button',{name:'Share',exact:true}).click();await page.locator('.hero #plunshare').waitFor();assert.deepEqual(changes,[true]);
 await page.locator('.hero #plcopylink').waitFor();assert.match(await page.locator('.hero').innerText(),/next sync/);await cleanCopy(page);
 fs.mkdirSync(process.argv[3],{recursive:true});await page.screenshot({path:path.join(process.argv[3],`${width}-shared-top-card.png`),fullPage:true});
 await page.locator('#plunshare').click();await page.locator('.hero #plshare').waitFor();assert.deepEqual(changes,[true,false]);
 await go(page,'/settings/playfrom');await page.locator('#gorder .porder-row').first().waitFor();
 assert.ok(await page.locator('[data-set-panel="playfrom"]').count() || await page.locator('#gorder').count());
 await cleanCopy(page);await page.screenshot({path:path.join(process.argv[3],`${width}-personal-settings.png`),fullPage:true});assert.deepEqual(state.errors,[]);await context.close();
 const l=await create(browser,width,{role:'listener'});await go(l.page,'/settings');await l.page.locator('#gorder').waitFor();assert.match(await l.page.locator('#main').innerText(),/Play from/i);assert.deepEqual(l.state.errors,[]);await l.context.close();
 console.log(`PASS ${width}: settings for owner/listener, top-card private/public/pending, confirmation/cancel/unshare, no collection order/overflow`);
}}finally{await browser.close();}})().catch(e=>{console.error(e);process.exit(1);});
