// The control audit needs the same search inventory on every shard and reload.
// Public services change their ranked results: CI 42648925379 saw an album on
// shard 3 that its owner, shard 1, never received. Keep those controls in a fixed
// response; real service/search integration remains covered by ui-search.js.
const fixture = require('./fixtures/ui-search-queen.json');
async function install(context) {
  await context.route(/\/v1\/search\?/, (route) => {
    const u = new URL(route.request().url());
    if (u.searchParams.get('q') !== fixture.q) return route.fallback();
    const scope = u.searchParams.get('in') || 'everywhere';
    const answers = scope === 'everywhere' ? fixture.answers : fixture.answers.filter((a) => a.source === scope);
    // A connected source absent from this capture still has a valid empty answer.
    const selected = answers.length ? answers : [{ source: scope, status: 'ok', hits: [], counts: [] }];
    return route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': selected.map((a) => a.source).join(',') }, body: selected.map((a) => JSON.stringify(a)).join('\n') + '\n' });
  });
  await context.route(/\/v1\/search\/tags\?/, (route) => {
    if (new URL(route.request().url()).searchParams.get('prefix') !== fixture.q) return route.fallback();
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ mine: [], others: [] }) });
  });
}
async function settle(page, timeout = 15000) {
  const u = new URL(page.url());
  const q = u.searchParams.get('q');
  const scope = u.searchParams.get('in') || 'everywhere';
  if (u.pathname !== '/search' || !q) return;
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    // Poll with evaluate because the node's CSP disallows waitForFunction.
    if (await page.evaluate(([query, source]) => window.pmLastSearch?.q === query && document.querySelector('#scopechips .on')?.dataset.scopeIn === source && document.querySelector('#sprog')?.hidden, [q, scope])) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`Search did not finish before the control inventory: ${q}`);
}
module.exports = { install, settle };
