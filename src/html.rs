use crate::output::Output;

pub const CSS: &str = r##"
:root{
  --bg:#f4f6f4; --surface:#fff; --surface2:#eaeeec; --sunken:#eef1ef;
  --text:#101512; --muted:#5b6763; --faint:#8a9490; --border:#dce2df; --grid:#e6ebe8;
  --accent:#00846d;
  --c0:#00846d; --c1:#ad5527; --c2:#3465c8; --c3:#6d7a15; --c4:#a83a8e; --c5:#9a4a2f;
}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){
  --bg:#0e1211; --surface:#151a18; --surface2:#1c2220; --sunken:#111615;
  --text:#e5ebe8; --muted:#8d9a95; --faint:#6c7873; --border:#28302d; --grid:#212927;
  --accent:#2ea88f;
  --c0:#2ea88f; --c1:#c9743f; --c2:#6b8ae0; --c3:#8b9a2c; --c4:#cc63b4; --c5:#bf6a52;
}}
:root[data-theme="dark"]{
  --bg:#0e1211; --surface:#151a18; --surface2:#1c2220; --sunken:#111615;
  --text:#e5ebe8; --muted:#8d9a95; --faint:#6c7873; --border:#28302d; --grid:#212927;
  --accent:#2ea88f;
  --c0:#2ea88f; --c1:#c9743f; --c2:#6b8ae0; --c3:#8b9a2c; --c4:#cc63b4; --c5:#bf6a52;
}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);
  font:15px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;-webkit-font-smoothing:antialiased}
.mono,code,td.num,th.num{font-family:ui-monospace,SFMono-Regular,"JetBrains Mono",Menlo,monospace;font-variant-numeric:tabular-nums}
.wrap{max-width:1180px;margin:0 auto;padding:2rem 1.4rem 4rem}
h1{font-size:1.5rem;font-weight:650;letter-spacing:-.02em;margin:0 0 .2rem;text-wrap:balance}
.sub{color:var(--muted);font-size:.88rem;margin:0 0 1.5rem}
.card{background:var(--surface);border:1px solid var(--border);border-radius:10px;padding:1.2rem 1.3rem;
  box-shadow:0 1px 2px rgba(0,0,0,.04),0 10px 26px -20px rgba(0,0,0,.25)}
svg{display:block;width:100%;height:auto;overflow:visible}
.legend{display:flex;flex-wrap:wrap;gap:.4rem 1rem;margin-top:.9rem;font-size:.8rem;color:var(--muted)}
.legend span{display:inline-flex;align-items:center;gap:.4rem}
.legend i{width:11px;height:11px;border-radius:3px;display:inline-block;flex:none}
.tbl-scroll{overflow-x:auto}
.sechead th{font-size:.65rem;text-transform:uppercase;letter-spacing:.11em;color:var(--faint);
  font-weight:600;background:var(--bg);border-top:2px solid var(--faint);
  border-bottom:1px solid var(--border);padding:.55rem .7rem;text-align:left;position:static}
tbody.sec:first-of-type .sechead th{border-top:none}
tbody.sec .sechead:hover{background:transparent}
table{border-collapse:collapse;width:100%;font-size:.85rem;min-width:520px}
th,td{text-align:left;padding:.5rem .7rem;border-bottom:1px solid var(--border);white-space:nowrap}
thead th{font-size:.7rem;text-transform:uppercase;letter-spacing:.08em;color:var(--muted);
  font-weight:600;background:var(--surface2);position:sticky;top:0}
td.num,th.num{text-align:right}
tbody tr:hover{background:var(--sunken)}
.cellbar{height:9px;border-radius:0 3px 3px 0;background:var(--accent);min-width:1px}
.barwrap{width:120px;background:var(--sunken);border-radius:3px}
.tip{position:fixed;z-index:60;background:var(--text);color:var(--bg);padding:.45rem .6rem;border-radius:6px;
  font-size:.74rem;line-height:1.5;pointer-events:none;opacity:0;transition:opacity .08s;max-width:280px;
  font-family:ui-monospace,SFMono-Regular,Menlo,monospace}
