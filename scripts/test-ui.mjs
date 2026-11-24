import fs from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const html = fs.readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');
new vm.Script(html.match(/<script>([\s\S]*?)<\/script>/)[1]);
const nodes = Object.fromEntries(['running','save','curChip','current','list'].map(id => [id, {innerHTML:'',lastElementChild:{},className:''}]));
const ctx = {
  $: id => nodes[id], busy:false, refreshing:false, again:false, refreshGen:0,
  hasLogin:false, naming:false, masked:false, lastSt:null,
  esc:s => String(s ?? ''), tip:s => s || '', pii:s => s || '',
  akey:s => s || '', icon:() => '', field:() => '', eye:() => '',
  hid:() => false, fmtTime:() => '', setNaming:() => {}, toast:() => {},
};
vm.createContext(ctx);
vm.runInContext(html.slice(html.indexOf('function render(st)'), html.indexOf('async function refresh()')), ctx);
vm.runInContext(html.slice(html.indexOf('async function refresh()'), html.indexOf('// 用户操作结束后')), ctx);
const profile = {name:'Saved account',summary:{email:'test@example.com'},active:false,saved_at:1};
const state = {profiles:[profile],current:null,active:null,app_running:false,process_error:'query failed',current_error:null,profile_errors:[]};
ctx.render(state);
assert.equal(nodes.running.lastElementChild.textContent, '状态未知');
assert.match(nodes.list.innerHTML, /data-sw="Saved account"/);
ctx.render({...state,process_error:null,current_error:'credential unavailable'});
assert.match(nodes.current.innerHTML, /credential unavailable/);
assert.match(nodes.list.innerHTML, /data-sw="Saved account"/);
const lastList = nodes.list.innerHTML;
ctx.invoke = async () => {throw new Error('backend failed');};
await ctx.refresh();
assert.equal(nodes.list.innerHTML, lastList);
assert.equal(nodes.running.lastElementChild.textContent, '状态未知');
console.log('Frontend regression passed: partial errors preserve saved accounts and failed refresh clears stale status.');
