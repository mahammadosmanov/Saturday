import {chromium,webkit} from '@playwright/test';
import {mkdir,writeFile} from 'node:fs/promises';
const out=new URL('../artifacts/compact/',import.meta.url);await mkdir(out,{recursive:true});
const results=[];
for(const [engine,type] of Object.entries({chromium,webkit})) {
 const browser=await type.launch();
 try {
  const page=await browser.newPage({viewport:{width:960,height:720},deviceScaleFactor:2});
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.addInitScript(()=>{
   window.paintTimes=[];const clear=CanvasRenderingContext2D.prototype.clearRect;
   CanvasRenderingContext2D.prototype.clearRect=function(...args){if(this.canvas.classList.contains('core-particles'))window.paintTimes.push(performance.now());return clear.apply(this,args);};
  });
  await page.goto('http://127.0.0.1:1420/');await page.evaluate(()=>document.fonts.ready);
  await page.getByRole('region',{name:'Assistant core'}).waitFor();
  const sample=async(label)=>{
   await page.evaluate(()=>{window.paintTimes=[];});await page.waitForTimeout(3000);
   return page.evaluate(label=>{
    const t=window.paintTimes,deltas=t.slice(1).map((n,i)=>n-t[i]).sort((a,b)=>a-b);
    return {label,canvasPaints:t.length,medianGapMs:deltas[Math.floor(deltas.length*.5)],p95GapMs:deltas[Math.floor(deltas.length*.95)],maxGapMs:deltas.at(-1),ringPlayers:[...document.querySelectorAll('[data-orbit]')].map(el=>{const a=el.getAnimations()[0];return {ring:el.dataset.orbit,state:a.playState,rate:a.playbackRate,time:a.currentTime};})};
   },label);
  };
  const idle=await sample('idle');
  await page.screenshot({path:new URL(`home-${engine}.png`,out).pathname,fullPage:true});
  await page.getByRole('button',{name:'Workspace',exact:true}).click();
  await page.screenshot({path:new URL(`workspace-${engine}.png`,out).pathname,fullPage:true});
  await page.getByRole('button',{name:'Diagnostics',exact:true}).click();await page.getByRole('button',{name:'Run diagnostic',exact:true}).click();await page.keyboard.press('Escape');
  const active=await sample('diagnostic');
  await page.getByRole('button',{name:'Workspace',exact:true}).click();
  if(await page.getByRole('button',{name:'Cancel diagnostic',exact:true}).count())await page.getByRole('button',{name:'Cancel diagnostic',exact:true}).click();
  await page.keyboard.press('Escape');await page.getByRole('button',{name:'Settings',exact:true}).click();await page.getByRole('button',{name:'Amber',exact:true}).click();await page.keyboard.press('Escape');
  await page.screenshot({path:new URL(`amber-${engine}.png`,out).pathname,fullPage:true});
  const main=await page.evaluate(()=>({text:document.querySelector('main').innerText,heading:document.querySelector('h1').textContent,mainButtons:document.querySelectorAll('main button').length,coreSize:document.querySelector('.core-stage').getBoundingClientRect().width,scrollWidth:document.documentElement.scrollWidth,scrollHeight:document.documentElement.scrollHeight}));
  await page.setViewportSize({width:390,height:844});await page.screenshot({path:new URL(`mobile-${engine}.png`,out).pathname,fullPage:true});
  if(errors.length||idle.canvasPaints<20||active.canvasPaints<20)throw new Error(JSON.stringify({engine,errors,idle,active}));
  results.push({engine,idle,active,main,errors});
 } finally {await browser.close();}
}
await writeFile(new URL('animation-evidence.json',out),JSON.stringify({method:'Canvas clearRect timestamps, 3-second idle and active samples per browser; observed scheduling, not hardware/compositor certification.',results},null,2)+'\n');
console.log(JSON.stringify(results,null,2));
