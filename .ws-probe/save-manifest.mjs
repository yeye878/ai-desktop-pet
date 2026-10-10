// Capture fixtures keyed by the *effective* query production actually sends,
// plus the entity-pollution path (Bing HTML used alone when RSS yields nothing).
import { writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const dir = join(here, 'fixtures');
mkdirSync(dir, { recursive: true });

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
const H = { Accept: 'text/html,application/rss+xml,application/xml;q=0.9,*/*;q=0.8', 'Accept-Language': 'zh-CN,zh;q=0.9,en;q=0.7', 'User-Agent': UA };

// base query, recency_days production infers, and the resulting effective query
const cases = [
  { id: 'weather', base: '今天上海天气怎么样', recency: 2, eff: '今天上海天气怎么样 after:2026-10-08' },
  { id: 'ainews', base: '今天有什么AI新闻', recency: 2, eff: '今天有什么AI新闻 after:2026-10-08' },
  { id: 'finance', base: '2026年10月 央行 逆回购 公开市场操作', recency: null, eff: '2026年10月 央行 逆回购 公开市场操作' },
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
  const q = encodeURIComponent(c.eff);
  const rss = await get(`https://cn.bing.com/search?format=rss&q=${q}`);
  const html = await get(`https://cn.bing.com/search?q=${q}`);
  writeFileSync(join(dir, `${c.id}-eff-rss.xml`), rss, 'utf8');
  writeFileSync(join(dir, `${c.id}-eff-html.html`), html, 'utf8');
  manifest.push({ id: c.id, base: c.base, recency: c.recency, eff: c.eff, rss: `${c.id}-eff-rss.xml`, html: `${c.id}-eff-html.html` });
  console.log(`${c.id}: eff="${c.eff}" rss=${rss.length}B html=${html.length}B`);
}
writeFileSync(join(dir, 'manifest.json'), JSON.stringify(manifest, null, 2), 'utf8');
console.log('manifest written');