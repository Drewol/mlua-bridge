extern crate proc_macro;
use proc_macro::{Ident, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, Attribute, DeriveInput, Expr, ImplItem, ItemImpl, PatType, ReturnType,
    Signature, Token, Type,
};

enum MluaReturnType {
    Void,
    Primitive,
    Result,
}

enum TakesSelf {
    No,
    Yes,
    Mut,
}

struct ExportedFn {
    ret: MluaReturnType,
    takes_self: TakesSelf,
    is_field: bool,
    sig: Signature,
}

#[proc_macro_attribute]
pub fn mlua_bridge(attr: TokenStream, mut item: TokenStream) -> TokenStream {
    //if function returns mlua result: map it to lua
    //      How to check for correct return type?

    //if function argument includes #[appdata]: fetch it from lua appdata before calling function
    //if function is `get_` or `set_` and takes no arguments: map to field
    //collect type information along the way to generate luals definitions

    let impl_item = item.clone();
    let impl_item = syn::parse_macro_input!(impl_item as ItemImpl);

    let mut fns = vec![];

    for ele in impl_item.items {
        let ImplItem::Fn(f) = ele else {
            continue;
        };

        fns.push(f);
    }

    let mut exported_fns = vec![];

    for ele in fns.into_iter() {
        let sig = ele.sig;

        let ret = match &sig.output {
            ReturnType::Default => MluaReturnType::Void,
            ReturnType::Type(_, t) => match t.as_ref() {
                Type::Path(r) => {
                    if r.path.segments.last().is_some_and(|r| r.ident == "Result") {
                        MluaReturnType::Result
                    } else {
                        MluaReturnType::Primitive
                    }
                }
                _ => MluaReturnType::Primitive,
            },
        };

        let takes_self = sig
            .inputs
            .first()
            .map(|a| match a {
                syn::FnArg::Receiver(r) => {
                    if r.mutability.is_some() {
                        TakesSelf::Mut
                    } else {
                        TakesSelf::Yes
                    }
                }
                syn::FnArg::Typed(_) => TakesSelf::No,
            })
            .unwrap_or(TakesSelf::No);

        let is_field = matches!(&sig.ident.to_string()[..4], "get_" | "set_");

        exported_fns.push(ExportedFn {
            ret,
            takes_self,
            is_field,
            sig,
        });
    }

    let (fields, funcs): (Vec<_>, Vec<_>) = exported_fns.into_iter().partition(|x| x.is_field);

    let mut funcs_impl = quote! {};

    for f in funcs {
        let name = f.sig.ident.clone();
        let self_name = f.sig.ident;
        let name = quote! {stringify!(#name)};
        let args: Vec<PatType> = f
            .sig
            .inputs
            .iter()
            .cloned()
            .filter_map(|x| match x {
                syn::FnArg::Receiver(_) => None,
                syn::FnArg::Typed(pat_type) => Some(pat_type),
            })
            .collect();

        let args_tup = args.iter().map(|x| x.pat.clone());
        let args_typ = args.iter().map(|x| x.ty.clone());
        let args_tup = quote! {(#(#args_tup),*)};
        let args_typ = quote! { (#(#args_typ),*)};
        let args = quote! {#args_tup: #args_typ};

        let question = match f.ret {
            MluaReturnType::Void => quote! {},
            MluaReturnType::Primitive => quote! {},
            MluaReturnType::Result => quote! {?},
        };

        let t = match f.takes_self {
            TakesSelf::No => {
                quote! {methods.add_function_mut(#name, |_lua, #args| Ok(Self::#self_name #args_tup)#question);}
            }
            TakesSelf::Yes => quote! {methods.add_method(#name, |_lua, s, #args| Ok(s.#self_name #args_tup)#question);},
            TakesSelf::Mut => quote! {methods.add_method_mut(#name, |_lua, s, #args| Ok(s.#self_name #args_tup)#question);},
        };

        t.to_tokens(&mut funcs_impl);
    }

    let item_ident = impl_item.self_ty.into_token_stream();

    let trait_impl = quote! {
        impl ::mlua::UserData for #item_ident {
            fn add_methods<M: ::mlua::UserDataMethods<Self>>(methods: &mut M) {
                #funcs_impl
            }
        }
    };
    let mut item = proc_macro2::TokenStream::from(item);
    trait_impl.to_tokens(&mut item);

    item.into()
}