.tip.on{opacity:1}
.empty{color:var(--faint);padding:2rem;text-align:center;font-size:.9rem}
.foot{margin-top:1.4rem;font-size:.75rem;color:var(--faint)}
.foot code{background:var(--surface2);padding:.15rem .4rem;border-radius:4px}
.src{color:var(--faint);font-size:.8rem}
.sub a,.src a{color:var(--accent);text-decoration:none;border-bottom:1px solid var(--border)}
.sub a:hover,.src a:hover{border-bottom-color:var(--accent)}
.sub a:focus-visible,.src a:focus-visible{outline:2px solid var(--accent);outline-offset:2px;border-radius:2px}
.drillbar{display:flex;align-items:center;gap:.6rem;margin-bottom:.8rem;font-size:.82rem}
.drillbar button{font:inherit;font-family:ui-monospace,Menlo,monospace;padding:.15rem .55rem;
  border:1px solid var(--border);border-radius:5px;background:var(--surface2);color:var(--text);cursor:pointer}
.drillbar button:hover:not(:disabled){border-color:var(--accent);color:var(--accent)}
.drillbar button:disabled{opacity:.4;cursor:default}
.drillbar .cur{color:var(--muted)}
a.dr{color:var(--text);text-decoration:none;border-bottom:1px dotted var(--border);cursor:pointer}
a.dr:hover{color:var(--accent);border-bottom-color:var(--accent)}
path.dr{cursor:pointer}
.src a{color:var(--accent);text-decoration:none;border-bottom:1px solid var(--border)}
.src a:hover{border-bottom-color:var(--accent)}
.src a:focus-visible{outline:2px solid var(--accent);outline-offset:2px;border-radius:2px}
.head{display:flex;gap:.6rem;align-items:baseline;flex-wrap:wrap;margin:0 0 1.5rem}
.head .sub{margin:0}
@media (prefers-reduced-motion:reduce){*{transition:none!important}}
.caveat{max-width:70ch;margin:1.4rem 0 0;font-size:.8rem;line-height:1.6;color:var(--faint);
  border-left:2px solid var(--border);padding-left:.9rem}
.caveat ul{margin:.45rem 0 0;padding-left:1.1rem}
.caveat li{margin:.35rem 0}
.caveat b{color:var(--muted);font-weight:600}
"##;

/// The standing caveats, shown under every chart.
///
/// Kept here, in one string, because the interactive app and the exported
/// single-file page must never disagree about what the numbers mean. Each entry is
/// a known, deliberate bias — a reader who has not been told about them will draw a
/// conclusion the data does not support.
pub const CAVEAT_HTML: &str = r##"<div class="caveat">
<strong>How to read these numbers.</strong>
<ul>
<li><b>Contributors are keyed on email address.</b> One person committing under
several addresses &mdash; a work address and a GitHub noreply, say &mdash; counts as
several contributors. That inflates contributor counts and deflates every per-human
figure. The bias applies evenly across the whole history, so trends hold even where
the absolute level is soft.</li>
<li><b>Co-authors count as contributors.</b> A <code>Co-authored-by</code> trailer
credits its subject in full, so a commit written with an agent &mdash; or landed by
one on someone's behalf &mdash; puts those people in the count. This is deliberate:
attribution is exactly where AI-assisted work shows up.</li>
<li><b>Merge commits are excluded</b> from every metric, so totals here will not
match a plain <code>git rev-list</code> count on a repo that merges rather than
rebases.</li>
<li><b>&ldquo;File touches&rdquo; is not a count of files.</b> A file changed in
fifty commits is fifty touches. Distinct file counts appear only in the folder-size
view.</li>
<li><b>Line counts include blanks and comments by default</b>, because that is what
git counts. The <i>count lines</i> control drops whitespace, or whitespace and
comments as well. Comment and code are told apart by a lexical rule, not a
parser: reading a whole file it is exact, but a diff shows only a hunk, so a block
comment opened outside that hunk can be missed. It under-counts comments rather than
mistaking code for one, and the totals themselves always stay as git reported
them.</li>
<li><b>Only the default branch is read.</b> Feature branches, local commits and
uncommitted work never appear.</li>
</ul>
</div>"##;

