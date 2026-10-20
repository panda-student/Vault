import {readFile,readdir,writeFile} from 'node:fs/promises';
import {dirname,join} from 'node:path';
import {createHash} from 'node:crypto';
// Input is cargo metadata --filter-platform x86_64-pc-windows-msvc.
const metadata=JSON.parse(await readFile('src-tauri/target/dependency-metadata.json','utf8'));
const nodes=new Map(metadata.resolve.nodes.map(n=>[n.id,n]));
const ids=new Set();
function visit(id){
 if(ids.has(id))return;ids.add(id);
 for(const dep of nodes.get(id)?.deps??[]) if(dep.dep_kinds.some(k=>k.kind===null))visit(dep.pkg);
}
visit(metadata.resolve.root);
const chunks=['Third-party notices\n\nGenerated from the Windows dependency graph and frontend runtime packages.\nEach component retains its own license; this project does not relicense dependencies.\n'];
const missing=[];
const sharedTexts=new Map();
async function append(name,license,directory,extra){
 const names=(await readdir(directory)).filter(n=>/^(license|licence|copying|copyright|notice)([._-]|$)/i.test(n));
 if(extra&&!names.includes(extra))names.push(extra);
 const texts=[];
 for(const file of names){try{texts.push(await readFile(join(directory,file),'utf8'));}catch{}}
 if(!texts.length){try{texts.push(await readFile(join('docs/licenses',name.split(' ')[0]+'.txt'),'utf8'));}catch{}}
 const refs=texts.map(text=>{
   const hash=createHash('sha256').update(text).digest('hex');
   if(!sharedTexts.has(hash))sharedTexts.set(hash,{id:sharedTexts.size+1,text});
   return sharedTexts.get(hash).id;
 });
 chunks.push(`\n${name}\nDeclared license: ${license||'See component files'}\nLicense/notice texts: ${refs.join(', ')}\n`);
 if(!texts.length)missing.push(name);
}
for(const p of metadata.packages.filter(p=>ids.has(p.id)&&p.id!==metadata.resolve.root).sort((a,b)=>a.name.localeCompare(b.name))){
 await append(`${p.name} ${p.version}`,p.license,dirname(p.manifest_path),p.license_file);
}
for(const name of ['vue','@vue/shared','@vue/reactivity','@vue/runtime-core','@vue/runtime-dom','@tauri-apps/api']){
 const directory=join('node_modules',name),p=JSON.parse(await readFile(join(directory,'package.json'),'utf8'));
 await append(`${name} ${p.version}`,p.license,directory);
}
await writeFile('THIRD_PARTY_NOTICES.txt',chunks.join('\n')+'\n'+[...sharedTexts.values()].map(v=>`${'='.repeat(72)}\nLicense/notice text ${v.id}\n\n${v.text}`).join('\n\n'));
console.log(JSON.stringify({components:chunks.length-1,missing}));
if(missing.length)process.exitCode=1;
