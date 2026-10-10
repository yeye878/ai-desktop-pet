// Build a before/after manifest: the "after:" variant production USED to send,
// next to the plain query the fixed code now sends.
import { writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const dir = join(here, 'fixtures');
mkdirSync(dir, { recursive: true });

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
const H = { Accept: 'text/html,application/rss+xml,application/xml;q=0.9,*/*;q=0.8', 'Accept-Language': 'zh-CN,zh;q=0.9,en;q=0.7', 'User-Agent': UA };

const cases = [
  { id: 'weather', base: '今天上海天气怎么样', recency: 2 },
  { id: 'ainews', base: '今天有什么AI新闻', recency: 2 },
  { id: 'finance', base: '2026年10月 央行 逆回购 公开市场操作', recency: null },
];

async function get(url) {
  const c = new AbortController();
  const t = setTimeout(() => c.abort(), 15000);
  try {
    const r = await fetch(url, { headers: H, redirect: 'follow', signal: c.signal });
    return await r.text();
  } finally { clearTimeout(t); }
}

const manifest = [];
for (const c of cases) {
  const q = encodeURIComponent(c.base);
  const rss = await get(`https://cn.bing.com/search?format=rss&q=${q}`);
  const html = await get(`https://cn.bing.com/search?q=${q}`);
  writeFileSync(join(dir, `${c.id}-raw-rss.xml`), rss, 'utf8');
  writeFileSync(join(dir, `${c.id}-raw-html.html`), html, 'utf8');
  const entry = { id: c.id, query: c.base, recency: c.recency, rss: `${c.id}-raw-rss.xml`, html: `${c.id}-raw-html.html` };
  manifest.push(entry);
  console.log(`${c.id}: query="${c.base}" rss=${rss.length}B html=${html.length}B`);
}
writeFileSync(join(dir, 'manifest.json'), JSON.stringify(manifest, null, 2), 'utf8');
console.log('manifest written');