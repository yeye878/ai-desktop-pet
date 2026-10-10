// Faithful reproduction of the Rust request layer in src-tauri/src/direct_api/tools.rs
// (search_with_provider), dumping raw bodies so parser/ranker behaviour can be inspected.
import { writeFileSync, mkdirSync } from 'node:fs';

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
const HEADERS = {
  'Accept': 'text/html,application/rss+xml,application/xml;q=0.9,*/*;q=0.8',
  'Accept-Language': 'zh-CN,zh;q=0.9,en;q=0.7',
  'User-Agent': UA,
};
const dir = new URL('./raw/', import.meta.url).pathname.replace(/^\//, '');
mkdirSync(dir, { recursive: true });

const queries = [
  '今天上海天气怎么样',
  'DeepSeek V4 模型 发布时间',
  '2026年10月 央行 逆回购 公开市场操作',
];

const endpoints = [
  { id: 'bing-rss', base: 'https://cn.bing.com/search', qs: (q) => ({ format: 'rss', q }) },
  { id: 'bing-html', base: 'https://cn.bing.com/search', qs: (q) => ({ q }) },
  { id: 'ddg-html', base: 'https://html.duckduckgo.com/html/', qs: (q) => ({ q }) },
];

async function get(url) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), 12000);
  try {
    const res = await fetch(url, { headers: HEADERS, redirect: 'follow', signal: ctrl.signal });
    const body = await res.text();
    return { status: res.status, body, finalUrl: res.url };
  } catch (e) {
    return { status: 0, body: '', error: `${e?.name}: ${e?.message}`, finalUrl: url };
  } finally {
    clearTimeout(timer);
  }
}

function tagText(item, tag) {
  const open = item.indexOf(`<${tag}>`);
  if (open < 0) {
    const selfClosed = item.match(new RegExp(`<${tag}[^>]*/>`));
    if (selfClosed) return null;
    const linked = item.match(new RegExp(`<${tag}[^>]*>([\\s\\S]*?)</${tag}>`, 'i'));
    return linked ? linked[1] : null;
  }
  const start = open + tag.length + 2;
  const end = item.indexOf(`</${tag}>`, start);
  return end < 0 ? null : item.slice(start, end);
}

const decode = (s) => s
  .replace(/<!\[CDATA\[([\s\S]*?)\]\]>/g, '$1')
  .replace(/&lt;/g, '<').replace(/&gt;/g, '>')
  .replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&apos;/g, "'")
  .replace(/&amp;/g, '&')
  .replace(/&#(\d+);/g, (_, d) => String.fromCharCode(Number(d)));

for (const q of queries) {
  console.log(`\n${'='.repeat(78)}\nQUERY: ${q}\n${'='.repeat(78)}`);
  for (const ep of endpoints) {
    const url = `${ep.base}?${new URLSearchParams(ep.qs(q)).toString()}`;
    const r = await get(url);
    const file = `${dir}${ep.id}--${encodeURIComponent(q).slice(0, 40)}.txt`;
    writeFileSync(file, r.body, 'utf8');
    console.log(`\n--- ${ep.id} | status=${r.status} bytes=${r.body.length}${r.error ? ' error=' + r.error : ''}`);
    console.log(`    final url: ${r.finalUrl}`);
    console.log(`    saved: ${file}`);

    if (ep.id === 'bing-rss') {
      const items = r.body.split('<item>').slice(1);
      const count = items.length;
      const withDesc = items.filter((i) => {
        const d = tagText(i, 'description');
        return d != null && d.trim().length > 0;
      }).length;
      console.log(`    <item> count=${count} withNonEmptyDescription=${withDesc}`);
      items.slice(0, 12).forEach((it, i) => {
        const t = tagText(it, 'title');
        const l = tagText(it, 'link');
        const d = tagText(it, 'description');
        console.log(`      [${i + 1}] T: ${decode(t ?? '').slice(0, 100)}`);
        console.log(`          U: ${decode(l ?? '').slice(0, 120)}`);
        console.log(`          D: ${decode(d ?? '(none)').replace(/\s+/g, ' ').slice(0, 160)}`);
        console.log(`          pubDate: ${tagText(it, 'pubDate') ?? '(none)'}`);
      });
    }

    if (ep.id === 'bing-html') {
      const blocks = r.body.split('<li class="b_algo"').slice(1);
      console.log(`    b_algo blocks=${blocks.length}`);
      blocks.slice(0, 8).forEach((b, i) => {
        const m = b.match(/<h2[^>]*>[\s\S]*?<a[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/i);
        const cap = b.match(/<p[^>]*class="[^"]*b_lineclamp[^"]*"[^>]*>([\s\S]*?)<\/p>/i)
          ?? b.match(/<p[^>]*>([\s\S]*?)<\/p>/i);
        const strip = (s) => decode(s ?? '').replace(/<[^>]*>/g, '').replace(/\s+/g, ' ').trim();
        console.log(`      [${i + 1}] T: ${strip(m?.[2]).slice(0, 100)}`);
        console.log(`          U: ${strip(m?.[1]).slice(0, 120)}`);
        console.log(`          S: ${strip(cap?.[1]).slice(0, 160)}`);
      });
    }

    if (ep.id === 'ddg-html') {
      const captcha = r.body.includes('ddg-captcha') || r.body.includes('anomaly-modal');
      const anchors = r.body.split('<a class="result__a" href="').slice(1);
      const snips = r.body.split('class="result__snippet"').slice(1);
      console.log(`    captcha/anomaly=${captcha} result__a anchors=${anchors.length} result__snippet=${snips.length}`);
      anchors.slice(0, 8).forEach((a, i) => {
        const href = a.slice(0, a.indexOf('"'));
        const gt = a.indexOf('>');
        const close = a.indexOf('</a>', gt);
        const title = decode(a.slice(gt + 1, close)).replace(/<[^>]*>/g, '').replace(/\s+/g, ' ').trim();
        console.log(`      [${i + 1}] T: ${title.slice(0, 100)}`);
        console.log(`          rawHref: ${href.slice(0, 140)}`);
      });
    }
  }
}
console.log('\ndone');