pub const CHART_JS: &str = r##"
const LIGHT=['#00846d','#ad5527','#3465c8','#6d7a15','#a83a8e','#9a4a2f'];
const DARK =['#2ea88f','#c9743f','#6b8ae0','#8b9a2c','#cc63b4','#bf6a52'];
function isDark(){
  const t=document.documentElement.getAttribute('data-theme');
  if(t) return t==='dark';
  return matchMedia('(prefers-color-scheme: dark)').matches;
}
// Fixed order, never cycled: a series keeps its colour when the filter changes the
// set, and a 7th series folds into the neutral rather than inventing a hue.
function color(i){const p=isDark()?DARK:LIGHT; return i<p.length?p[i]:(isDark()?'#6c7873':'#8a9490');}
function niceStep(raw){
  if(!(raw>0))return 1;
  const mag=Math.pow(10,Math.floor(Math.log10(raw)));
  const f=raw/mag;
  return (f<=1?1:f<=2?2:f<=5?5:10)*mag;
}
function fmt(n){
  if(n===null||n===undefined||isNaN(n))return '—';
  const a=Math.abs(n);
  if(a>=1e9)return (n/1e9).toFixed(1)+'B';
  if(a>=1e6)return (n/1e6).toFixed(1)+'M';
  if(a>=1e3)return Math.round(n).toLocaleString();
  return (Math.round(n*100)/100).toLocaleString();
}
function esc(s){return String(s).replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));}

let TIP=null;
function tip(){ if(!TIP){TIP=document.createElement('div');TIP.className='tip';document.body.appendChild(TIP);} return TIP; }
function showTip(html,x,y){const t=tip();t.innerHTML=html;t.classList.add('on');
  t.style.left=Math.min(x+14,innerWidth-t.offsetWidth-10)+'px';
  t.style.top=Math.max(y-t.offsetHeight-12,8)+'px';}
function hideTip(){ if(TIP)TIP.classList.remove('on'); }

