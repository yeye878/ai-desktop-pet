const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const baseUrl = process.env.ORCH_BASE_URL || 'http://127.0.0.1:1443';
const output = path.join(process.env.TEMP || process.cwd(), 'pet-orchestration-qa');
fs.mkdirSync(output, { recursive: true });

(async () => {
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME_PATH || 'C:/Program Files/Google/Chrome/Application/chrome.exe' });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(15000);
    const errors = [];
    page.on('pageerror', err => { errors.push(err.message); console.error(err.message); });
    await page.goto(baseUrl);
    await page.locator('.sidebar-item-main').click();

    // 切到智能调度模式（默认 direct，feature flag 关闭）。
    await page.locator('.orch-mode-option').filter({ hasText: '智能调度' }).click();
    assert.equal(await page.locator('.orch-mode-option.active').textContent(), '智能调度');
    // 互斥：自由协同条隐藏。
    assert.equal(await page.locator('.collaboration-bar').count(), 0);

    // 困难任务：必经评估 + 独立审核。
    await page.locator('.dash-chat-input').fill('修复记忆库的 bug');
    await page.locator('.dash-chat-send-btn').click();
    const card = page.locator('.orch-task-card');
    await card.waitFor();
    await card.getByText('需要独立审核').waitFor();
    await card.getByText('任务评估').waitFor();
    await page.screenshot({ path: path.join(output, 'running-review.png') });
    await card.getByText('验收通过', { exact: false }).first().waitFor({ timeout: 10000 });
    await page.screenshot({ path: path.join(output, 'succeeded-review.png') });

    // 详情面板：节点 / 审核报告 / 交付物。
    await card.getByRole('button', { name: '查看详情' }).click();
    const detail = page.locator('.orch-detail');
    await detail.waitFor();
    await detail.locator('.review-report').waitFor();
    await detail.getByText('checks-pass').waitFor();
    await detail.getByRole('button', { name: /v1 ·/ }).click();
    await detail.locator('.orch-detail-artifact pre').waitFor();
    await page.screenshot({ path: path.join(output, 'detail.png') });
    await detail.getByRole('button', { name: '关闭' }).click();

    // 简单任务：L0 快速路径，无独立审核标签。
    await page.locator('.dash-chat-input').fill('改写这段话');
    await page.locator('.dash-chat-send-btn').click();
    await page.locator('.orch-task-card').last().getByText('验收通过', { exact: false }).first().waitFor({ timeout: 10000 });
    const lastCard = page.locator('.orch-task-card').last();
    assert.equal(await lastCard.getByText('需要独立审核').count(), 0);
    await page.screenshot({ path: path.join(output, 'fast-path.png') });

    // 停止按钮：运行中可取消。
    await page.locator('.dash-chat-input').fill('重构调度器');
    await page.locator('.dash-chat-send-btn').click();
    const runningCard = page.locator('.orch-task-card').last();
    await runningCard.getByRole('button', { name: '停止' }).click();
    await runningCard.getByText('已取消').first().waitFor();

    assert.deepEqual(errors, []);
    console.log('orchestration browser qa ok');
  } finally {
    await browser.close();
  }
})().catch(err => { console.error(err); process.exit(1); });