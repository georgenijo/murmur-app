// Refresh neutral Rust parity fixtures from the retained React implementation.
// Run explicitly on the Mac mini after npm ci; no application data or network.
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
import {mkdirSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
const root=fileURLToPath(new URL('../',import.meta.url));
const require=createRequire(new URL('../app/package.json',import.meta.url));
const {build}=require('esbuild');
mkdirSync(root+'app/gpui/target',{recursive:true});
const output=root+'app/gpui/target/theme-fixtures.cjs';
await build({stdin:{contents:`
import {resolveTheme} from './app/src/lib/appearance/resolve';
import {OPENCODE_THEME} from './app/src/lib/appearance/palettes';
import {writeFileSync} from 'node:fs';
const themes=[{version:1,presetId:'sonic'},OPENCODE_THEME,
 {version:1,presetId:'custom',accent:'#aa77ee'},
 {version:1,presetId:'custom',background:'#777777',foreground:'#777777',accent:'#777777'},
 {version:1,presetId:'custom',background:'#102132',foreground:'#eeeeee',accent:'#55aaee',contrast:40},
 {version:1,presetId:'custom',background:'#f8f0e8',accent:'#114488',contrast:-70},
 {version:1,presetId:'custom',dark:{background:'#000000',surface:'#000000','surface-container':'#10121a','on-surface':'#f5f5f5','on-surface-variant':'#b7b8d0','outline-variant':'#53566e',primary:'#74a9d8','on-primary':'#140d2a'}}];
const cases=themes.flatMap((theme,index)=>['light','dark'].map(mode=>({name:index+'-'+mode,theme,dark:mode==='dark',tokens:resolveTheme(theme,mode).tokens})));
writeFileSync(${JSON.stringify(root+'app/gpui/fixtures/theme-resolution.json')},JSON.stringify(cases,null,2)+'\\n');
`,resolveDir:root,loader:'ts'},bundle:true,platform:'node',format:'cjs',outfile:output});
execFileSync(process.execPath,[output],{stdio:'inherit'});
