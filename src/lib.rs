extern crate proc_macro;

use darling::{ast::NestedMeta, FromMeta};
use ident_case::RenameRule;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{ImplItem, ItemImpl, PatType, ReturnType, Signature, Type, Visibility};

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
    is_field: FieldType,
    sig: Signature,
}

#[derive(Debug, FromMeta, Default)]
struct ImplMeta {
    #[darling(default)]
    rename_funcs: RenameRule,
    #[darling(default)]
    rename_fields: RenameRule,
    pub_only: Option<()>,
    no_auto_fields: Option<()>,
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
        .partition(|a| match a.ty.as_ref() {
            Type::Reference(_) => true,
            _ => false,
        })
}

#[derive(PartialEq)]
enum FieldType {
    None,
    Get,
    Set,
}

#[proc_macro_attribute]
pub fn mlua_bridge(attr: TokenStream, item: TokenStream) -> TokenStream {
    //TODO: if function returns mlua result: map it to lua
    //      How to check for correct return type?

    // if function argument is of AppDataRef(Mut) type: fetch it from lua appdata before calling function
    // if function is `get_` or `set_` and takes no arguments: map to field
    //TODO: collect type information along the way to generate luals definitions

    //TODO: Write out proper darling error as they're more detailed
    let impl_meta = NestedMeta::parse_meta_list(attr.into()).expect("Failed to parse attribute");
    let ImplMeta {
        pub_only,
        rename_funcs,
        rename_fields,
        no_auto_fields,
    } = ImplMeta::from_list(&impl_meta).expect("Failed to parse attribute");
    let pub_only = pub_only.is_some();

    let impl_item = item.clone();
    let impl_item = syn::parse_macro_input!(impl_item as ItemImpl);

    let mut fns = vec![];
    let mut consts = vec![];

    for ele in impl_item.items {
        match ele {
            ImplItem::Const(c) => consts.push(c),
            ImplItem::Fn(f) => fns.push(f),
            _ => continue,
        }
    }

    let mut exported_fns = vec![];

    for ele in fns.into_iter() {
        if pub_only && !matches!(ele.vis, Visibility::Public(_)) {
            continue;
        }

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

        let field_incompat_args = sig
            .inputs
            .iter()
            .filter(|x| match x {
                syn::FnArg::Receiver(_) => false,
                syn::FnArg::Typed(pat_type) => match pat_type.ty.as_ref() {
                    Type::Reference(_) => false,
                    _ => true,
                },
            })
            .count();
        let fn_name = sig.ident.to_string();
        let is_field = no_auto_fields.is_none()
            && fn_name.len() > 4
            && matches!(&fn_name[..4], "get_" | "set_");
        let is_field = if is_field {
            match &fn_name[..4] {
                "get_" if field_incompat_args == 0 => FieldType::Get,
                "set_" if field_incompat_args == 1 => FieldType::Set,
                _ => FieldType::None,
            }
        } else {
            FieldType::None
        };

        exported_fns.push(ExportedFn {
            ret,
            takes_self,
            is_field,
            sig,
        });
    }

    let (fields, funcs): (Vec<_>, Vec<_>) = exported_fns
        .into_iter()
        .partition(|x| x.is_field != FieldType::None);

    let mut funcs_impl = quote! {};
    let mut fields_impl = quote! {};

    for f in funcs {
        let name = f.sig.ident.to_token_stream();

        let name = rename_funcs
            .apply_to_field(name.to_string())
            .to_token_stream();

        let (app_data, lua_args) = split_appdata_args(&f.sig);
        let self_name = f.sig.ident;
        let name = quote! {#name};
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
            let Type::Reference(ref_type) = x.ty.as_ref() else {
                unreachable!()
            };
            let name = x.pat.to_token_stream();
            let t = ref_type.elem.to_token_stream();

            if ref_type.mutability.is_some() {
                quote! {let #name = &mut *_lua.app_data_mut::<#t>().ok_or(mlua::Error::external("AppData not set"))?; }
            }
            else {
                quote! {let #name = &*_lua.app_data_ref::<#t>().ok_or(mlua::Error::external("AppData not set"))?; }
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
        let name = rename_fields.apply_to_field(format!("{}", &name.to_string()[4..]));
        let self_name = f.sig.ident.clone();
        let name = quote! {#name};

        let (app_data, lua_args) = split_appdata_args(&f.sig);
        if lua_args.len() > 1 {
            panic!("Incalid field signature")
        }

        let value_name = lua_args
            .get(0)
            .map(|x| x.pat.clone().to_token_stream())
            .unwrap_or_default();

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
        let rust_args = rust_args.iter().map(|x| x.pat.clone());
        let rust_args = quote! {(#(#rust_args),*)};

        let app_data = app_data.iter().map(|x| {
            let Type::Reference(ref_type) = x.ty.as_ref() else {
                unreachable!()
            };
            let name = x.pat.to_token_stream();
            let t = ref_type.elem.to_token_stream();

            if ref_type.mutability.is_some() {
                quote! {let #name = &mut *_lua.app_data_mut::<#t>().ok_or(mlua::Error::external("AppData not set"))?; }
            }
            else {
                quote! {let #name = &*_lua.app_data_ref::<#t>().ok_or(mlua::Error::external("AppData not set"))?; }
            }
        });

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
                fields.add_field_function_set(#name, |_lua, _, #value_name| {
                    #(#app_data)*

                    Ok(#self_ident #self_name #rust_args #question)});
            },
            (TakesSelf::No, false) => quote! {
                fields.add_field_function_get(#name, |_lua, _| {
                    #(#app_data)*

                    Ok(#self_ident #self_name #rust_args #question)});
            },
            (_, true) => quote! {
                fields.add_field_method_set(#name, |_lua, s, #value_name| {
                    #(#app_data)*

                    Ok(#self_ident #self_name #rust_args #question)});

            },
            (_, false) => quote! {
                fields.add_field_method_get(#name, |_lua, s| {
                    #(#app_data)*

                    Ok(#self_ident #self_name #rust_args #question)});
            },
        };

        t.to_tokens(&mut fields_impl);
    }

    for c in consts {
        let name = c.ident;
        quote! {
            fields.add_field_function_get(stringify!(#name)     , |_lua, _| Ok(Self::#name));
        }
        .to_tokens(&mut fields_impl);
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
