//! Target-owned transport for host handles captured by device Rust callables.
//! This is checked seam data, not parsed or printed query grammar.
use cq_lang_sequence::gpu::realization::Capture;
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, Ident, Type};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureHandle {
    pub name: Ident,
    pub host_type: Type,
    pub host_value: Expr,
    pub device_type: Type,
}

/// The owning handle remains the binding visible to every host printer. Only
/// the indexed closed arguments of device callables receive its device carrier.
#[derive(Clone, Debug, Default)]
pub struct DeviceCaptureBindings {
    handles: Vec<CaptureHandle>,
    arguments: BTreeMap<(usize, usize), usize>,
}

fn error(message: &str) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}

impl DeviceCaptureBindings {
    pub fn bind(records: &[Capture], handles: Vec<CaptureHandle>) -> syn::Result<Self> {
        let mut arguments = BTreeMap::new();
        for (capture, record) in records.iter().enumerate() {
            if let Capture::Callable { closed, .. } = record {
                for (argument, expression) in closed.iter().enumerate() {
                    if let Expr::Path(path) = expression {
                        if let Some(handle) = handles.iter().position(|handle| path.path.is_ident(&handle.name)) {
                            arguments.insert((capture, argument), handle);
                        }
                    }
                }
            }
        }
        let bindings = Self { handles, arguments };
        bindings.check(records)?;
        Ok(bindings)
    }

