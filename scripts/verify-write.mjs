import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import {mkdtemp,writeFile,readFile,copyFile,chmod,readdir} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {spawn,execFile} from 'node:child_process';
import {createHash} from 'node:crypto';
import {promisify} from 'node:util';
const run=promisify(execFile);
const directory=await mkdtemp(join(tmpdir(),'vault-write-audit-'));
const executable=resolve(process.env.VAULT_WRITE_EXE || 'src-tauri/target/release/local-password-manager.exe');
const report={passed:false,executable,directory,checks:[]};
const outputs=[];
const check=(name,passed)=>{report.checks.push({name,passed});console.log(`${passed?'PASS':'FAIL'}: ${name}`);if(!passed)throw Error('Audit check failed');};
const transport=new StdioClientTransport({command:executable,args:['--mcp','--data-dir',join(directory,'data')],stderr:'pipe'});
transport.stderr?.on('data',data=>outputs.push(String(data)));
const client=new Client({name:'isolated-native-write-audit',version:'1.0.0'});
const call=async(name,args)=>{const response=await client.callTool({name,arguments:args},undefined,{timeout:12000});outputs.push(JSON.stringify(response));return {failed:response.isError===true,value:response.structuredContent??JSON.parse(response.content[0].text)};};
const secret='FAKE_NATIVE_WRITE_ONLY_安全_907';
const states=[];
const helper=async(action,state,editor,file)=>{
 report.stage=editor?`${action}-${editor}`:action;
 const args=['-NoProfile','-ExecutionPolicy','Bypass','-File',resolve('scripts/verify-native-write.ps1'),'-Action',action,'-StatePath',state,...(editor?['-Editor',editor,'-FilePath',file]:[])];
 if(action==='Launch'){
  // GUI children may inherit helper pipes. Do not wait for GUI EOF.
  const child=spawn('powershell.exe',args,{windowsHide:true,stdio:'ignore'});
  for(let attempt=0;attempt<160;attempt++){
   try{JSON.parse((await readFile(state,'utf8')).replace(/^\uFEFF/,''));return {launched:true};}catch{}
   if(child.exitCode!==null&&child.exitCode!==0)throw Error('Editor launch helper failed');
   await new Promise(resolve=>setTimeout(resolve,200));
  }
  throw Error('Editor launch timed out');
 }
 const {stdout}=await run('powershell.exe',args,{windowsHide:true,timeout:45000,maxBuffer:1024*1024});return JSON.parse(stdout.trim().replace(/^\uFEFF/,''));
};
try{
 report.executableSha256=createHash('sha256').update(await readFile(executable)).digest('hex');
 await client.connect(transport);
 const created=await call('create_entries',{kind:'password',title:'Isolated native writing fixture',secret,account:'fixture-account'});
 const args={credentialId:created.value.id,field:'secret'};
 const file=join(directory,'direct.txt');
 let result=await call('write_credential',{...args,target:{type:'file',path:file,template:'TOKEN={{credential}}\nkeep=true\n'}});
 check('Direct file create receives exact credential; MCP only returns status',!result.failed&&result.value.status==='file_written'&&await readFile(file,'utf8')===`TOKEN=${secret}\nkeep=true\n`);
 result=await call('write_credential',{...args,target:{type:'file',path:file}});
 check('Default create never overwrites existing file',result.failed&&await readFile(file,'utf8')===`TOKEN=${secret}\nkeep=true\n`);
 await writeFile(file,'before=1\nTOKEN=REPLACE_HERE\nafter=2\n');
 result=await call('write_credential',{...args,target:{type:'file',path:file,mode:'replace',placeholder:'REPLACE_HERE'}});
 check('Existing file placeholder replacement preserves surrounding text',!result.failed&&await readFile(file,'utf8')===`before=1\nTOKEN=${secret}\nafter=2\n`);
 result=await call('write_credential',{...args,target:{type:'file',path:file,mode:'replace',placeholder:'absent'}});
 check('Missing placeholder fails without changing file',result.failed&&await readFile(file,'utf8')===`before=1\nTOKEN=${secret}\nafter=2\n`);
 result=await call('write_credential',{...args,target:{type:'file',path:'relative.txt'}});
 check('Relative file path is rejected',result.failed&&result.value.code==='ABSOLUTE_FILE_PATH_REQUIRED');
 result=await call('write_credential',{...args,target:{type:'file',path:join(directory,'duplicate.txt'),template:'{{credential}} {{credential}}'}});
 check('Ambiguous duplicate marker is rejected before creating a file',result.failed&&!(await readdir(directory)).includes('duplicate.txt'));
 const binary=join(directory,'binary.bin');await writeFile(binary,Buffer.from([255,254,255]));
 result=await call('write_credential',{...args,target:{type:'file',path:binary,mode:'replace'}});
 check('Non-UTF8 existing file is not modified',result.failed&&(await readFile(binary)).equals(Buffer.from([255,254,255])));
 const locked=join(directory,'readonly.txt');await writeFile(locked,'{{credential}}');await chmod(locked,0o444);
 try{result=await call('write_credential',{...args,target:{type:'file',path:locked,mode:'replace'}});check('Read-only file fails without changing contents',result.failed&&await readFile(locked,'utf8')==='{{credential}}');}finally{await chmod(locked,0o666);}
 check('No staging files remain after file operations',!(await readdir(directory)).some(name=>name.startsWith('.vault-write-')));
 for(const editor of ['notepad','ise']){
  const file=join(directory,`vault-write-audit-${editor}-${Date.now()}.${editor==='ise'?'ps1':'txt'}`);
  const state=join(directory,`${editor}.json`);
  const prefix=editor==='ise'?'# fixture-token=':'fixture-token=';
  await writeFile(file,(editor==='ise'?'\uFEFF':'')+prefix);
  await helper('Launch',state,editor,file);states.push(state);
  let focus=await helper('Focus',state);
  // No credential has been captured or sent yet; retry fixture focus only.
  for(let attempt=0;attempt<2&&!focus.focused;attempt++)focus=await helper('Focus',state);
  check(`Actual ${editor} has keyboard focus`,focus.focused===true);
  report[editor]={controlType:focus.controlType};
  const captured=await call('capture_fill_target',{});
  report[editor].captureStatus=captured.value.status;
  check(`Actual ${editor} target captured`,!captured.failed&&captured.value.status==='target_captured'&&captured.value.processId===focus.processId);
  const written=await call('write_credential',{...args,target:{type:'input',targetId:captured.value.targetId}});
  report[editor].writeStatus=written.value.status;
  check(`Actual ${editor} receives input events`,!written.failed&&written.value.status==='input_sent');
  await new Promise(resolve=>setTimeout(resolve,400));
  await helper('Save',state);
  let accepted=false;
  for(let attempt=0;attempt<20;attempt++){const content=(await readFile(file,'utf8')).replace(/^\uFEFF/,'').replace(/\r\n/g,'\n');if(content===prefix+secret||content===prefix+secret+'\n'){accepted=true;break;}await new Promise(resolve=>setTimeout(resolve,100));}
  check(`Independent saved ${editor} file confirms exact inserted value and preserved prefix`,accepted);
  check(`Actual ${editor} target cannot be replayed`,(await call('write_credential',{...args,target:{type:'input',targetId:captured.value.targetId}})).value.status==='target_unavailable');
  await helper('Capture',state);
  await copyFile(join(directory,editor+'.png'),resolve('docs/evidence/native-write-'+editor+'.png'));
  await helper('Close',state);states.pop();
 }
 check('MCP and diagnostics contain no protected plaintext or base64',outputs.every(text=>!text.includes(secret)&&!text.includes(Buffer.from(secret).toString('base64'))));
 report.passed=true;
}catch(error){report.failureStage=report.stage;report.failureType=error.name;report.failureCode=error.code??null;report.error='Native write verification failed; inspect named check and status evidence';console.error(report.error);if(error.stderr){for(const reason of ['Could not focus isolated editor','Not the isolated Notepad tab','Audit window owner changed','Isolated window capture failed'])if(error.stderr.includes(reason)){report.helperReason=reason;console.error(reason);}}process.exitCode=1;}
finally{for(const state of states){try{await helper('Save',state);await helper('Close',state);}catch{}}await client.close();await writeFile(resolve('docs/evidence/native-write-verification.json'),JSON.stringify(report,null,2));}
