// Real native input through Vault MCP; browser automation only sets up/observes fixtures.
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdtemp, writeFile, mkdir, readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve, join, dirname} from 'node:path';
import {pathToFileURL} from 'node:url';
import {randomUUID, createHash} from 'node:crypto';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';

const run = promisify(execFile);
const directory = await mkdtemp(join(tmpdir(), 'vault-browser-fill-'));
await mkdir('artifacts', {recursive:true});
const title = 'vault browser fill audit ' + randomUUID();
const html = join(directory, 'fixture.html');
const secret = 'FAKE_BROWSER_FILL_ONLY_安全_42';
await writeFile(html, `<!doctype html><meta charset="utf-8"><title>${title}</title>
<style>body{font:18px sans-serif;padding:24px}input{display:block;padding:8px;margin:12px}label{display:block}</style>
<h1>Isolated Vault browser filling test</h1><form onsubmit="event.preventDefault();window.submits++">
<label>Account<input id="account" autocomplete="username"></label>
<label>Password<input id="password" type="password" autocomplete="current-password"></label>
<input id="localized" type="password" aria-label="登录密码" autocomplete="off">
<label>Readonly<input id="readonly" readonly></label>
<label>Duplicate<input id="duplicate1" type="password"></label>
<label>Duplicate<input id="duplicate2" type="password"></label>
<button type="submit">Sign in</button></form>
<script>window.submits=0;window.events=[];document.addEventListener('input',e=>window.events.push({id:e.target.id,trusted:e.isTrusted}));</script>`);
const agent = process.env.AGENT_BROWSER_EXE || join(dirname(process.execPath),'node_modules/agent-browser/bin/agent-browser-win32-x64.exe');
const session = 'vault-browser-fill-' + process.pid;
const browserPath = process.env.VAULT_BROWSER_EXE || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const browser = async (...args) => (await run(agent, ['--session', session, ...args], {timeout:30000, maxBuffer:1024*1024, windowsHide:true})).stdout.trim();
const evaluate = async expression => JSON.parse(await browser('eval', expression));
const executable = resolve(process.env.VAULT_EXE || 'src-tauri/target/release/local-password-manager.exe');
const client = new Client({name:'isolated-browser-fill-audit', version:'1'});
const transport = new StdioClientTransport({command:executable, args:['--mcp','--data-dir',join(directory,'data')], stderr:'pipe'});
transport.stderr?.resume();
const outputs = [];
const call = async (name, args) => {
  const result = await client.callTool({name, arguments:args});
  outputs.push(JSON.stringify(result));
  if (result.isError) throw Error('MCP returned an error: ' + JSON.stringify(result));
  return result.structuredContent ?? JSON.parse(result.content.find(c=>c.type==='text').text);
};
const report = {passed:false, directory, executable, browserPath, checks:[]};
const check = (name, passed) => {
  report.checks.push({name, passed});
  console.log(`${passed ? 'PASS' : 'FAIL'}: ${name}`);
  if (!passed) throw Error(name);
};
const locate = (fieldName, expectedPassword) => call('capture_fill_target', {windowTitle:title, fieldName, expectedPassword});
try {
  report.executableSha256 = createHash('sha256').update(await readFile(executable)).digest('hex');
  await browser('--executable-path', browserPath, '--headed', 'open', pathToFileURL(html).href);
  report.browser = await evaluate('({userAgent:navigator.userAgent})');
  const window = JSON.parse((await run('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File','scripts/verify-browser-window.ps1','-Title',title],{windowsHide:true,timeout:15000})).stdout);
  report.windowSetup = window;
  check('fixture window has native foreground before testing', window.foreground===true);
  check('fixture setup did not focus a form input', (await evaluate('({inputFocused:document.activeElement instanceof HTMLInputElement})')).inputFocused===false);
  await client.connect(transport);
  const entry = await call('create_entries', {kind:'password', title:'Browser fixture', secret});
  const fill = targetId => call('write_credential', {credentialId:entry.id, field:'secret', target:{type:'input', targetId}});
  // Setup activates only the browser window. Vault must locate/focus each input;
  // no browser typing, native input click or DOM assignment of the secret is used.
  for (const [label, id, expected] of [['Password','password',true], ['登录密码','localized',true], ['Account','account',false]]) {
    const target = await locate(label, expected);
    (report.targets ??= []).push({label, result:target});
    if (!target.targetId) console.log('Capture status:', JSON.stringify(target));
    check(`${label}: native target acquired`, target.status==='target_captured' && target.isPassword===expected && target.processId===window.processId);
    const result = await fill(target.targetId);
    check(`${label}: native events dispatched`, result.status==='input_sent');
    const state = await evaluate(`({matched:document.getElementById('${id}').value===${JSON.stringify(secret)},active:document.activeElement.id,trusted:window.events.filter(e=>e.id==='${id}').every(e=>e.trusted),events:window.events.filter(e=>e.id==='${id}').length,submits:window.submits})`);
    report[id] = state;
    check(`${label}: actual DOM value matches synthetic credential`, state.matched && state.active===id);
    check(`${label}: trusted native input and no submit`, state.trusted && state.events>0 && state.submits===0);
    check(`${label}: reference is single use`, (await fill(target.targetId)).status==='target_unavailable');
  }
  for (const [label, expected, reason] of [['Account',true,'field_type_mismatch'], ['Readonly',false,'read_only_or_non_input'], ['Duplicate',true,'ambiguous_field'], ['Missing',true,'field_not_found']]) {
    const result = await locate(label, expected);
    check(`${label}: rejected before typing (${reason})`, !result.targetId && result.reason===reason);
  }
  const target = await locate('Password',true);
  check('Focus-change fixture captured', !!target.targetId);
  await browser('click','#account');
  check('Changed focus invalidates reference', (await fill(target.targetId)).status==='target_changed');
  check('MCP outputs contain no credential plaintext', !outputs.join('').includes(secret));
  await browser('screenshot',resolve('artifacts/browser-fill-edge.png'));
  report.passed=true;
} finally {
  await client.close().catch(()=>{});
  await browser('close').catch(()=>{});
  await mkdir('artifacts',{recursive:true});
  await writeFile('artifacts/browser-fill-acceptance.json',JSON.stringify(report,null,2));
}
