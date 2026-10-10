// 1) Is the DuckDuckGo fallback even reachable from this network?
// 2) How often does the default first provider (Bing RSS) return zero items,
//    which is the condition that promotes the entity-polluted HTML parser?
import dns from 'node:dns/promises';

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
const H = { Accept: 'text/html,application/rss+xml,application/xml;q=0.9,*/*;q=0.8', 'Accept-Language': 'zh-CN,zh;q=0.9,en;q=0.7', 'User-Agent': UA };

console.log('=== DNS ===');
for (const host of ['html.duckduckgo.com', 'duckduckgo.com', 'cn.bing.com']) {
  try {
    const a = await dns.lookup(host, { all: true });
    console.log(`  ${host} -> ${a.map((x) => x.address).join(', ')}`);
  } catch (e) {
    console.log(`  ${host} -> DNS FAIL ${e.code}`);
  }
}

console.log('\n=== DuckDuckGo HTML reachability ===');
try {
  const c = new AbortController();
  const t = setTimeout(() => c.abort(), 12000);
  const r = await fetch('https://html.duckduckgo.com/html/?q=test', { headers: H, signal: c.signal });
  const b = await r.text();
  clearTimeout(t);
  console.log(`  status=${r.status} bytes=${b.length} captcha=${b.includes('ddg-captcha') || b.includes('anomaly-modal')} anchors=${b.split('<a class="result__a" href="').length - 1}`);
} catch (e) {
  console.log(`  FAILED: ${e?.name}: ${e?.message} cause=${e?.cause?.code ?? e?.cause?.message ?? 'n/a'}`);
}

console.log('\n=== Bing RSS item counts across query shapes ===');
const queries = [
  '今天上海天气怎么样',
  'DeepSeek',
  'rust async trait object safety',
  '2026年10月10日 A股 收盘',
  '上海天气',
  'site:github.com reqwest proxy',
  '怎么用 Rust 写一个 TCP 服务器 教程',
  '雷军 最新演讲 全文',
];
for (const q of queries) {
  try {
    const c = new AbortController();
    const t = setTimeout(() => c.abort(), 12000);
    const r = await fetch(`https://cn.bing.com/search?format=rss&q=${encodeURIComponent(q)}`, { headers: H, redirect: 'follow', signal: c.signal });
    const b = await r.text();
    clearTimeout(t);
    const items = b.split('<item>').length - 1;
    console.log(`  items=${String(items).padStart(2)}  status=${r.status}  q="${q}"`);
  } catch (e) {
    console.log(`  ERROR ${e?.name} q="${q}"`);
  }
}