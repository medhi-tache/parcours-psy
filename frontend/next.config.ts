import type { NextConfig } from 'next';
const macBuild=process.env.PARCOURS_MAC_BUILD==='1';
const config:NextConfig={output:macBuild?'standalone':undefined,distDir:macBuild?'.next-macos':'.next',typescript:{tsconfigPath:macBuild?'tsconfig.macos.json':'tsconfig.json'},poweredByHeader:false,async headers(){return [{source:'/:path*',headers:[{key:'X-Content-Type-Options',value:'nosniff'},{key:'Referrer-Policy',value:'no-referrer'},{key:'X-Frame-Options',value:'DENY'},{key:'Cache-Control',value:'no-store'}]}];}};
export default config;