function renderSeries(el,d){
  const ov=d.overlay&&d.overlay.points&&d.overlay.points.some(v=>v>0)?d.overlay:null;
  const W=1000,H=340,mL=58,mR=ov?62:16,mT=ov?32:14,mB=30;
  const n=d.x.length, S=d.series.filter(s=>s.points.some(v=>v>0)) ;
  if(!n||!S.length){el.innerHTML='<div class="empty">No data in this range.</div>';return;}
  const iw=W-mL-mR, ih=H-mT-mB;
  let max=0;
  if(d.stacked){for(let i=0;i<n;i++){let s=0;for(const q of S)s+=q.points[i]||0;max=Math.max(max,s);}}
  else{for(const q of S)for(const v of q.points)max=Math.max(max,v||0);}
  if(max<=0)max=1;
  const X=i=> n===1?mL+iw/2 : mL+(i/(n-1))*iw;
  const Y=v=> mT+ih-(v/max)*ih;
  // A second, independent scale. Where the overlay crosses the bands below is a
  // consequence of these two ranges and means nothing on its own.
  let omax=0; if(ov)for(const v of ov.points)omax=Math.max(omax,v||0);
  const oStep=ov?niceStep(omax/4):1;
  if(ov)omax=Math.max(Math.ceil(omax/oStep)*oStep,oStep);
  const Y2=v=> mT+ih-(v/omax)*ih;

  let g='';
  // Round the axis to human numbers (1/2/5 x 10^n) and extend the scale to the
  // next one, so ticks read 600/400/200 rather than 638/510.4/382.8.
  const yStep=niceStep(max/5);
  max=Math.ceil(max/yStep)*yStep;
  const TICKS=Math.round(max/yStep);
  for(let t=0;t<=TICKS;t++){
    const v=yStep*t, y=Y(v);
    g+=`<line x1="${mL}" y1="${y}" x2="${W-mR}" y2="${y}" stroke="var(--grid)" stroke-width="1"/>`;
    g+=`<text x="${mL-8}" y="${y+4}" text-anchor="end" font-size="11" fill="var(--faint)" class="mono">${fmt(v)}</text>`;
  }
  if(ov){
    const oticks=Math.round(omax/oStep);
    for(let t=0;t<=oticks;t++){
      const v=oStep*t, y=Y2(v);
      g+=`<text x="${W-mR+8}" y="${y+4}" text-anchor="start" font-size="11" fill="var(--text)" class="mono">${fmt(v)}</text>`;
    }
    // Sit the titles clear of the topmost tick label rather than on it, and keep
    // the right-hand one inside the viewBox.
    g+=`<text x="${W-4}" y="12" text-anchor="end" font-size="10" fill="var(--text)">${esc(d.overlay_label||ov.name)} →</text>`;
    g+=`<text x="4" y="12" text-anchor="start" font-size="10" fill="var(--faint)">← ${esc(d.y_label||'')}</text>`;
  }
  const step=Math.max(1,Math.ceil(n/7));
  for(let i=0;i<n;i+=step){
    g+=`<text x="${X(i)}" y="${H-8}" text-anchor="middle" font-size="11" fill="var(--faint)" class="mono">${esc(d.x[i])}</text>`;
  }

  let marks='';
  if(d.stacked){
    const base=new Array(n).fill(0);
    S.forEach((s,si)=>{
      let top='',bot='';
      for(let i=0;i<n;i++){const v=base[i]+(s.points[i]||0);top+=`${X(i)},${Y(v)} `;}
      for(let i=n-1;i>=0;i--){bot+=`${X(i)},${Y(base[i])} `;}
      // 2px surface gap between bands keeps adjacent fills readable.
      marks+=`<polygon points="${top}${bot}" fill="${color(si)}" fill-opacity="0.85"
              stroke="var(--surface)" stroke-width="1.5"/>`;
      for(let i=0;i<n;i++)base[i]+=(s.points[i]||0);
    });
  }else{
    S.forEach((s,si)=>{
      let pts='';
      for(let i=0;i<n;i++)pts+=`${X(i)},${Y(s.points[i]||0)} `;
      marks+=`<polyline points="${pts}" fill="none" stroke="${color(si)}" stroke-width="2"
              stroke-linejoin="round" stroke-linecap="round"/>`;
    });
  }

  if(ov){
    let pts='';
    for(let i=0;i<n;i++)pts+=`${X(i)},${Y2(ov.points[i]||0)} `;
    // Ink rather than a palette hue, and dashed: it reads as a reference line on
    // its own axis instead of another band in the stack.
    marks+=`<polyline points="${pts}" fill="none" stroke="var(--text)" stroke-width="2"
            stroke-dasharray="6 4" stroke-linejoin="round" stroke-linecap="round" opacity="0.75"/>`;
  }
  el.innerHTML=`<svg viewBox="0 0 ${W} ${H}" role="img"
      aria-label="${esc(d.title)}">${g}${marks}
      <line id="cross" x1="0" y1="${mT}" x2="0" y2="${mT+ih}" stroke="var(--faint)"
        stroke-width="1" stroke-dasharray="3 3" opacity="0"/></svg>
    <div class="legend">${S.map((s,i)=>
      `<span><i style="background:${color(i)}"></i>${esc(s.name)}</span>`).join('')
      +(ov?`<span><i style="background:var(--text);opacity:.75"></i>${esc(ov.name)} <em style="font-style:normal;opacity:.7">(right axis)</em></span>`:'')}</div>`;

  const svg=el.querySelector('svg'), cross=svg.querySelector('#cross');
  svg.addEventListener('mousemove',e=>{
    const r=svg.getBoundingClientRect();
    const px=(e.clientX-r.left)/r.width*W;
    let i=Math.round(((px-mL)/iw)*(n-1));
    i=Math.max(0,Math.min(n-1,i));
    cross.setAttribute('x1',X(i));cross.setAttribute('x2',X(i));cross.setAttribute('opacity','1');
    let tot=0; const lines=S.map((s,si)=>{const v=s.points[i]||0;tot+=v;
      return `<div><span style="color:${color(si)}">■</span> ${esc(s.name)} ${fmt(v)}</div>`;}).join('');
    const otip=ov?`<div>▦ ${esc(ov.name)} ${fmt(ov.points[i]||0)}</div>`:'';
    showTip(`<b>${esc(d.x[i])}</b>${lines}${S.length>1?`<div style="opacity:.65">total ${fmt(tot)}</div>`:''}${otip}`,e.clientX,e.clientY);
  });
  svg.addEventListener('mouseleave',()=>{cross.setAttribute('opacity','0');hideTip();});
}

