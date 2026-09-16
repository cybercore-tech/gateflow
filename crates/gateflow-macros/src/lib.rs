//! Procedural macro companion to the `gateflow` crate.
//!
//! See the `gateflow` crate's own docs for what this actually does; this
//! crate only expands the attribute — `gateflow::netns::fork_and_enter`
//! does the real work.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

/// Wraps a test function so its body runs inside a freshly created,
/// unprivileged network namespace instead of the host's.
///
/// # Requirements
///
/// The `gateflow` crate must be a dependency named `gateflow` (its default
/// extern crate name) with the `macros` feature enabled, since the
/// expanded code refers to it by the absolute path `::gateflow`.
///
/// # Known limitation
///
/// If the test body's final expression is a `#[must_use]` value without a
/// trailing `;`, expansion still compiles but that value's `must_use`
/// warning fires (the expression becomes a discarded block statement) —
/// end test bodies with `;`-terminated statements or `assert!`/`panic!`
/// calls, not a bare trailing expression, until this is addressed.
///
/// # Example
///
/// ```ignore
/// #[gateflow::isolated_net]
/// fn sees_its_own_namespace() {
///     // runs as uid 0 inside a network namespace nothing else can see
/// }
/// ```
#[proc_macro_attribute]
pub fn isolated_net(attr: TokenStream, item: TokenStream) -> TokenStream {
    let _ = attr; // no arguments accepted yet
    let input = parse_macro_input!(item as ItemFn);
    expand(input).into()
}

fn expand(input: ItemFn) -> TokenStream2 {
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
        ..
    } = input;

    quote! {
        #[test]
        #(#attrs)*
        #vis #sig {
            let __gateflow_exit_code = ::gateflow::netns::fork_and_enter(move || {
                #block
                0
            })
            .expect("fork_and_enter failed before the sandboxed body could run");

            assert_eq!(
                __gateflow_exit_code,
                0,
                "sandboxed test body failed (see child process stderr above)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::expand;

    #[test]
    fn wraps_body_in_fork_and_enter() {
        let input: syn::ItemFn = parse_quote! {
            fn my_test() {
                assert!(true);
            }
        };

        let expanded = expand(input).to_string();

        assert!(expanded.contains("fork_and_enter"));
        assert!(expanded.contains("assert_eq"));
    }
}
