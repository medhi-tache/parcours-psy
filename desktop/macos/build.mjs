import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import fs from 'node:fs/promises';
const project=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const build=path.join(project,'desktop/macos/build');
const frontend=path.join(project,'frontend');
const app=path.resolve(process.argv[2]||path.join(project,'..','Parcours Psy.app'));
if(process.platform!=='darwin'||process.arch!=='arm64')throw Error('Compilation prévue sur un Mac Apple Silicon.');
const env={...process.env,MACOSX_DEPLOYMENT_TARGET:'14.0',NEXT_TELEMETRY_DISABLED:'1'};
function run(command,args,cwd=project,extra={}){const r=spawnSync(command,args,{cwd,env:{...env,...extra},stdio:'inherit'});if(r.status!==0)throw Error(`${command} a échoué (${r.status}).`);}
await fs.mkdir(build,{recursive:true});
run('cargo',['test','--locked','--offline']);
run('cargo',['build','--release','--locked','--offline']);
const nextEnv=path.join(frontend,'next-env.d.ts');
const saved=await fs.readFile(nextEnv);
const ts=JSON.parse(await fs.readFile(path.join(frontend,'tsconfig.json'),'utf8'));
ts.include=['next-env.d.ts','app/**/*.ts','app/**/*.tsx','components/**/*.ts','components/**/*.tsx','.next-macos/types/**/*.ts'];
ts.exclude=['node_modules','.next'];
await fs.writeFile(path.join(frontend,'tsconfig.macos.json'),JSON.stringify(ts,null,2));
try{run('npm',['run','build'],frontend,{PARCOURS_MAC_BUILD:'1'});}finally{await fs.writeFile(nextEnv,saved);}
// Refuse destructive replacement outside a bundle with our identifier.
try{const old=await fs.readFile(path.join(app,'Contents/Info.plist'),'utf8');if(!old.includes('fr.parcourspsy.client'))throw Error('Destination occupée par une autre application.');await fs.rm(app,{recursive:true});}catch(e){if(e.code!=='ENOENT')throw e;}
const resources=path.join(app,'Contents/Resources');
await fs.mkdir(path.join(app,'Contents/MacOS'),{recursive:true});
await fs.mkdir(path.join(resources,'runtime'),{recursive:true});
await fs.copyFile(path.join(project,'desktop/macos/Info.plist'),path.join(app,'Contents/Info.plist'));
run('swiftc',['-swift-version','5','-O','-target','arm64-apple-macosx14.0','-module-cache-path',path.join(build,'module-cache'),'-framework','AppKit','-framework','WebKit','desktop/macos/Client.swift','-o',path.join(app,'Contents/MacOS/ParcoursPsy')]);
run('swiftc',['-swift-version','5','-module-cache-path',path.join(build,'module-cache'),'-framework','AppKit','desktop/macos/Icon.swift','-o',path.join(build,'MakeIcon')]);
run(path.join(build,'MakeIcon'),[path.join(build,'AppIcon.iconset')]);
run('iconutil',['-c','icns',path.join(build,'AppIcon.iconset'),'-o',path.join(resources,'AppIcon.icns')]);
await fs.copyFile(process.execPath,path.join(resources,'runtime/node'));
await fs.copyFile(path.join(project,'target/release/parcours-psy'),path.join(resources,'runtime/parcours-psy'));
await fs.cp(path.join(frontend,'.next-macos/standalone'),path.join(resources,'frontend'),{recursive:true,dereference:true});
await fs.cp(path.join(frontend,'.next-macos/static'),path.join(resources,'frontend/.next-macos/static'),{recursive:true});
try{await fs.cp(path.join(frontend,'public'),path.join(resources,'frontend/public'),{recursive:true});}catch(e){if(e.code!=='ENOENT')throw e;}
// Keep license notices with the redistributed runtime and npm/Rust dependencies.
const licenses=path.join(resources,'ThirdPartyLicenses');await fs.mkdir(licenses,{recursive:true});
async function collect(dir,prefix){for(const entry of await fs.readdir(dir,{withFileTypes:true})){if(entry.name.startsWith('.'))continue;const source=path.join(dir,entry.name);if(entry.isDirectory())await collect(source,path.join(prefix,entry.name));else if(/^(licen[sc]e|copying|notice|copyright)(\.|$|-)/i.test(entry.name)){const dest=path.join(licenses,prefix,entry.name);await fs.mkdir(path.dirname(dest),{recursive:true});await fs.copyFile(source,dest);}}}
await collect(path.join(frontend,'node_modules'),'npm');
const home=process.env.CARGO_HOME||path.join(process.env.HOME,'.cargo');
const registry=path.join(home,'registry/src');
const lock=await fs.readFile(path.join(project,'Cargo.lock'),'utf8');
for(const registryEntry of await fs.readdir(registry,{withFileTypes:true})){if(!registryEntry.isDirectory())continue;const registryName=registryEntry.name;for(const match of lock.matchAll(/name = "([^"]+)"\nversion = "([^"]+)"/g)){const dir=path.join(registry,registryName,`${match[1]}-${match[2]}`);try{const files=await fs.readdir(dir);for(const name of files.filter(n=>/^(licen[sc]e|copying|notice|copyright)(\.|$|-)/i.test(n))){const source=path.join(dir,name);if((await fs.stat(source)).isFile()){const dest=path.join(licenses,'rust',`${match[1]}-${match[2]}`,name);await fs.mkdir(path.dirname(dest),{recursive:true});await fs.copyFile(source,dest);}}}catch(e){if(e.code!=='ENOENT')throw e;}}}
await fs.copyFile(path.join(project,'desktop/macos/Node-LICENSE'),path.join(licenses,'Node-LICENSE'));
for(const file of ['runtime/node','runtime/parcours-psy']){await fs.chmod(path.join(resources,file),0o755);run('codesign',['--force','--sign','-',path.join(resources,file)]);}
run('codesign',['--force','--sign','-',app]);
run('codesign',['--verify','--deep','--strict',app]);
console.log(`Application créée : ${app}`);
