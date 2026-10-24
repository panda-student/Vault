// Isolated native UI fixture. Never opens the user's vault or contacts suppliers.
import { createServer as createTcpServer } from 'node:net';
import { spawn } from 'node:child_process';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const executable = resolve(process.env.VAULT_UI_EXE || 'src-tauri/target/release/local-password-manager.exe');
const directory = await mkdtemp(join(tmpdir(), 'vault-ui-audit-'));
const secret='SYNTHETIC_UI_AUDIT_KEY_ONLY';
const portProbe = createTcpServer();
await new Promise(resolve => portProbe.listen(0, '127.0.0.1', resolve));
const port = portProbe.address().port;
await new Promise(resolve => portProbe.close(resolve));
const client = new Client({ name: 'isolated-ui-fixture', version: '1.0.0' }, { capabilities: {} });
await client.connect(new StdioClientTransport({ command: executable, args: ['--mcp', '--data-dir', directory], stderr: 'pipe' }));
await client.callTool({name:'create_entries',arguments:{items:[
 {kind:'database',title:'订单数据库 · 测试',tags:['订单系统','测试'],database:{databaseType:'MySQL',host:'db-test.example',port:'3306',databaseName:'orders'},account:'orders_test',secret},
 {kind:'database',title:'订单数据库 · UVT',tags:['订单系统','UVT'],database:{databaseType:'Oracle',host:'db-uvt.example',port:'1521',databaseName:'ORCL'},account:'orders_uvt',secret},
 {kind:'password',title:'GitHub · 个人',tags:['GitHub','个人'],account:'alex.personal@example.com',url:'https://github.com',secret},
 {kind:'password',title:'GitHub · 工作',tags:['GitHub','工作'],account:'alex.work@example.com',url:'https://github.com',secret},
 {kind:'api',title:'模型服务',tags:['内部平台','工作'],secret,services:[{protocol:'openai',url:'https://api.example/v1'},{protocol:'anthropic',url:'https://anthropic.example'}]}
]}});
await client.close();
const desktop = spawn(executable, ['--data-dir', directory], { windowsHide: true, stdio: 'ignore', env: { ...process.env, VAULT_UI_DEBUG_PORT: String(port), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` } });
const runtime = { executable, directory, pid: desktop.pid, port };
await writeFile('src-tauri/target/ui-audit-runtime.json', JSON.stringify(runtime, null, 2));
console.log(JSON.stringify(runtime));
let stopping = false;
function stop() { if (stopping) return; stopping = true; desktop.kill(); process.exit(0); }
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
desktop.on('exit',()=>process.exit(0));
