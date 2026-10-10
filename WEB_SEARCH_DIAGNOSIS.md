# web_search 工具诊断报告

> 结论：模型"拿不到有效信息"不是模型的问题，而是**搜索管线自己把有效结果洗掉了**。
> 下面每一条都有实测复现，不是推测。

- 诊断时间：2026-10-10
- 对象：`src-tauri/src/direct_api/tools.rs` 中的 `web_search`（默认 `search_provider = "bing"`，见 `api_client.rs:46`）
- 复现材料：`.ws-probe/`（抓取脚本 + 原始响应 fixture + Rust 探针测试）

---

## 0. 复现方式

```powershell
# 1) 抓真实的 Bing 响应（脚本按生产代码同样的 URL/UA/参数发出请求）
node .ws-probe\save-manifest.mjs

# 2) 把真实响应喂进"真实的生产管线"，打印模型实际收到的文本
cd src-tauri
cargo test --lib probe_dump_production_search_output -- --nocapture
```

第 2 步调用的是**生产代码本体**（`parse_bing_rss` / `parse_bing_html` / `rank_search_results` /
`format_search_results`），只把网络层换成了离线 fixture，因此输出与线上逐字一致。

> 说明：按"不改代码"的要求，那个探针测试**已从源码中移除**，`tools.rs` 与原状逐字节一致（190723 字节）。
> 探针源码保存在 `.ws-probe/probe-test.rs.txt`，按其头部说明贴回 `mod tests` 末尾即可重跑第 2 步。

---

## 1. 【最严重】给 Bing 加 `after:` 日期算子，直接把查询打废

`effective_query_for_date()`（`tools.rs:201-220`）在 `recency_days` 存在时追加 `after:YYYY-MM-DD`。
而 `infer_recency_days()`（`tools.rs:85-141`）只要查询里出现
`今天/今日/日报/快讯/最近/最新/近期/新闻/动态/现状/进展/更新/latest/recent/news...`
就自动推断出 2~7 天，于是自动追加该算子。

**cn.bing.com 不理解 `after:`**——代码自己的注释也承认了（`tools.rs:2369`）。实测后果：

| 差集 | 发给 Bing 的查询 | 返回的前 5 条 |
|---|---|---|
| 原始查询 | `今天上海天气怎么样` | 上海天气网、中国天气网、上海市气象局、7k7p…（**主题正确**） |
| 生产实际发出 | `今天上海天气怎么样 after:2026-10-08` | 日历网、今日黄历、天长天气网、"历史上的今天"、百度百科"今天（诗刊）"（**全废**） |

```
@@ weather | 用户查询: 今天上海天气怎么样
@@ 实际发出: 今天上海天气怎么样 after:2026-10-08 | recency_days=Some(2)
[1] 今日黄历宜忌查询,今日老黄历,今天是什么日子老黄历_天天黄历
    摘要: 今天是第 41 周,距离2026年结束还有 11 周 今天是 世界空间周、辛亥革命纪念日...
[2] 【天长今天天气预报】天长天气预报24小时详情_天长天气网   <- 注意是"天长"，不是"上海"
[3] 历史上的今天 - 所有日期-每日历史事件、名人诞辰与忌日查询
[4] 今天（北岛、芒克等创办诗刊）_百度百科
[5] 今天农历是多少,今天农历是几号-天天黄历
```

`今天有什么AI新闻`（同样 recency=2）→ 结果全是黄历 / 百度百科"今天"词条 / 历史上的今天，
**一条 AI 新闻都没有**。

讽刺的是 `lib.rs:1006` 的系统提示在主动让模型传 `recency_days`：
> "对新闻、日报、快讯、最近动态这类任务，优先调用 web_search 并传入 recency_days"

也就是说：**产品鼓励的用法，正好触发把这个工具打废的分支**。新闻/时效类查询是 agent 最高频的场景，
这解释了"经常返回很杂的信息"。

另注：只有东方财富那条路径会剥掉 `after:`（`eastmoney_keyword()`，`tools.rs:2889`），
Bing/DDG 三条路径都吃生查询（`search_with_provider`，`tools.rs:2832`），没有按引擎适配。

## 2. 【严重】排序没有相关性下限，跑题的也能进最终 5 条

`rank_search_results()`（`tools.rs:3061-3104`）：