function renderTable(el,d){
  const drill=d.drill||[];
  const bar=drill.length?drillBar((d.scope&&d.scope.path)||''):'';
  if(!d.rows.length){el.innerHTML=bar+'<div class="empty">Nothing matched.</div>';wireDrill(el);return;}
  const bc=d.bar_column;
  const isNum=d.columns.map((_,i)=>d.rows.every(r=>typeof r[i]!=='string'));
  let max=0;
  if(bc!==null&&bc!==undefined)for(const r of d.rows)max=Math.max(max,Number(r[bc])||0);
  const head=d.columns.map((c,i)=>`<th class="${isNum[i]?'num':''}">${esc(c)}</th>`).join('')
    +((bc!==null&&bc!==undefined)?'<th></th>':'');
  const rowHtml=(r,ri)=>{
    // Only the first cell of a row that names a directory becomes a link;
    // `compare` mixes summary measures and folders in one table.
    const tds=r.map((v,i)=>{
      // Leading spaces mark a sub-measure of the row above. HTML collapses them,
      // so turn them into real indentation instead of losing the nesting.
      let indent=0, val=v;
      if(i===0&&typeof v==='string'){
        const m=v.match(/^ +/); if(m){indent=m[0].length; val=v.slice(indent);}
      }
      const cell=(i===0&&drill[ri])?drillLink(drill[ri],String(val)):(typeof val==='string'?esc(val):fmt(val));
      const style=indent?` style="padding-left:calc(.7rem + ${indent*0.6}rem)"`:'';
      return `<td class="${isNum[i]?'num mono':''}"${style}>${cell}</td>`;
    }).join('');
    let cbar='';
    if(bc!==null&&bc!==undefined){
      const pct=max>0?Math.max((Number(r[bc])||0)/max*100,0):0;
      cbar=`<td><div class="barwrap"><div class="cellbar" style="width:${pct}%"></div></div></td>`;
    }
    return `<tr>${tds}${cbar}</tr>`;
  };

  // Sections become separate tbodies with a heading, so whole-scope totals don't
  // read as the first few rows of the per-directory list.
  const secs=d.sections||[];
  const ncol=d.columns.length+((bc!==null&&bc!==undefined)?1:0);
  let body;
  if(secs.length){
    body=secs.map((sc,si)=>{
      const end=si+1<secs.length?secs[si+1].start:d.rows.length;
      const inner=d.rows.slice(sc.start,end).map((r,i)=>rowHtml(r,sc.start+i)).join('');
      return `<tbody class="sec"><tr class="sechead"><th colspan="${ncol}">${esc(sc.label)}</th></tr>${inner}</tbody>`;
    }).join('');
  }else{
    body=`<tbody>${d.rows.map(rowHtml).join('')}</tbody>`;
  }
  el.innerHTML=bar+`<div class="tbl-scroll"><table><thead><tr>${head}</tr></thead>${body}</table></div>`;
  wireDrill(el);
}

