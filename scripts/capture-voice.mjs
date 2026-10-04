import { chromium, webkit } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
const directory = 'artifacts/voice';
await mkdir(directory, {recursive: true});
const results = [];
for (const [name, engine] of [['chromium', chromium], ['webkit', webkit]]) {
 const browser = await engine.launch();
 try {
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  // Deliberate denied-device fixture. Never access the owner's microphone.
  await page.addInitScript(() => Object.defineProperty(navigator, 'mediaDevices', {
   configurable: true,
   value: {getUserMedia: () => Promise.reject(new DOMException('fixture', 'NotAllowedError'))},
  }));
  for (const [width,height] of [[960,720],[600,600],[390,844],[1440,900]]) {
   await page.setViewportSize({width,height});
   await page.goto('http://127.0.0.1:1420/');
   await page.evaluate(() => document.fonts.ready);
   await page.screenshot({path:`${directory}/${name}-${width}x${height}.png`});
   const measure = () => page.evaluate(() => ({
    width: innerWidth, height: innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
    scrollHeight: document.documentElement.scrollHeight,
    core: document.querySelector('.core-stage').getBoundingClientRect().width,
   }));
   const idle = await measure();
   await page.getByRole('button', {name:'Start listening',exact:true}).click();
   await page.getByText(/Microphone permission denied/).waitFor();
   const denied = await measure();
   results.push({engine:name,idle,denied});
  }
  results.push({engine:name,errors});
 } finally { await browser.close(); }
}
await writeFile(`${directory}/geometry.json`,JSON.stringify(results,null,2));
console.log(JSON.stringify(results,null,2));
