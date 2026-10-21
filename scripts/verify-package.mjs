import {mkdtemp,readFile,writeFile,readdir} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
const run=promisify(execFile),version=JSON.parse(await readFile('package.json','utf8')).version;
const artifacts=resolve(process.env.VAULT_ARTIFACTS||'artifacts'),root=await mkdtemp(join(tmpdir(),'vault-package-'));
const size=JSON.parse((await readFile(join(artifacts,'size.json'),'utf8')).replace(/^\uFEFF/,''));
const hash=async p=>createHash('sha256').update(await readFile(p)).digest('hex');
await run('powershell',['-NoProfile','-Command','Expand-Archive -LiteralPath $env:VAULT_ZIP -DestinationPath $env:VAULT_UNZIP'],{windowsHide:true,env:{...process.env,VAULT_ZIP:join(artifacts,`vault-${version}.zip`),VAULT_UNZIP:root}});
const folder=join(root,`vault-${version}`),executable=join(folder,'Vault.exe');
assert.equal(await hash(join(artifacts,`vault-${version}.zip`)),size.zipSha256);
assert.equal(await hash(executable),size.files.find(f=>f.name==='Vault.exe').sha256);
assert.deepEqual((await readdir(folder)).sort(),['LICENSE.txt','NOTICE.txt','README.txt','THIRD_PARTY_NOTICES.txt','Vault.exe','third-party-sources'].sort());
assert.ok((await readFile(join(folder,'LICENSE.txt'),'utf8')).includes('Apache License'));
for(const script of ['verify-next.mjs','verify-write.mjs']){
 const {stdout}=await run(process.execPath,[join('scripts',script)],{windowsHide:true,timeout:240000,maxBuffer:1024*1024,env:{...process.env,VAULT_EXE:executable,VAULT_WRITE_EXE:executable}});
 console.log(stdout);
}
await writeFile('docs/evidence/package-verification.json',JSON.stringify({passed:true,version,executableSha256:await hash(executable),zipSha256:size.zipSha256,onlyDistributionFiles:true,size},null,2));
console.log('PASS package contents, hashes, MCP, desktop, native editors and file writes');