function renderTree(el,d){
  const root=d.root;
  const cur=root&&root.name!=='/'?root.name:'';
  const bar=drillBar(cur);
  // "size" holds whichever measure was asked for; only bytes want a unit.
  const sz=n=>d.measure==='bytes'?bytes(n):(d.measure==='sloc'?fmt(n)+' lines':fmt(n));
  if(!root||!root.children||!root.children.length){
    el.innerHTML=bar+'<div class="empty">Nothing to show at this path.</div>';wireDrill(el);return;}
  const SZ=560,R=SZ/2,ring=Math.min(58,R/4);
  let arcs='';
  function arc(cx,cy,r0,r1,a0,a1){
    const big=(a1-a0)>Math.PI?1:0;
    const p=(r,a)=>[cx+r*Math.cos(a),cy+r*Math.sin(a)];
    const [x0,y0]=p(r1,a0),[x1,y1]=p(r1,a1),[x2,y2]=p(r0,a1),[x3,y3]=p(r0,a0);
    return `M${x0} ${y0}A${r1} ${r1} 0 ${big} 1 ${x1} ${y1}L${x2} ${y2}A${r0} ${r0} 0 ${big} 0 ${x3} ${y3}Z`;
  }
  let items=[];
  function walk(nodes,depth,a0,a1,total,ci,prefix){
    if(depth>3||total<=0)return;
    let a=a0;
    nodes.forEach((c,i)=>{
      const full=prefix?prefix+'/'+c.name:c.name;
      const span=(c.size/total)*(a1-a0);
      if(span<=0.004)return;   // below ~0.2 degrees is a hairline nobody can hit
      const col=depth===0?color(i%6):color(ci%6);
      const r0=ring*(depth+1), r1=ring*(depth+2);
      arcs+=`<path class="${c.dir?'dr':''}" d="${arc(R,R,r0,r1,a,a+span)}" fill="${col}"
        fill-opacity="${1-depth*0.22}" stroke="var(--surface)" stroke-width="1.5"
        data-i="${items.length}"/>`;
      items.push({name:c.name,size:c.size,files:c.files,depth,path:full,dir:!!c.dir});
      if(c.children&&c.children.length)walk(c.children,depth+1,a,a+span,c.size,depth===0?i:ci,full);
      a+=span;
    });
  }
  walk(root.children,0,-Math.PI/2,Math.PI*1.5,root.size,0,cur);

  const top=root.children.slice(0,12);
  el.innerHTML=bar+`<div style="display:flex;gap:1.6rem;flex-wrap:wrap;align-items:flex-start">
    <svg viewBox="0 0 ${SZ} ${SZ}" style="max-width:${SZ}px;flex:1 1 340px">${arcs}
      <text x="${R}" y="${R-2}" text-anchor="middle" font-size="15" fill="var(--text)">${esc(root.name)}</text>
      <text x="${R}" y="${R+16}" text-anchor="middle" font-size="12" fill="var(--faint)" class="mono">${fmt(root.files)} files</text>
    </svg>
    <div style="flex:1 1 300px;min-width:280px">
      <table><thead><tr><th>entry</th><th class="num">size</th><th class="num">files</th><th class="num">%</th></tr></thead>
      <tbody>${top.map((c,i)=>`<tr><td><span style="color:${color(i%6)}">■</span> ${
          c.dir?drillLink(cur?cur+'/'+c.name:c.name,c.name):esc(c.name)}</td>
        <td class="num mono">${sz(c.size)}</td><td class="num mono">${fmt(c.files)}</td>
        <td class="num mono">${(c.size/root.size*100).toFixed(1)}%</td></tr>`).join('')}</tbody></table>
    </div></div>`;

  el.querySelectorAll('path').forEach(p=>{
    p.addEventListener('mousemove',e=>{
      const it=items[+p.dataset.i];
      showTip(`<b>${esc(it.name)}</b><div>${sz(it.size)} · ${fmt(it.files)} files</div>
        <div style="opacity:.65">${(it.size/root.size*100).toFixed(1)}% of ${esc(root.name)}</div>`,e.clientX,e.clientY);
    });
    p.addEventListener('mouseleave',hideTip);
    const it=items[+p.dataset.i];
    if(it&&it.dir&&drillable())p.addEventListener('click',()=>window.__drill(it.path));
  });
  wireDrill(el);
}
function bytes(b){const U=['B','KB','MB','GB','TB'];let v=b,i=0;
  while(v>=1024&&i<U.length-1){v/=1024;i++;}
  return i===0?b+' B':v.toFixed(1)+' '+U[i];}

// The commit a view was computed from, linked out to the forge when we know the
// remote. Text-only fallback keeps it useful for a repo with no origin.
function sourceHtml(d){
  const s=d&&d.source; if(!s)return '';
  const sha=s.url?`<a href="${esc(s.url)}" target="_blank" rel="noopener">${esc(s.short)}</a>`
                 :`<span class="mono">${esc(s.short)}</span>`;
  return `<span class="src">${esc(s.repo)} @ <span class="mono">${sha}</span></span>`;
}

