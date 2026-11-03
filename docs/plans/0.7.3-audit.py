"""Create an isolated source copy with v0.7.3 audit probes; never edit production source.

Run from any directory with Python 3. The printed directory contains a Cargo
project. Run its release tests filtered by audit_073, serially, outside the
sandbox for Metal access. No dependencies are installed by this script.
"""
from pathlib import Path
import shutil
src=Path(__file__).resolve().parents[2]
import tempfile
dst=Path(tempfile.mkdtemp(prefix='litty-audit-073-'))
for name in ['src','patches','assets','resources','Cargo.toml','Cargo.lock']:
    p=src/name
    if p.is_dir(): shutil.copytree(p,dst/name,dirs_exist_ok=True)
    else: shutil.copy2(p,dst/name)
def append(name,s):
    with (dst/name).open('a') as f:f.write(s)
# Track requested bytes, counting each realloc's full requested size (not RSS).
p=dst/'src/test_alloc.rs'
s=p.read_text().replace('static COUNT:', 'static BYTES: Cell<usize> = const { Cell::new(0) };\n    static COUNT:')
s=s.replace('fn record() {','fn record(bytes: usize) {').replace('count.set(Some(n + 1));','count.set(Some(n + 1)); BYTES.with(|b| b.set(b.get() + bytes));')
s=s.replace('record();\n        unsafe { System.alloc', 'record(layout.size());\n        unsafe { System.alloc').replace('record();\n        unsafe { System.realloc', 'record(size);\n        unsafe { System.realloc')
s += '''\npub fn audit(label: &str, f: impl FnOnce()) {\n BYTES.with(|b| b.set(0)); let n=allocations(f); let bytes=BYTES.with(Cell::get);\n println!("AUDIT {label}: allocations={n} requested_bytes={bytes}");\n}\n'''
p.write_text(s)
append('src/terminal/grid/tests.rs',r'''
#[test]
fn audit_073_search_and_clusters() {
    use std::{hint::black_box, time::Instant};
    assert_eq!(std::mem::size_of::<Cell>(),12);
    for width in [20,160] {
        let mut g=Grid::new(160,24);
        let line=format!("{}\r\n", "x".repeat(width));
        for _ in 0..5024 {feed(&mut g,&line);}
        assert_eq!(g.history,5000);
        let snapshot=g.search_text();
        println!("AUDIT search width={width}: rows={} row_capacity={} columns={} column_capacity={} column_bytes={} text_len={} text_capacity={}",snapshot.rows.len(),snapshot.rows.capacity(),snapshot.columns.len(),snapshot.columns.capacity(),snapshot.columns.capacity()*24,snapshot.text.len(),snapshot.text.capacity());
        crate::test_alloc::audit("search snapshot",|| {black_box(g.search_text());});
        let mut samples=Vec::new();
        for _ in 0..31 {let t=Instant::now(); let x=g.search_text(); samples.push(t.elapsed().as_nanos());black_box(x);}
        samples.sort(); println!("AUDIT search width={width} median_ns={} min_ns={} max_ns={}",samples[15],samples[0],samples[30]);
    }
    // Same snapshot text, different stored widths: rows+text cannot recover cell mappings.
    let mut a=Grid::new(4,1); feed(&mut a,"\x1b[?2027h❤️");
    let mut c=Grid::new(1,1); feed(&mut c,"\x1b[?2027h❤️");
    c.resize(4,1);
    let wide=a.search_text();let narrow=c.search_text();
    assert_eq!(wide.text,narrow.text);
    assert_eq!(wide.rows,narrow.rows);
    let re=regex::Regex::new("❤️").unwrap();
    assert_ne!(wide.find(&re)[0].end,narrow.find(&re)[0].end);
    println!("AUDIT identical text {:?}: wide={:?} narrow={:?}",wide.text,wide.find(&re),narrow.find(&re));
    let cluster=format!("ب{}","\u{064e}".repeat(31));
    let mut g=Grid::new(80,2);feed(&mut g,&cluster);
    let stored=g.cluster(g.display_row(0)[0].codepoint).unwrap();
    assert_eq!(stored.len(),64);
    let f=crate::terminal::bidi::arabic_form(None,'ب',None);
    let joined:String=std::iter::once(f).chain(stored.chars().skip(1)).collect();
    assert_eq!(joined.len(),65);
    println!("AUDIT Arabic cluster stored={} joined={}",stored.len(),joined.len());
}
''')
append('src/ai/http.rs',r'''
#[test]
fn audit_073_quote() {
    for (label,s) in [("ASCII 500KB", "x".repeat(500_000)),("escape-heavy 500KB", "\\\"\n\rx".repeat(100_000))] {
        let result=quote(&s);
        crate::test_alloc::audit(label,|| {std::hint::black_box(quote(&s));});
        println!("AUDIT quote {label} input={} output={}",s.len(),result.len());
    }
}
''')
append('src/ai/json.rs',r'''
#[test]
fn audit_073_encode() {
    let history:Vec<_>=(0..100).map(|_|obj([("role","user".into()),("content","x".repeat(5_000).into())])).collect();
    crate::test_alloc::audit("100x5KB history encode lengths",|| {
        let n:usize=history.iter().map(|m|m.encode().len()).sum();
        assert_eq!(n,502800);
    });
}
''')
append('src/render/atlas/raster.rs',r'''
#[test]
fn audit_073_stem() {
    use std::{hint::black_box,time::Instant};
    for thick in [false,true] {
        THICKEN.store(thick,Ordering::Relaxed);
        let table:[u8;256]=std::array::from_fn(|i|stem_boost(i as u8));
        let gamma=if thick {0.55f32}else{0.78};
        for v in 0..256 {assert_eq!(table[v],((v as f32/255.0).powf(gamma)*255.0+0.5)as u8);}
        for lut in [false,true] {
            let mut samples=Vec::new();
            for _ in 0..11 {
                let t=Instant::now(); let mut sum=0u64;
                for i in 0..1_000_000 {let v=black_box(i as u8); sum+=if lut {black_box(&table)[v as usize]}else{stem_boost(v)} as u64;}
                black_box(sum);samples.push(t.elapsed().as_nanos());
            }
            samples.sort();println!("AUDIT stem thick={thick} lut={lut} million_pixels_median_ns={}",samples[5]);
        }
    }
    THICKEN.store(false,Ordering::Relaxed);
}
''')
append('src/render/renderer/ui/mod.rs',r'''
#[test]
fn audit_073_overlays() {
    use crate::render::atlas::{Atlas,load_font};
    use crate::terminal::{search::SearchBar,history::{HistorySearch,MatchResult}};
    let gpu=wgpu::Instance::default();
    let adapter=pollster::block_on(gpu.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
    let (device,queue)=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(),None)).unwrap();
    let mut atlas=Atlas::new(&device,&queue,load_font(&[]),16.0);
    let mut instances=vec![Instance::default();160*40];
    let mut dirty=DirtyRows::NONE;
    let mut cells=Cells{atlas:&mut atlas,queue:&queue,instances:&mut instances,cols:160};
    let titles=vec!["shell".into(),"a very long title that must be truncated inside the allocated tab span ".repeat(3)];
    for _ in 0..3 {tab_bar::build(&mut cells,&titles,0);}
    crate::test_alloc::audit("warm tab bar 2 tabs",||tab_bar::build(&mut cells,&titles,0));
    let mut search=SearchBar::new();search.query="needle".into();search.total=123;search.current=Some(0);
    for _ in 0..3 {search_box::build(&mut cells,2,40,&search,&mut dirty);}
    crate::test_alloc::audit("warm search box",||search_box::build(&mut cells,2,40,&search,&mut dirty));
    let mut history=HistorySearch::new();history.query="echo".into();
    for i in 0..9 {history.items.push_back(format!("echo {}", "x".repeat(200)));history.filtered.push(MatchResult{item_index:i,score:0,match_indices:vec![0,1,2,3]});}
    let geometry=history_popup::geometry(160,40).unwrap();
    for _ in 0..3 {history_popup::build(&mut cells,geometry,40,&history,&mut dirty);}
    crate::test_alloc::audit("warm history popup 9 results 205 chars",||history_popup::build(&mut cells,geometry,40,&history,&mut dirty));
}
''')
print(dst)
