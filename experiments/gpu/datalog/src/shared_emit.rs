//! Print the checked common traversal and one projection/suffix per sink.
use super::{Access, Atom, RoundShared};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use syn::Ident;

fn var(n: usize) -> Ident { format_ident!("v{n}") }
fn dir(n: usize) -> Ident { format_ident!("src{n}") }
fn reads(atoms: &[Atom], base: usize) -> (Vec<TokenStream>,TokenStream,TokenStream) {
    let mut params = Vec::new();
    let mut words = Vec::new();
    let mut bounds = Vec::new();
    for (offset,a) in atoms.iter().enumerate() {
        let src = dir(base + offset);
        let w = format_ident!("W{}",base+offset);
        params.push(match a.access {
            Access::Hash => quote!(#src: ::gpu_datalog_runtime::shared_prefix::Directory),
            _ => {
                words.push(&a.word);
                bounds.push(quote!(#w: ::gpu_datalog_runtime::Word + Copy));
                quote!(#src: ::gpu_datalog_runtime::RadixLayoutView<#w,u64>)
            }
        });
    }
    if words.is_empty() { (params,quote!(),quote!()) }
    else { (params,quote!(<#(#bounds),*>),quote!(::<#(#words),*>)) }
}

fn loops(atoms: &[Atom], start: usize, mut body: TokenStream) -> TokenStream {
    for (offset,a) in atoms.iter().enumerate().rev() {
        let src = dir(start + offset);
        let cols = a.vars.len();
        let vars: Vec<_> = a.vars.iter().map(|&v| var(v)).collect();
        let keys = a.keys;
        let load = |index: TokenStream| vars.iter().enumerate().skip(keys).map(|(c,v)| {
            let c = c as u32;
            quote!(let #v = decode_key(#src, r, #index, #c) as u32;)
        }).collect::<TokenStream>();
        body = match a.access {
            Access::Scan => {
                let load = load(quote!(i as u64));
                quote! { let mut r = 0; while r < #src.keys.run_count {
                    let n = run(#src.keys,r).keys_len as usize;
                    let mut i = shared_tid(); while i < n {
                        #load #body i += shared_stride();
                    } r += 1;
                }}
            }
            Access::Radix => {
                let fields = vars[..keys].iter();
                let n = keys as u32;
                let load = load(quote!(row));
                quote! {{let values = [#(#fields as i64),*];
                    let (low,high,fits) = encode_prefix(#src,values.as_ptr(),#n);
                    if fits { let mut r = 0; while r < #src.keys.run_count {
                        let (mut row,end) = interval(run(#src.keys,r),low,high);
                        while row < end { #load #body row += 1; }
                        r += 1;
                    }}
                }}
            }
            Access::Hash => {
                let key = |i: usize| vars.get(i).filter(|_| i < keys).map_or(quote!(0), |v| quote!(#v));
                let (a,b,c) = (key(0),key(1),key(2));
                let n = keys as u32;
                let eqs = vars[..keys].iter().enumerate().map(|(i,v)| {let j=i+1; quote!(*row.add(#j) == #v)});
                let loads = vars.iter().enumerate().skip(keys).map(|(i,v)| {let j=i+1; quote!(let #v = *row.add(#j);)});
                let stride = cols + 1;
                quote! {{let h = shared_hash(#a,#b,#c,#n);
                    let mut slot = *#src.heads.add((h & #src.mask) as usize);
                    while slot != 0 { let row = #src.rows.add((slot as usize-1)*#stride); slot = *row;
                        if #(#eqs)&&* { #(#loads)* #body }
                    }
                }}
            }
        };
    }
    body
}

/// Definitions/cache lifetime are component-local. `prepare` lives in exactly
/// one epoch; branch launches borrow the validated witness buffer on replays.
pub(crate) fn lower(group: &RoundShared, heads: &std::collections::BTreeSet<usize>) -> (TokenStream,TokenStream,BTreeMap<usize,TokenStream>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1,Ordering::Relaxed);
    let module = format_ident!("__cq_round_shared_{id}");
    let instance = format_ident!("__cq_round_shared_module_{id}");
    let directories = format_ident!("__cq_round_shared_tables_{id}");
    let hint = format_ident!("__cq_round_shared_hint_{id}");
    let bindings = format_ident!("__cq_round_shared_witnesses_{id}");
    let prefix = format_ident!("round_shared_prefix_{id}");
    let prefix_prepare = format_ident!("prepare_{prefix}");
    let width = group.live.len();
    let stores = group.live.iter().enumerate().map(|(i,&v)| {let v=var(v);quote!(*p.add(#i) = #v;)});
    let mut defs = TokenStream::new();
    let mut prepare = TokenStream::new();
    let mut kernels = TokenStream::new();
    let attr = quote!(#[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]);
    let body = loops(&group.prefix,0,quote! {
        let slot = DeviceAtomicU64::from_ptr(cursor).fetch_add(1,Relaxed);
        if slot < cap { let p = out.add(slot as usize * #width); #(#stores)* }
    });
    let (params,bounds,prefix_words) = reads(&group.prefix,0);
    kernels.extend(quote! { #attr pub fn #prefix #bounds(#(#params,)* out: *mut u32, cap:u64, cursor:*mut u64) { unsafe { #body } } });
    // Use unique local names outside each kernel's ABI: several groups may be
    // live simultaneously, and a branch must not observe another group's view.
    let mut bind = |a: &Atom,i: usize| {
        let arg = format_ident!("__cq_round_shared_input_{id}_{i}");
        let host = &a.host;
        if a.access == Access::Hash {
            let slot = format_ident!("__cq_round_shared_index_{id}_{i}");
            let keys = a.keys as u32;
            defs.extend(quote!(let mut #slot = ::gpu_datalog_runtime::shared_prefix::IndexSlot::default();));
            prepare.extend(quote!(let #arg = #slot.get(&#directories, &__cq_gpu_pool, &#host, #keys)?;));
        } else {
            prepare.extend(quote!(let #arg = #host.view_on(&__cq_gpu_pool)?;));
        }
        arg
    };
    let prefix_inputs = group.prefix.iter().enumerate().map(|(i,a)| bind(a,i)).collect::<Vec<_>>();
    let mut next = group.prefix.len();
    let mut branches = BTreeMap::new();
    for sink in &group.sinks {
        let direct = sink.suffix.is_empty();
        let (params,bounds,words) = reads(&sink.suffix,next);
        let inputs = sink.suffix.iter().enumerate().map(|(i,a)| bind(a,i+next)).collect::<Vec<_>>();
        let proj = format_ident!("round_shared_project_{id}_{}",sink.hole);
        let project_prepare = format_ident!("prepare_{proj}");
        let load = group.live.iter().enumerate().map(|(j,&v)| { let v = var(v);quote!(let #v = *p.add(#j);)});
        let a = var(sink.head[0]);
        let packed = if let Some(&b) = sink.head.get(1) {let b=var(b);quote!(u64::from(#a)<<32 | u64::from(#b))} else {quote!(u64::from(#a))};
        let write = if direct { quote!(*out.add(i) = #packed;) } else { quote!{
            let slot = DeviceAtomicU64::from_ptr(cursor).fetch_add(1,Relaxed);
            if slot < cap { *out.add(slot as usize) = #packed; }
        }};
        let body = loops(&sink.suffix,next,write);
        next += sink.suffix.len();
        kernels.extend(quote!{ #attr pub fn #proj #bounds(#(#params,)* rows:*const u32, n:usize, out:*mut u64, cap:u64, cursor:*mut u64) { unsafe {
            let mut i = shared_tid(); while i<n { let p=rows.add(i*#width); #(#load)* #body i+=shared_stride(); }
        }}});
        let packing = format_ident!("__cq_gpu_packing_{}",sink.owner);
        // SAFETY: the emitted suffix's only store is `write` above, using
        // this packing, either at the witness position or a bounded reservation.
        let project = quote! { unsafe { ::gpu_datalog_runtime::shared_prefix::project(__cq_gpu_arena, &#packing, #bindings.len, #direct, |out,cap,cursor| {
            #instance.#proj(__cq_gpu_pool.stream(), &#instance.#project_prepare #words (::gpu_datalog_runtime::shared_prefix::grid(#bindings.len))?,
                #(#inputs,)* #bindings.ptr(), #bindings.len, out, cap, cursor)?;
            Ok(())
        })? } };
        branches.insert(sink.hole,if heads.contains(&sink.hole) {quote!(Some(#project))} else {project});
    }
    let root = &group.prefix[0].host;
    // SAFETY: `stores` writes each live column once at the validated slot;
    // the bounded store and unconditional count run on this pool's stream.
    prepare.extend(quote!{
        let #bindings = unsafe { ::gpu_datalog_runtime::shared_prefix::materialize(&__cq_gpu_pool,#width,&mut #hint,#root.is_empty(),|out,cap,cursor| {
            #instance.#prefix(__cq_gpu_pool.stream(), &#instance.#prefix_prepare #prefix_words (::gpu_datalog_runtime::shared_prefix::grid(#root.len()))?,
                #(#prefix_inputs,)* out,cap,cursor)?;
            Ok(())
        })? };
    });
    let tables = (!defs.is_empty()).then(|| quote!(let #directories = ::gpu_datalog_runtime::shared_prefix::Directories::new(&__cq_gpu_pool)?;));
    (quote! {
        #[::cuda_device::cuda_module]
        mod #module {
            use ::cuda_device::{kernel,launch_bounds,launch_contract,atomic::{AtomicOrdering::Relaxed, DeviceAtomicU64}};
            use ::gpu_datalog_runtime::{radix_run::device::{decode_key,encode_prefix,interval,run},shared_prefix::{shared_hash,shared_tid,shared_stride}};
            #kernels
        }
        let #instance = unsafe { #module::load(__cq_gpu_pool.context())? };
        let mut #hint = ::gpu_datalog_runtime::shared_prefix::WitnessHint::default();
        #tables
        #defs
    },prepare,branches)
}