// The header as links rather than a formatted string: repo names go to the forge,
// and each date boundary goes to the commit it actually resolves to.
function scopeHtml(d){
  const sc=d&&d.scope;
  if(!sc)return esc((d&&d.subtitle)||'');
  const repos=sc.repos.map(r=>r.url
    ?`<a href="${esc(r.url)}" target="_blank" rel="noopener">${esc(r.name)}</a>`
    :esc(r.name)).join(', ')||'no repos';
  const dl=x=>{
    if(!x)return '';
    if(!x.url)return esc(x.date);
    const why=x.approximate?'nearest commit inside the range':'commit on this date';
    return `<a href="${esc(x.url)}" target="_blank" rel="noopener" title="${esc(x.sha.slice(0,8))} — ${why}">`
      +esc(x.date)+(x.approximate?'*':'')+'</a>';
  };
  const path=sc.path?` · ${esc(sc.path)}`:'';
  // A tree is a snapshot at one commit, not a range, so a date range would be
  // meaningless here; its subtitle carries the measure summary instead.
  if(d.kind==='tree'){
    return `${repos}${path}${d.subtitle?' · '+esc(d.subtitle):''}`;
  }
  let when='full history';
  if(sc.since&&sc.until)when=`${dl(sc.since)} → ${dl(sc.until)}`;
  else if(sc.since)when=`since ${dl(sc.since)}`;
  else if(sc.until)when=`through ${dl(sc.until)}`;
  return `${repos} · ${when}${path}`;
}

// Drilling is only wired up in the served app. A standalone exported page has no
// server to ask, so folders render as plain text there.
function drillable(){ return typeof window.__drill==='function'; }
function parentOf(p){ const i=p.lastIndexOf('/'); return i<0?'':p.slice(0,i); }
function drillBar(cur){
  if(!drillable())return '';
  return `<div class="drillbar"><button type="button" data-up="${esc(parentOf(cur))}"${cur?'':' disabled'}
    title="up one folder">..</button><span class="cur mono">${esc(cur||'(repo root)')}</span></div>`;
}
function drillLink(path,label){
  if(!drillable()||!path)return esc(label);
  return `<a class="dr" href="#" data-drill="${esc(path)}">${esc(label)}</a>`;
}
function wireDrill(el){
  if(!drillable())return;
  el.querySelectorAll('[data-up]').forEach(b=>b.onclick=()=>window.__drill(b.dataset.up));
  el.querySelectorAll('[data-drill]').forEach(a=>a.onclick=e=>{
    e.preventDefault(); window.__drill(a.dataset.drill);
  });
}

function render(el,d){
  if(d.kind==='series')return renderSeries(el,d);
  if(d.kind==='table')return renderTable(el,d);
  if(d.kind==='tree')return renderTree(el,d);
  el.innerHTML='<div class="empty">Unknown view.</div>';
}
matchMedia('(prefers-color-scheme: dark)').addEventListener('change',()=>{
  if(window.__rerender)window.__rerender();
});
"##;

fn json_for_script(o: &Output) -> String {
    // `</script>` inside a data blob would close the tag early; escaping `<` is the
    // simplest thing that cannot go wrong.
    serde_json::to_string(o)
        .unwrap_or_else(|_| "null".into())
        .replace('<', "\\u003c")
}

/// A single view, inlined into one file. No network at render time, so it can be
/// committed, attached to a PR, or emailed and it still works.
pub fn page(o: &Output) -> String {
    let cmd: Vec<String> = std::env::args().collect();
    format!(
        r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} — repo-metrics</title>
<style>{css}</style></head>
<body><div class="wrap">
<h1>{title}</h1>
<div class="head"><p class="sub" id="scope">{sub}</p><span id="src"></span></div>
<div class="card"><div id="chart"></div></div>
{caveat}
<p class="foot">Generated by <code>{cmd}</code></p>
</div>
<script>{js}
const DATA={data};
window.__rerender=()=>{{
  render(document.getElementById('chart'),DATA);
  document.getElementById('src').innerHTML=sourceHtml(DATA);
  document.getElementById('scope').innerHTML=scopeHtml(DATA);
}};
window.__rerender();
</script></body></html>"##,
        title = html_escape(o.title()),
        sub = html_escape(o.subtitle()),
        css = CSS,
        js = CHART_JS,
        caveat = CAVEAT_HTML,
        data = json_for_script(o),
        cmd = html_escape(&cmd.join(" ")),
    )
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
