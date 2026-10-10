// Minimal reachability probe (no deps): direct vs proxy.
const targets = [
  'https://example.com',
  'https://cn.bing.com/search?format=rss&q=test',
];

for (const t of targets) {
  try {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), 15000);
    const res = await fetch(t, { signal: ctrl.signal, redirect: 'follow' });
    clearTimeout(timer);
    const text = await res.text();
    console.log(`OK   ${res.status} len=${text.length} ${t}`);
  } catch (e) {
    console.log(`FAIL ${t} :: ${e?.name}: ${e?.message}${e?.cause ? ' | cause=' + (e.cause.code ?? e.cause.message) : ''}`);
  }
}