```rust
let positive_count = scored.iter().filter(|(score, _, _)| *score > 0).count();
if positive_count >= 3 { scored.retain(|(score, _, _)| *score > 0); }
```

只要 `score > 0` 就留下，而拿正分极其容易。`query_terms()`（`tools.rs:3235-3277`）会把中文
拆成**所有 2 字滑窗**（`add_cjk_windows`，`tools.rs:3299-3310`）外加单字，于是
`2026年10月 央行 逆回购 公开市场操作` 产生 `2026`、`10`、`年`、`月`、`公开`、`开市`、`市场`… 等 20+ 个词，
其中 `2026`/`年`/`月` 在任何网页上都命中。

实测（查询：`2026年10月 央行 逆回购 公开市场操作`，**10 个候选全部通过，一条都没筛掉**）：

```
@@ 解析: rss=10 html=10 合并后=10->10
---------------- C. 生产合并路径 (RSS + HTML): 候选->ranked=10 展示=5
[1] 2026年国庆假期盘点_新闻频道_央视网
[2] 2026年诺贝尔奖即将揭晓
[3] 2026年国际足联世界杯_百度百科
[4] 国务院办公厅关于2026年部分节假日安排的通知
[5] 2026年全国两会_中国政府网
```

**没有任何一条提到央行、逆回购或公开市场操作**。模型拿到这 5 条"很杂的信息"，自然无法作答——
这正是用户描述的现象。（真实链路里该查询会先走东方财富，但东财空/失败时就会掉进这个坑。）

`is_usable_search_result()`（`tools.rs:3356`）只挡 bing/ddg 自身域名和 captcha；
`is_low_signal_domain()`（`tools.rs:3388`）只列了 6 个社交站。都没有"是否命中查询主题"的判定。

## 3. 【中】HTML 实体解码不全，原始实体泄漏进模型上下文

`decode_html_entities()`（`tools.rs:3647-3659`）只处理 `&amp; &lt; &gt; &quot; &apos; &#x27; &#x2F; &#39; &#34; &#160; &nbsp;`。

Bing HTML 的每条摘要都以 `日期&ensp;&#0183;&ensp;正文` 开头，而：

- `&ensp;` 未处理；
- `&#0183;` 是**带前导零**的数字实体，任何朴素 `&#NNN;` 实现都会漏。

实测模型看到的摘要原文（B 路径，即 RSS 空/失败时回退 HTML）：

```
摘要: 14 小时之前&ensp;&#0183;&ensp;今天是第 41 周,距离2026年结束还有 11 周...
摘要: 1978年12月23日&ensp;&#0183;&ensp;一九九零年春十多位作家在挪威 奥斯陆 聚会...
```

正常路径下 RSS 先返回 10 条、HTML 的相同 URL 被去重，所以泄漏被掩盖；
但 **RSS 少一条就会混进一条带实体的摘要**（同一份列表里有干净有脏的 → 正符合"很杂"），
RSS 失败时则 5 条全是实体。

## 4. 【中】第三个搜索源实际是死的，等于只有 Bing 一个引擎

`web_search_with_options()`（`tools.rs:2384-2398`）的循环：

```rust
if source_names.len() >= 2 || all_results.len() >= SEARCH_CANDIDATE_LIMIT { break; }
```

`BingRss` 和 `BingHtml` **是同一个引擎**（都是 `https://cn.bing.com/search`，`tools.rs:65`），
所以只要 RSS 有结果（实测 8 种查询全部返回 9~10 条），第二轮 HTML 跑完
`source_names.len() == 2` 就 break —— **DuckDuckGo 永远不会被执行**。

而 DuckDuckGo 在本机网络上本来就是不通的：

```
=== DNS ===
  html.duckduckgo.com -> 31.13.87.19, 2a03:2880:f129:83:face:b00c:0:25de   <- 被解析到 Meta 的 IP（DNS 污染）
=== DuckDuckGo HTML reachability ===
  FAILED: TypeError: fetch failed cause=UND_ERR_CONNECT_TIMEOUT
```

结论：`provider_order` 看似有三级兜底，实际是**单引擎**；Bing 对这个查询不行时，没有任何替补。

## 5. 【中】RSS 的 `pubDate` 被丢弃，时效信号两边不对齐