    /// Formal rule GPU-HandleCarrier: each declared owning-handle operand in a
    /// device callable has exactly its declared device-carrier binding.
    pub fn check(&self, records: &[Capture]) -> syn::Result<()> {
        let mut names = BTreeSet::new();
        for handle in &self.handles {
            if !names.insert(handle.name.to_string()) {
                return Err(error("duplicate host-handle capture declaration"));
            }
        }
        for (&(capture, argument), &handle) in &self.arguments {
            let Some(Capture::Callable { closed, .. }) = records.get(capture) else {
                return Err(error("device handle binding does not name a callable capture"));
            };
            let (Some(expression), Some(declaration)) = (closed.get(argument), self.handles.get(handle)) else {
                return Err(error("device handle binding index is outside its records"));
            };
            if !matches!(expression, Expr::Path(path) if path.path.is_ident(&declaration.name)) {
                return Err(error("device carrier differs from the captured owning handle"));
            }
        }
        for (capture, record) in records.iter().enumerate() {
            if let Capture::Callable { closed, reads, .. } = record {
                if reads.iter().any(|(name, _)| names.contains(&name.to_string())) {
                    return Err(error("host handle cannot shadow a row input"));
                }
                for (argument, expression) in closed.iter().enumerate() {
                    let expected = match expression {
                        Expr::Path(path) => self.handles.iter().position(|handle| path.path.is_ident(&handle.name)),
                        _ => None,
                    };
                    if expected.is_none() && cq_lang_structures::syntax::rust::free_variables(expression)
                        .names.iter().any(|name| names.contains(name))
                    {
                        return Err(error("host-owned handles must cross as declared capture operands"));
                    }
                    if self.arguments.get(&(capture, argument)).copied() != expected {
                        return Err(error("device callable would receive an owning host handle"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Formal rule GPU-HostHandle: a host printer receives the owning binding,
    /// never the device carrier. Rust also checks each initializer against this
    /// explicit host type and HostDeviceHandle's associated carrier type.
    pub fn check_host_bindings(&self, bindings: &[(Ident, Type)]) -> syn::Result<()> {
        if bindings.len() != self.handles.len() { return Err(error("incomplete host-handle bindings")); }
        let mut seen = BTreeSet::new();
        for (name, ty) in bindings {
            let Some(handle) = self.handles.iter().find(|handle| handle.name == *name) else {
                return Err(error("host printer received an undeclared handle"));
            };
            if !seen.insert(name.to_string()) || *ty != handle.host_type {
                return Err(error("host printer received a device carrier instead of its owning handle"));
            }
        }
        Ok(())
    }

    pub fn host_setup(&self) -> syn::Result<proc_macro2::TokenStream> {
        let bindings: Vec<_> = self.handles.iter().map(|handle| (handle.name.clone(), handle.host_type.clone())).collect();
        self.check_host_bindings(&bindings)?;
        let declarations = self.handles.iter().map(|handle| {
            let CaptureHandle { name, host_type, host_value, device_type } = handle;
            quote! {
                let _: ::core::marker::PhantomData<#device_type> =
                    ::core::marker::PhantomData::<<#host_type as ::gpu_device_columns::HostDeviceHandle>::Device>;
                let #name: #host_type = #host_value;
            }
        });
        Ok(quote!(#(#declarations)*))
    }

    pub(crate) fn operand(&self, capture: usize, argument: usize) -> Option<proc_macro2::TokenStream> {
        self.arguments.get(&(capture, argument)).map(|&index| {
            let CaptureHandle { name, host_type, device_type, .. } = &self.handles[index];
            quote!({
                let __cq_device_capture: #device_type =
                    <#host_type as ::gpu_device_columns::HostDeviceHandle>::device(&#name);
                __cq_device_capture
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Vec<Capture>, Vec<CaptureHandle>) {
        (vec![Capture::Callable {
            reads: vec![(syn::parse_quote!(row), syn::parse_quote!(u32))],
            output: syn::parse_quote!(i64),
            closed: vec![syn::parse_quote!(handle)],
            body: syn::parse_quote!(unsafe { __cq_closed_0.assume_init().value(*row) }),
        }], vec![CaptureHandle {
            name: syn::parse_quote!(handle), host_type: syn::parse_quote!(&HostHandle),
            host_value: syn::parse_quote!(&owned), device_type: syn::parse_quote!(DeviceView),
        }])
    }
    #[test]
    fn owning_and_device_carriers_cannot_be_interchanged() {
        let (records, handles) = fixture();
        let bindings = DeviceCaptureBindings::bind(&records, handles).unwrap();
        assert!(bindings.check_host_bindings(&[(syn::parse_quote!(handle), syn::parse_quote!(&HostHandle))]).is_ok());
        assert!(bindings.check_host_bindings(&[(syn::parse_quote!(handle), syn::parse_quote!(DeviceView))]).is_err());
        let mut missing = bindings.clone();
        missing.arguments.clear();
        assert!(missing.check(&records).is_err());
        let mut wrong = bindings;
        wrong.arguments.insert((0, 0), 9);
        assert!(wrong.check(&records).is_err());
        let (mut records, handles) = fixture();
        let Capture::Callable { closed, .. } = &mut records[0] else { unreachable!() };
        closed[0] = syn::parse_quote!(&handle);
        assert!(DeviceCaptureBindings::bind(&records, handles).is_err());
    }
    #[test]
    fn no_handles_preserve_capture_code_byte_for_byte() {
        let (records, _) = fixture();
        let bindings = DeviceCaptureBindings::bind(&records, Vec::new()).unwrap();
        let before = crate::capture::print(&records, None);
        let after = crate::capture::print(&records, Some(&bindings));
        assert_eq!(before.setup.to_string(), after.setup.to_string());
        assert_eq!(before.parameters.iter().map(|(_, ty, value)| quote!(#ty #value).to_string()).collect::<Vec<_>>(),
            after.parameters.iter().map(|(_, ty, value)| quote!(#ty #value).to_string()).collect::<Vec<_>>());
        assert_eq!(before.rust_functions.iter().map(|(name, bounds, value)| quote!(#name #bounds #value).to_string()).collect::<Vec<_>>(),
            after.rust_functions.iter().map(|(name, bounds, value)| quote!(#name #bounds #value).to_string()).collect::<Vec<_>>());
        assert_eq!(before.disabled.iter().map(|(id, value)| (*id, value.to_string())).collect::<Vec<_>>(),
            after.disabled.iter().map(|(id, value)| (*id, value.to_string())).collect::<Vec<_>>());
    }
}
