// A/B: raw query vs the production `effective_query` (raw + " after:YYYY-MM-DD").
// Measures Bing RSS item counts and top-5 titles for each variant.
import { writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const dir = join(here, 'afterprobe');
mkdirSync(dir, { recursive: true });

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
const H = { Accept: 'text/html,application/rss+xml,application/xml;q=0.9,*/*;q=0.8', 'Accept-Language': 'zh-CN,zh;q=0.9,en;q=0.7', 'User-Agent': UA };

// Today is 2026-10-10; recency inferral for "今天..." gives 2 -> after:2026-10-08
const cases = [
  { q: '今天上海天气怎么样', eff: '今天上海天气怎么样 after:2026-10-08' },
  { q: '今天有什么AI新闻', eff: '今天有什么AI新闻 after:2026-10-08' },
  { q: '最新DeepSeek模型进展', eff: '最新DeepSeek模型进展 after:2026-10-03' },
];

async function get(url) {
  const c = new AbortController();
  const t = setTimeout(() => c.abort(), 12000);
  try {
    const r = await fetch(url, { headers: H, redirect: 'follow', signal: c.signal });
    return { status: r.status, body: await r.text() };
  } catch (e) {
    return { status: 0, body: '', err: `${e?.name}` };
  } finally { clearTimeout(t); }
}

const decode = (s) => s.replace(/<!\[CDATA\[([\s\S]*?)\]\]>/g, '$1')
  .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"')
  .replace(/&#39;/g, "'").replace(/&apos;/g, "'").replace(/&amp;/g, '&')
  .replace(/&#(\d+);/g, (_, d) => String.fromCharCode(Number(d)));

const tag = (item, t) => {
  const o = item.indexOf(`<${t}>`);
  if (o < 0) return null;
  const s = o + t.length + 2;
  const e = item.indexOf(`</${t}>`, s);
  return e < 0 ? null : item.slice(s, e);
};

for (const c of cases) {
  console.log(`\n${'='.repeat(76)}\nCASE: ${c.q}\n  raw => ${c.q}\n  eff => ${c.eff}\n${'='.repeat(76)}`);
  for (const [label, q] of [['RAW', c.q], ['EFF(after:)', c.eff]]) {
    const rss = await get(`https://cn.bing.com/search?format=rss&q=${encodeURIComponent(q)}`);
    const items = rss.body.split('<item>').slice(1);
    console.log(`\n  [${label}] bing-rss status=${rss.status}${rss.err ? ' err=' + rss.err : ''} items=${items.length}`);
    items.slice(0, 5).forEach((it, i) => {
      console.log(`      ${i + 1}. ${decode(tag(it, 'title') ?? '').slice(0, 90)}`);
      console.log(`         ${decode(tag(it, 'link') ?? '').slice(0, 90)}`);
    });
    writeFileSync(join(dir, `${label}-${c.q.slice(0, 12).replace(/[^\w\u4e00-\u9fa5]/g, '_')}.xml`), rss.body, 'utf8');
  }
}