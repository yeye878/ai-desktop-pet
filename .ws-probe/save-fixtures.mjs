// Save stable-named fixtures for the Rust reproduction test.
import { writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const dir = join(here, 'fixtures');
mkdirSync(dir, { recursive: true });

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
const HEADERS = {
  Accept: 'text/html,application/rss+xml,application/xml;q=0.9,*/*;q=0.8',
  'Accept-Language': 'zh-CN,zh;q=0.9,en;q=0.7',
  'User-Agent': UA,
};

const cases = [
  { id: 'weather', query: '今天上海天气怎么样' },
  { id: 'finance', query: '2026年10月 央行 逆回购 公开市场操作' },
];

async function get(url) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), 12000);
  try {
    const res = await fetch(url, { headers: HEADERS, redirect: 'follow', signal: ctrl.signal });
    return await res.text();
  } finally {
    clearTimeout(timer);
  }
}

for (const c of cases) {
  const q = encodeURIComponent(c.query);
  const rss = await get(`https://cn.bing.com/search?format=rss&q=${q}`);
  writeFileSync(join(dir, `${c.id}-bing-rss.xml`), rss, 'utf8');
  const html = await get(`https://cn.bing.com/search?q=${q}`);
  writeFileSync(join(dir, `${c.id}-bing-html.html`), html, 'utf8');
  console.log(`${c.id}: rss=${rss.length}B html=${html.length}B`);
}
console.log('fixtures written to', dir);