const path = require('path');
const root = path.resolve(__dirname, '..', '..', '..');
const { chromium } = require(path.join(root, 'e2e', 'node_modules', 'playwright'));
const sharp = require(path.join(root, 'frontend', 'node_modules', 'sharp'));
(async () => {
  const dir = __dirname;
  const out = path.join(root, 'website', 'public', 'ui');
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1600, height: 1000 }, deviceScaleFactor: 2 });
  for (const view of ['vault', 'canvas', 'twin']) {
    const file = 'file:///' + path.join(dir, view + '.html').split(path.sep).join('/');
    await page.goto(file);
    await page.waitForTimeout(600);
    const png = await page.screenshot({ type: 'png' });
    await sharp(png).resize(1600).webp({ quality: 84 }).toFile(path.join(out, view + '.webp'));
    console.log(view, 'ok');
  }
  await browser.close();
})().catch((err) => { console.error(err); process.exit(1); });
