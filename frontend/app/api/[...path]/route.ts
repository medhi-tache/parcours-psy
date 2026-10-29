import { NextRequest } from 'next/server';
export const runtime='nodejs';
export const dynamic='force-dynamic';
async function forward(req:NextRequest,context:{params:Promise<{path:string[]}>}){
 const origin=req.headers.get('origin'),allowed=process.env.APP_ORIGIN?[process.env.APP_ORIGIN.replace(/\/$/,'')]:['http://127.0.0.1:3000','http://localhost:3000'];
 const host=req.headers.get('host');if(!allowed.some(value=>new URL(value).host===host)||(origin&&!allowed.includes(origin)))return Response.json({error:'Origine non autorisée.'},{status:403});
 const {path}=await context.params,base=new URL(process.env.RUST_API_URL||'http://127.0.0.1:8080');
 if(!['http:','https:'].includes(base.protocol))return Response.json({error:'Configuration API invalide.'},{status:500});
 const target=new URL('/api/'+path.map(encodeURIComponent).join('/'),base);target.search=req.nextUrl.search;
 const headers=new Headers();for(const key of ['cookie','content-type','origin']){const value=req.headers.get(key);if(value)headers.set(key,value);}
 let body:Uint8Array|undefined;
 if(req.method!=='GET'&&req.method!=='HEAD'&&req.body){const reader=req.body.getReader(),parts:Uint8Array[]=[];let size=0;while(true){const {done,value}=await reader.read();if(done)break;size+=value.byteLength;if(size>2*1024*1024){await reader.cancel();return Response.json({error:'Requête trop volumineuse.'},{status:413});}parts.push(value);}body=new Uint8Array(size);let offset=0;for(const p of parts){body.set(p,offset);offset+=p.byteLength;}}
 try{const upstream=await fetch(target,{method:req.method,headers,body:body as BodyInit|undefined,cache:'no-store',redirect:'manual',signal:AbortSignal.timeout(15000)});const output=new Headers({'Cache-Control':'no-store','Content-Type':upstream.headers.get('content-type')||'application/json'});for(const cookie of upstream.headers.getSetCookie())output.append('Set-Cookie',cookie);return new Response(upstream.body,{status:upstream.status,headers:output});}catch{return Response.json({error:'API Rust indisponible. Vérifiez que le serveur est lancé.'},{status:502});}
}
export {forward as GET,forward as POST,forward as PUT};
