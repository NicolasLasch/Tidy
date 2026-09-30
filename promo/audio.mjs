// Procedural soundtrack: synthesized in pure JS, timed to the animation's hit points. 120 BPM.
import {writeFileSync} from 'node:fs';
const SR=44100,DUR=24.4,N=Math.round(SR*DUR),L=new Float32Array(N),R=new Float32Array(N);
let seed=1;const rnd=()=>{seed=(seed*1664525+1013904223)>>>0;return seed/4294967296*2-1};
function add(t0,len,fn,g=1,pan=0){const a=Math.max(0,Math.floor(t0*SR)),b=Math.min(N,Math.floor((t0+len)*SR));for(let i=a;i<b;i++){const t=(i-a)/SR,v=fn(t)*g;L[i]+=v*(1-Math.max(0,pan));R[i]+=v*(1+Math.min(0,pan))}}
const kick=(t,g=1)=>{let ph=0;add(t,.4,s=>{ph+=2*Math.PI*(48+110*Math.exp(-s*22))/SR;return Math.sin(ph)*Math.exp(-s*6.5)},g*.9)};
const boom=(t,g=1)=>{let ph=0;add(t,1.4,s=>{ph+=2*Math.PI*(28+70*Math.exp(-s*8))/SR;return Math.sin(ph)*Math.exp(-s*2.4)},g);add(t,.5,s=>rnd()*Math.exp(-s*9)*.6,g*.6)};
const clap=(t,g=1)=>{let lp=0;add(t,.25,s=>{lp+=(rnd()-lp)*.5;return (rnd()-lp*.3)*Math.exp(-s*22)*(1+Math.sin(s*220)*.3)},g*.35)};
const hat=(t,g=1,o=false)=>{let p=0;add(t,o?.22:.06,s=>{const x=rnd();const y=x-p;p=x;return y*Math.exp(-s*(o?16:70))},g*.22,Math.sin(t*3)*.4)};
const tick=(t,g=1)=>{let p=0;add(t,.03,s=>{const x=rnd();const y=x-p*.6;p=x;return y*Math.exp(-s*160)},g*.35)};
const pop=(t,f=700,g=1)=>{let ph=0;add(t,.16,s=>{ph+=2*Math.PI*(f*(1+.8*Math.exp(-s*30)))/SR;return Math.sin(ph)*Math.exp(-s*24)},g*.4)};
const chime=(t,f,g=1)=>{add(t,1.2,s=>(Math.sin(2*Math.PI*f*s)+.4*Math.sin(2*Math.PI*f*2.01*s)+.15*Math.sin(2*Math.PI*f*3.98*s))*Math.exp(-s*3.2),g*.18,.2)};
function whoosh(t0,len,g=1,up=true){let lp=0,lp2=0;add(t0,len,s=>{const k=s/len,c=up?.02+k*k*.6:.6-k*.55;lp+=(rnd()-lp)*c;lp2+=(lp-lp2)*c;return lp2*Math.sin(Math.PI*Math.min(1,k*(up?1:1.6)))**(up?2:1)*(up?k+.3:1)*4},g*.55)}
function bass(t,f,len,g=1){let ph=0;add(t,len,s=>{ph+=2*Math.PI*f/SR;const x=Math.sin(ph)+.35*Math.sin(2*ph)+.12*Math.sin(3*ph);return x*Math.min(1,s*120)*Math.exp(-s*3.2)},g*.42)}
function pad(t,notes,len,g=1){notes.forEach((f,i)=>add(t,len,s=>{const e=Math.min(1,s/.5)*Math.min(1,(len-s)/.4);return (Math.sin(2*Math.PI*f*s)+Math.sin(2*Math.PI*f*1.004*s)*.7+.25*Math.sin(2*Math.PI*f*2*s))*e},g*.05,(i-1)*.4))}
const A=110,mtof=m=>440*Math.pow(2,(m-69)/12);
const prog=[[57,60,64],[53,57,60],[48,55,60],[55,59,62]],rootM=[33,29,36,31];
// intro tension (0–1.7): tick rush + rumble
for(let i=0,t=.1;t<1.7;i++){tick(t,.5+t*.4);t+=Math.max(.04,.16-t*.09)}
whoosh(.0,1.7,.6,true);boom(1.75,1.1);
// 1.75–3.0: sparse
kick(2.25,.8);kick(2.75,.9);whoosh(2.6,.75,.8,true);
// 3.0 drop
boom(3.0,1);chime(3.05,mtof(81),1);chime(3.35,mtof(84),.8);chime(3.65,mtof(88),.7);
for(let b=0;b<46;b++){const t=3.0+b*.5;if(t>=22.6)break;kick(t,.85);}
for(let b=0;b<90;b++){const t=3.0+b*.25;if(t>=22.8)break;if(b%2==1)hat(t,b%4==3?1.1:.8,b%4==3);else if(t>5.4)hat(t,.5)}
for(let b=0;b<34;b++){const t=3.5+b*1;if(t>5.3&&t<22.6)clap(t,1)}
for(let b=0;b<40;b++){const t=3.0+b*2;if(t>=24.4)break;const c=Math.floor((t-3)/2)%4;pad(t,prog[c].map(mtof),2.2,1)}
for(let b=0;b<90;b++){const t=5.4+b*.25;if(t>=22.6)break;const c=Math.floor((t-3)/2)%4;if(b%4!=3)bass(t,mtof(rootM[c]+(b%8==6?12:0)),.22,b%2?.7:1)}
// transitions
[[5.05,.4],[14.05,.4],[16.0,.3],[17.0,.3],[17.95,.3],[18.85,.3]].forEach(([t,l])=>whoosh(t,l+.05,.9,true));
boom(5.4,.6);boom(14.4,.7);boom(16.3,.9);boom(17.25,.9);boom(18.2,.9);boom(19.15,1.3);
// typing
for(let t=6.05;t<7.08;t+=1/59)tick(t+ (rnd()*.004),.9);
pop(7.2,520,1);pop(7.45,640,.6);for(let i=0;i<4;i++)pop(8.05+i*.09+.45,760+i*90,.5);pop(8.85,900,.7);
pop(10.15,600,1);chime(10.2,mtof(76),.8);pop(10.45,700,.5);pop(10.57,780,.5);pop(10.69,860,.5);
pop(11.05,620,1);chime(11.1,mtof(79),1);chime(11.3,mtof(84),1);chime(11.5,mtof(88),.9);
pop(12.95,540,1);whoosh(12.85,.3,.5,true);pop(13.5,700,1);chime(13.55,mtof(84),1);chime(13.75,mtof(91),.9);
// storage count-up blips
for(let i=0;i<8;i++)pop(15.05+i*.1,900+i*80,.22);
// final chord shine
chime(19.4,mtof(72),1);chime(19.5,mtof(76),.9);chime(19.6,mtof(79),.9);chime(20.1,mtof(84),.8);chime(20.5,mtof(88),.7);chime(20.9,mtof(91),.6);chime(22.0,mtof(84),.5);
// master: soft clip, normalize, fade out
let peak=0;for(let i=0;i<N;i++){L[i]=Math.tanh(L[i]*1.1);R[i]=Math.tanh(R[i]*1.1);peak=Math.max(peak,Math.abs(L[i]),Math.abs(R[i]))}
const g=.89/peak,buf=Buffer.alloc(44+N*4);
buf.write('RIFF',0);buf.writeUInt32LE(36+N*4,4);buf.write('WAVEfmt ',8);buf.writeUInt32LE(16,16);buf.writeUInt16LE(1,20);buf.writeUInt16LE(2,22);buf.writeUInt32LE(SR,24);buf.writeUInt32LE(SR*4,28);buf.writeUInt16LE(4,32);buf.writeUInt16LE(16,34);buf.write('data',36);buf.writeUInt32LE(N*4,40);
for(let i=0;i<N;i++){const f=Math.min(1,(DUR*SR-i)/(SR*1.2));buf.writeInt16LE(Math.round(L[i]*g*f*32767),44+i*4);buf.writeInt16LE(Math.round(R[i]*g*f*32767),46+i*4)}
writeFileSync(new URL('./out/soundtrack.wav',import.meta.url),buf);console.log('wrote soundtrack.wav, peak',peak.toFixed(2));