`parse_bing_rss()`（`tools.rs:3535-3563`）只取 `title`/`link`/`description`，
`<pubDate>周六, 10 10月 2026 06:24:00 GMT</pubDate>` 这个**真时间戳直接扔掉**。

而 `score_result_freshness()`（`tools.rs:3160-3201`）只能靠字符串猜："小时前"/"分钟前"/今天的 ISO 日期
——这些标记只存在于 Bing **HTML** 摘要里。于是同一个查询里，RSS 来源的结果永远拿不到新鲜度加分，
HTML 来源的能拿到，两条来源被用不同尺子打分。

## 6. 【次要】搜索 client 没有 `no_proxy()`，会静默继承环境代理

`web_search_with_options()`（`tools.rs:2356-2361`）构建 client 时没调 `.no_proxy()`，
而 **reqwest 0.11 默认从环境变量读代理**（已在
`~/.cargo/registry/.../reqwest-0.11.27/src/proxy.rs:897 get_from_environment()` 确认：
`HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`）。

本会话环境就有 `HTTPS_PROXY=http://127.0.0.1:7890` 且该端口当时是关的 —— 那种情况下搜索会**整体失败**。
更要紧的是代理若返回 HTTP 200 的登录/跳转页，会被当成搜索结果解析。

项目里其他出网 client 都显式关掉了代理：
`collaboration/mod.rs:438` 与 `mcp_permission.rs:432` 都用 `.no_proxy()`，
只有搜索这条没有（`lib.rs:300-330` 的天气逻辑甚至专门做了代理重试）——不一致。

---

## 7. 建议修法（按性价比排序）

1. **不再给 Bing 追加 `after:`**。要么彻底去掉该算子、把时效交给本地排序（`recency_days` 本来就只在
   `score_result_freshness` 里用），要么按引擎适配：Bing 用 `filters=ex1:"ez1"` 之类或干脆不用，
   东财保持现状。这一条就能救回绝大多数新闻/时效查询。 → **已修，见 §10**
2. **给排序加相关性下限**：要求"至少命中一个长度 ≥2 的实词"（过滤掉 `2026`/`年`/`月`/单字滑窗），
   命中数不足时**返回明确提示"结果与查询不相关"**，而不是硬塞 5 条跑题结果让模型去瞎猜。
   → **已修（按实词数量自适应阈值），见 §10**
3. **补全实体解码**：加 `&ensp; &emsp; &hellip; &middot; &ndash; &mdash; &lsquo; &rsquo; &ldquo; &rdquo;`
   以及**支持前导零/十六进制的通用 `&#...;` 解码**。 → **已修，见 §10**
4. **换/加一个真正可用的第二引擎**（DDG 在本网络不通）。可选：Bing 国际站、Brave、SearXNG 实例、
   或接一个带摘要的搜索 API。否则任何单引擎抖动都是全量噪声。 ← **未做（需要产品选型）**
5. **RSS 时间戳纳入排序**：解析 `pubDate` 填进 `SearchResult`，与 HTML 摘要统一比较。
   ← **未做**（要扩 `SearchResult` 字段，会牵动多处结构体字面量，建议和 4 一起做）
6. **搜索 client 加 `.no_proxy()`**，或显式读一个专用代理配置变量，并让失败原因可见
   （现在只剩"所有搜索源都未返回可用结果"，无法区分"被代理拦了"和"真的没结果"）。
   ← **未做（产品决策：国内用户可能恰恰需要代理才能出网，不能无脑关）**

## 8. 附带发现

- `format_search_results()` 的固定"提示"行让模型"优先只读取标题/摘要直接匹配任务的 1-2 个网页"，
  但结果本身跑题时模型无从判断，反而可能去读跑题页面，浪费轮次。
- `SEARCH_CANDIDATE_LIMIT = 12`、单源 `truncate(12)`、默认 `max_results = 5`：
  去重后进排序的候选通常只有 ~10 条，**样本池太小**，一旦 Bing 前 10 条跑题就必然全军覆没。
- `无描述` 只扣 4 分（`tools.rs:3146`），几乎不构成惩罚。

## 9. 临时产物（可删，都在工作树里，未提交）

- `.ws-probe/`：`fetch-raw.mjs`、`save-fixtures.mjs`、`save-manifest.mjs`、`save-fixtures-fixed.mjs`、
  `after-probe.mjs`、`coverage-probe.mjs`、`net-probe.mjs`、`raw/`、`fixtures/`（抓取的原始响应）
