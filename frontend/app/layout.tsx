import type {Metadata} from 'next';
import './globals.css';
export const metadata:Metadata={title:'Parcours Psy',description:'Coordination des programmes de soins psychiatriques — prototype local.'};
export default function Layout({children}:{children:React.ReactNode}){return <html lang="fr"><body>{children}</body></html>;}
