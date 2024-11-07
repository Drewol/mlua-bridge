extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{ImplItem, ItemImpl, PatType, ReturnType, Signature, Type};

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

fn split_appdata_args(sig: &Signature) -> (Vec<PatType>, Vec<PatType>) {
    sig.inputs
        .iter()
        .filter_map(|x| {
            if let syn::FnArg::Typed(t) = x {
                Some(t)
            } else {
                None
            }
        })
        .cloned()
        .partition(|a| {
            if let Type::Path(p) = a.ty.as_ref() {
                p.path
                    .segments
                    .iter()
                    .any(|p| p.ident == "AppDataRefMut" || p.ident == "AppDataRef")
            } else {
                false
            }
        })
}

#[proc_macro_attribute]
pub fn mlua_bridge(_attr: TokenStream, item: TokenStream) -> TokenStream {
    //TODO: if function returns mlua result: map it to lua
    //      How to check for correct return type?

    // if function argument is of AppDataRef(Mut) type: fetch it from lua appdata before calling function
    // if function is `get_` or `set_` and takes no arguments: map to field
    //TODO: collect type information along the way to generate luals definitions

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
    let mut fields_impl = quote! {};

    for f in funcs {
        let name = f.sig.ident.clone();
        let (app_data, lua_args) = split_appdata_args(&f.sig);
        let self_name = f.sig.ident;
        let name = quote! {stringify!(#name)};
        let rust_args: Vec<PatType> = f
            .sig
            .inputs
            .iter()
            .cloned()
            .filter_map(|x| match x {
                syn::FnArg::Receiver(_) => None,
                syn::FnArg::Typed(pat_type) => Some(pat_type),
            })
            .collect();

        let args_tup = lua_args.iter().map(|x| x.pat.clone());
        let args_typ = lua_args.iter().map(|x| x.ty.clone());
        let args_tup = quote! {(#(#args_tup),*)};
        let args_typ = quote! { (#(#args_typ),*)};
        let args = quote! {#args_tup: #args_typ};

        let rust_args = rust_args.iter().map(|x| x.pat.clone());
        let rust_args = quote! {(#(#rust_args),*)};

        let app_data = app_data.iter().map(|x| {
            let name = x.pat.to_token_stream();
            let t = x.ty.to_token_stream();
            if t.to_string().contains("AppDataRefMut") {
                quote! {let #name: #t = _lua.app_data_mut().ok_or(mlua::Error::external("AppData not set"))?; }
            }
            else {
                quote! {let #name: #t = _lua.app_data_ref().ok_or(mlua::Error::external("AppData not set"))?; }
            }
        });

        let question = match f.ret {
            MluaReturnType::Void | MluaReturnType::Primitive => quote! {},
            MluaReturnType::Result => quote! {?},
        };

        let method = match &f.takes_self {
            TakesSelf::No => quote! {add_function_mut},
            TakesSelf::Yes => quote! {add_method},
            TakesSelf::Mut => quote! {add_method_mut},
        };

        let self_ident = match &f.takes_self {
            TakesSelf::No => quote! {Self::},
            TakesSelf::Yes | TakesSelf::Mut => quote! {s.},
        };

        let closure_def = match &f.takes_self {
            TakesSelf::No => quote! { |_lua, #args| },
            TakesSelf::Yes | TakesSelf::Mut => quote! { |_lua, s, #args| },
        };

        let t = quote! {
            methods.#method(#name, #closure_def {
                #(#app_data)*

                 Ok(#self_ident #self_name #rust_args #question)
                });
        };

        t.to_tokens(&mut funcs_impl);
    }

    for f in fields {
        let name = f.sig.ident.clone();
        let name = format_ident!("{}", name.to_string()[4..]);
        let self_name = f.sig.ident.clone();
        let name = quote! {stringify!(#name)};

        let question = match &f.ret {
            MluaReturnType::Void | MluaReturnType::Primitive => quote! {},
            MluaReturnType::Result => quote! {?},
        };

        let self_ident = match &f.takes_self {
            TakesSelf::No => quote! {Self::},
            TakesSelf::Yes | TakesSelf::Mut => quote! {s.},
        };

        let t = match (&f.takes_self, &f.sig.ident.to_string().starts_with("set")) {
            (TakesSelf::No, true) => quote! {
                fields.add_field_function_set(#name, |_lua, _, v| Ok(#self_ident #self_name(v)#question));
            },
            (TakesSelf::No, false) => quote! {
                fields.add_field_function_get(#name, |_lua, _| Ok(#self_ident #self_name()#question));
            },
            (_, true) => quote! {
                fields.add_field_method_set(#name, |_lua, s, v| Ok(#self_ident #self_name(v)#question));

            },
            (_, false) => quote! {
                fields.add_field_method_get(#name, |_lua, s| Ok(#self_ident #self_name()#question));
            },
        };

        t.to_tokens(&mut fields_impl);
    }

    let item_ident = impl_item.self_ty.into_token_stream();

    let trait_impl = quote! {
        impl ::mlua::UserData for #item_ident {
            fn add_methods<M: ::mlua::UserDataMethods<Self>>(methods: &mut M) {
                #funcs_impl
            }

            fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
                #fields_impl
            }
        }
    };
    let mut item = proc_macro2::TokenStream::from(item);
    trait_impl.to_tokens(&mut item);

    item.into()
}
