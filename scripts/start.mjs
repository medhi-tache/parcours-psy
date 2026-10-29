import {spawn} from 'node:child_process';
import {existsSync,mkdirSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import os from 'node:os';
import net from 'node:net';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),front=path.join(root,'frontend');
const next=path.join(front,'node_modules','next','dist','bin','next');
const env={...process.env,NEXT_TELEMETRY_DISABLED:'1'};
const remote=process.env.RUST_API_URL&&!['http://127.0.0.1:8080','http://localhost:8080'].includes(process.env.RUST_API_URL);
const children=[];
function run(command,args,cwd=root){return new Promise((resolve,reject)=>{const child=spawn(command,args,{cwd,env,stdio:'inherit'});child.on('error',reject);child.on('exit',code=>code===0?resolve():reject(Error('La préparation a échoué. Consultez le message ci-dessus.')));});}
function available(port){return new Promise(resolve=>{const socket=net.connect({host:'127.0.0.1',port});socket.on('connect',()=>{socket.destroy();resolve(false);});socket.on('error',()=>resolve(true));});}
function start(command,args,cwd=root){const child=spawn(command,args,{cwd,env,stdio:'inherit'});children.push(child);child.on('error',error=>{console.error(error.message);stop(1);});child.on('exit',code=>{if(!stopping)stop(code||0);});return child;}
let stopping=false;
function stop(code=0){if(stopping)return;stopping=true;for(const child of children)child.kill('SIGTERM');setTimeout(()=>process.exit(code),300).unref();}
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>stop());
try{
 if(!existsSync(next))throw Error('Installez d’abord l’interface : dans frontend, exécutez npm ci.');
 for(const port of remote?[3000]:[8080,3000])if(!await available(port))throw Error(`Le port local ${port} est déjà utilisé. Fermez la précédente instance avant de relancer.`);
 if(!remote){const cargo=path.join(os.homedir(),'.cargo','bin',process.platform==='win32'?'cargo.exe':'cargo');await run(existsSync(cargo)?cargo:'cargo',['build','--locked','--offline']);}
 if(!existsSync(path.join(front,'.next','BUILD_ID')))await run(process.execPath,[next,'build'],front);
 if(!remote){mkdirSync(path.join(root,'data'),{recursive:true,mode:0o700});env.PARCOURS_DB=process.env.PARCOURS_DB||path.join(root,'data','parcours.sqlite');start(path.join(root,'target','debug',process.platform==='win32'?'parcours-psy.exe':'parcours-psy'),[]);}
 start(process.execPath,[next,'start','--hostname','127.0.0.1','--port','3000'],front);
 console.log('\nParcours Psy : http://127.0.0.1:3000\nLaissez cette fenêtre ouverte. Arrêt : Ctrl+C.\n');
}catch(error){console.error(error.message);stop(1);process.exitCode=1;}