- `.ws-probe/probe-test.rs.txt`：诊断/验证探针源码 + 贴回方法（**不在源码树里**，用完即删，
  这样 `tools.rs` 始终只有真实修复，不掺探针）
- 本报告 `WEB_SEARCH_DIAGNOSIS.md`

---

## 10. 修复记录（2026-10-10）

只改了 `src-tauri/src/direct_api/tools.rs` 一个文件，落地 §7 的第 1~3 条。

### 改了什么

1. **`effective_query()`（原 `effective_query_for_date()`）**：删掉 `after:` 注入，只保留 `site:`。
   时效性完全交给 `score_result_freshness` 与东方财富的发布时间过滤。函数不再需要 `today` 参数。
2. **`rank_search_results()` 加相关性下限**：新增 `RankedSearch { results, off_topic }` 返回值、
   `is_content_term()`（排除纯数字与单字）、`content_term_hits()`、`min_content_hits()`
   （实词 ≥4 个时要求命中 ≥2 个不同实词，否则退化为 1）。
   命中不足的候选计入 `off_topic` 并丢弃；URL 命中不再算相关性依据。
   - 全部候选跑题时，`web_search_with_options()` **返回明确的错误说明**（"候选全部与查询主题无关…请换更短更具体的关键词"），
     而不是把噪声塞给模型；有结果但丢弃过跑题候选时，在结果末尾追加一行提示。
3. **`decode_html_entities()` + 新增 `decode_numeric_entities()`**：补齐 `&ensp; &emsp; &thinsp;`
   `&hellip; &middot; &ndash; &mdash; &lsquo; &rsquo; &ldquo; &rdquo; &times; &copy; &reg; &deg;`，
   并支持 `&#0183;`（前导零十进制）与 `&#x27;`（十六进制）等全部数字实体写法；
   `&amp;` 改为最后解码，NBSP 归一成普通空格。

### 验证（真实数据，非模拟）

`.ws-probe/fixtures/` 里同时存着"修复前生产实际发出的查询（带 `after:`）"和"修复后的查询"两份真实响应，
探针把两份分别喂进生产管线对比（探针源码见 `.ws-probe/probe-test.rs.txt`）：

| 查询 | 修复前 | 修复后 |
|---|---|---|
| `今天上海天气怎么样` | 10 条候选只有 1 条过线，且是**「天长」**天气网（城市都不对） | 10 条全过线 → **5 条上海天气结果**（Raincast、7k7p、上海天气网、中国天气网、随心天气） |
| `今天有什么AI新闻` | 0 条过线（全是黄历/百度百科「今天」/历史上的今天）→ 报跑题 | 9 条过线 → **5 条真实 AI 快讯**（AI情报台、今日AI新闻、AIKnow…） |
| `2026年10月 央行 逆回购 公开市场操作` | 0 条过线，10 条跑题（国庆盘点/世界杯/诺贝尔奖/两会） | 仍 0 条过线，10 条跑题 → **如实报错并给出改写建议，不再喂噪声** |

> 第三条属于"Bing 对这个查询本身就不行"，修复的作用是**不再把跑题结果伪装成答案**；
> 线上该查询会先走东方财富（按发布时间过滤、不受 `after:` 影响），那条路未被改动。

新增回归测试 3 个（用实测内容做用例）：`effective_query_never_injects_unsupported_after_operator`、
`relevance_floor_drops_off_topic_candidates`、`relevance_floor_keeps_on_topic_candidates`、
`decodes_bing_html_entities_including_zero_padded_numeric`（共 4 个）。

测试：`cargo test --lib direct_api::tools` → **37 passed / 0 failed**。
（全量 `cargo test --lib` 有 3 个 `tts::edge_tts` 用例失败，原因是本机沙箱下 TLS 取不到凭证 +
`speech.platform.bing.com` 走不通，与本改动无关——它们访问的是 TTS 实时接口，
顺带印证了 §6 的代理问题：报错就是 `tcp connect error: 目标计算机积极拒绝`，
即 reqwest 用了环境里的 `HTTPS_PROXY`。）

### 仍未做（§7 的 4~6）

第二引擎选型、`pubDate` 纳入排序、代理策略，都需要产品决策，未擅自改动。