//! Procedural macro companion to the `gateflow` crate.
//!
//! See the `gateflow` crate's own docs for what this actually does; this
//! crate only expands the attribute — `gateflow::Sandbox::new().enter(..)`
//! does the real work.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Error, Expr, Ident, ItemFn, Result, Token, parse::Parse, parse::ParseStream, parse_macro_input,
};

#[derive(Default)]
struct ChaosArgs {
    delay_ms: Option<Expr>,
    jitter_ms: Option<Expr>,
    loss_percent: Option<Expr>,
    reorder_percent: Option<Expr>,
    corrupt_percent: Option<Expr>,
    duplicate_percent: Option<Expr>,
}

impl Parse for ChaosArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut args = Self::default();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: Expr = input.parse()?;

            let slot = match key.to_string().as_str() {
                "delay_ms" => &mut args.delay_ms,
                "jitter_ms" => &mut args.jitter_ms,
                "loss_percent" => &mut args.loss_percent,
                "reorder_percent" => &mut args.reorder_percent,
                "corrupt_percent" => &mut args.corrupt_percent,
                "duplicate_percent" => &mut args.duplicate_percent,
                _ => {
                    return Err(Error::new(
                        key.span(),
                        "unknown gateflow chaos parameter; expected one of: delay_ms, jitter_ms, loss_percent, reorder_percent, corrupt_percent, duplicate_percent",
                    ));
                }
            };

            if slot.is_some() {
                return Err(Error::new(key.span(), "duplicate gateflow chaos parameter"));
            }
            *slot = Some(value);

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected `,` between gateflow chaos parameters"));
            }
        }

        Ok(args)
    }
}

/// Wraps a test function so its body runs inside a freshly created,
/// unprivileged network namespace instead of the host's.
///
/// # Requirements
///
/// The `gateflow` crate must be a dependency named `gateflow` (its default
/// extern crate name) with the `macros` feature enabled, since the
/// expanded code refers to it by the absolute path `::gateflow`.
///
/// # Chaos parameters
///
/// The optional parameters configure real kernel `tc netem` chaos on the
/// sandbox's loopback interface. Values are compile-time Rust expressions;
/// durations are in milliseconds and percentages are in the range `0.0..=100.0`.
/// Supported parameters are `delay_ms`, `jitter_ms`, `loss_percent`,
/// `reorder_percent`, `corrupt_percent`, and `duplicate_percent`.
///
/// ```ignore
/// #[gateflow::isolated_net(delay_ms = 100, loss_percent = 1.0)]
/// fn tolerates_a_slow_lossy_loopback() {
///     // Runs inside a namespace whose loopback has real kernel netem applied.
/// }
/// ```
///
/// `reorder_percent` requires `delay_ms` because that is a kernel netem
/// requirement. The underlying `Percent` constructor clamps percentages to
/// `0.0..=100.0`.
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
    let args = parse_macro_input!(attr as ChaosArgs);
    let input = parse_macro_input!(item as ItemFn);
    expand(input, args).into()
}

fn expand(input: ItemFn, args: ChaosArgs) -> TokenStream2 {
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
        ..
    } = input;

    let mut netem = quote!(::gateflow::chaos::NetemConfig::new());
    let mut has_chaos = false;

    if let Some(value) = args.delay_ms {
        has_chaos = true;
        netem = quote! {
            #netem.delay(::std::time::Duration::from_millis(#value))
        };
    }
    if let Some(value) = args.jitter_ms {
        has_chaos = true;
        netem = quote! {
            #netem.jitter(::std::time::Duration::from_millis(#value))
        };
    }
    for (value, method) in [
        (args.loss_percent, quote!(loss)),
        (args.reorder_percent, quote!(reorder)),
        (args.corrupt_percent, quote!(corrupt)),
        (args.duplicate_percent, quote!(duplicate)),
    ] {
        if let Some(value) = value {
            has_chaos = true;
            netem = quote! {
                #netem.#method(::gateflow::chaos::Percent::new((#value) as f64))
            };
        }
    }

    let sandbox = if has_chaos {
        quote! {
            ::gateflow::Sandbox::new().chaos(#netem.build())
        }
    } else {
        quote! { ::gateflow::Sandbox::new() }
    };

    quote! {
        #[test]
        #(#attrs)*
        #vis #sig {
            let __gateflow_exit_code = #sandbox
                .enter(move || {
                    #block
                    0
                })
                .expect("Sandbox::new().enter failed before the sandboxed body could run");

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

    use super::{ChaosArgs, expand};

    #[test]
    fn wraps_body_in_sandbox_enter() {
        let input: syn::ItemFn = parse_quote! {
            fn my_test() {
                assert!(true);
            }
        };

        let expanded = expand(input, Default::default()).to_string();

        assert!(expanded.contains("Sandbox :: new"));
        assert!(expanded.contains(". enter"));
        assert!(expanded.contains("assert_eq"));
    }

    #[test]
    fn expands_chaos_parameters_into_netem_builder() {
        let input: syn::ItemFn = parse_quote! {
            fn lossy_test() {
                assert!(true);
            }
        };
        let args: ChaosArgs = syn::parse_str(
            "delay_ms = 100, jitter_ms = 20, loss_percent = 1.5, reorder_percent = 2.0",
        )
        .expect("chaos parameters should parse");

        let expanded = expand(input, args).to_string();

        assert!(expanded.contains("NetemConfig :: new"));
        assert!(expanded.contains("from_millis (100)"));
        assert!(expanded.contains("from_millis (20)"));
        assert!(expanded.contains("Percent :: new"));
        assert!(expanded.contains(". chaos"));
    }

    #[test]
    fn rejects_unknown_chaos_parameters() {
        let error = match syn::parse_str::<ChaosArgs>("latency_ms = 100") {
            Ok(_) => panic!("unknown chaos parameter should be rejected"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("unknown gateflow chaos parameter")
        );
    }
}
