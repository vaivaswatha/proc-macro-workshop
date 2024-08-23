use core::panic;

use proc_macro::{Span, TokenStream};
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse_macro_input, parse_quote, spanned::Spanned, AngleBracketedGenericArguments, Attribute, Data, DataStruct, DeriveInput, Expr, ExprLit, Field, GenericArgument, Lit, Meta, Path, PathArguments, PathSegment, Type, TypePath
};

// If there is an "#[builder(each = "...")] specified, return the name.
fn match_vec_each(attrs: &Vec<Attribute>) -> Result<Option<String>, syn::Error> {
    if attrs.is_empty() {
        return Ok(None);
    }
    let attr = &attrs[0];

    let err = |span| Err(syn::Error::new(
        span,
        "expected `builder(each = \"...\")`",
    ));

    if attrs.len() != 1 {
        return err(attr.span());
    }

    if !attr.path().is_ident("builder") {
        return err(attr.path().span());
    }
    let Expr::Assign(assign) = attr.parse_args()? else {
        return err(attr.span());
    };

    let Expr::Path(lhs_path) = &*assign.left else {
        return err(assign.span());
    };
    if !lhs_path.path.is_ident("each") {
        return err(lhs_path.span());
    }
    let Expr::Lit(ExprLit {
        lit: Lit::Str(str), ..
    }) = &*assign.right
    else {
        return err(assign.span());
    };
    Ok(Some(str.value()))
}

#[proc_macro_derive(Builder, attributes(builder))]
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();

    let builder_ident = format_ident!("{}{}", input.ident, "Builder");

    let mut builder_struct = input;
    builder_struct.ident = builder_ident.clone();

    #[derive(Clone)]
    enum SpecialFieldTypes {
        Option,
        Vec,
        Unknown,
    }

    // Right, builder_struct is the same as our input struct.
    // Modify it to add `Option<>` around each field.
    let mut field_idents = vec![];
    let mut field_tys = vec![];
    let mut vec_each = vec![];
    if let Data::Struct(DataStruct {
        fields: syn::Fields::Named(ref mut fields),
        ..
    }) = builder_struct.data
    {
        for field in fields.named.iter_mut() {
            let ty = &field.ty;
            // Check if this field is already an `Option`.
            // Just following the tree in 06-optional-field.rs.
            let mut is_option = false;
            if let Type::Path(TypePath {
                qself: None,
                path: Path { segments, .. },
            }) = ty
            {
                if let Some(PathSegment {
                    ident,
                    arguments:
                        PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }),
                }) = segments.first()
                {
                    if let Some(GenericArgument::Type(inner_ty)) = args.first() {
                        if ident == "Option" {
                            is_option = true;
                            field_tys.push((inner_ty.clone(), SpecialFieldTypes::Option));
                        }
                    }
                }
            }
            let mut is_vec = false;
            if let Type::Path(TypePath {
                qself: None,
                path: Path { segments, .. },
            }) = ty
            {
                if let Some(PathSegment {
                    ident,
                    arguments:
                        PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }),
                }) = segments.first()
                {
                    if let Some(GenericArgument::Type(inner_ty)) = args.first() {
                        if ident == "Vec" {
                            is_vec = true;
                            field_tys.push((inner_ty.clone(), SpecialFieldTypes::Vec));
                        }
                    }
                }
            }
            if is_vec {
                let parsed_attr_opt = match match_vec_each(&field.attrs) {
                    Ok(attr_opt) => attr_opt,
                    Err(e) => return e.to_compile_error().into(),
                };
                if let Some(each_name) = parsed_attr_opt {
                    vec_each.push(Some(each_name));
                } else {
                    vec_each.push(None);
                }
            } else {
                vec_each.push(None);
            }

            // We don't want attributes on struct Builder
            field.attrs.clear();

            if !is_option {
                if !is_vec {
                    field_tys.push((ty.clone(), SpecialFieldTypes::Unknown));
                }
                // This is not an Option, so add Option wrapper.
                let optioned_ty: Type = parse_quote! { std::option::Option<#ty> };
                *field = Field {
                    ty: optioned_ty,
                    ..field.clone()
                };
            }
            field_idents.push(field.ident.clone().unwrap());
        }
    } else {
        panic!("#[derive(Builder)] only works on named structs")
    }

    let mut output = quote! {};

    let builder_fn = quote! {
        impl #name {
            fn builder() -> #builder_ident {
                #builder_ident {
                    #( #field_idents : std::option::Option::None ), *
                }
            }
        }
    };

    let mut builder_methods = quote! {};
    for field_idx in 0..field_idents.len() {
        let field_ident = field_idents[field_idx].clone();
        let (field_ty, field_speciality) = field_tys[field_idx].clone();
        let mut generate_all_at_once = true;
        if let Some(each_name) = vec_each[field_idx].clone() {
            let fn_name = format_ident!("{}", each_name);
            if fn_name == field_ident {
                generate_all_at_once = false;
            }
            let each_method = quote! {
                pub fn #fn_name (&mut self, #fn_name : #field_ty) -> &mut Self {
                        match self.#field_ident {
                            Some(ref mut v) => {
                                v.push(#fn_name);
                            }
                            None => {
                                let mut x = Vec::new();
                                x.push(#fn_name);
                                self.#field_ident = Some(x);
                            }
                        }
                        self
                }
            };
            builder_methods.extend(each_method);
        }
        let method = if generate_all_at_once {
            let arg_ty = if matches!(field_speciality, SpecialFieldTypes::Vec) {
                parse_quote! { std::vec::Vec<#field_ty> }
            } else {
                field_ty
            };
            quote! {
                pub fn #field_ident (&mut self, #field_ident : #arg_ty) -> &mut Self {
                        self.#field_ident = std::option::Option::Some(#field_ident);
                        self
                }
            }
        } else {
            quote! {}
        };
        builder_methods.extend(method);
    }

    let mut uninit_checks = quote! {};
    let mut field_assigns = quote! {};
    for field_idx in 0..field_idents.len() {
        let field_ident = field_idents[field_idx].clone();
        let (_field_ty, field_specialty) = field_tys[field_idx].clone();
        let (check, assign) = match field_specialty {
            SpecialFieldTypes::Option => (
                quote! {},
                quote! {
                    #field_ident: std::mem::replace(&mut self.#field_ident, std::option::Option::None),
                },
            ),
            SpecialFieldTypes::Vec => (
                quote! {},
                quote! {
                    #field_ident: std::mem::replace(&mut self.#field_ident, std::option::Option::None).unwrap_or(Vec::new()),
                },
            ),
            SpecialFieldTypes::Unknown => (
                quote! {
                    if self.#field_ident.is_none() {
                        return Err(format!("Field {} not initialized", stringify!(#field_ident)).into());
                    }
                },
                quote! {
                    #field_ident: std::mem::replace(&mut self.#field_ident, std::option::Option::None).unwrap(),
                },
            ),
        };
        uninit_checks.extend(check);
        field_assigns.extend(assign);
    }
    let build_method = quote! {
        pub fn build(&mut self) -> std::result::Result<#name, std::boxed::Box<dyn std::error::Error>> {
            #uninit_checks
            Ok(#name {
                #field_assigns
            })
        }
    };
    let builder_methods = quote! {
        impl #builder_ident {
            #builder_methods
            #build_method
        }
    };

    output.extend(builder_struct.to_token_stream());
    output.extend(builder_fn.into_token_stream());
    output.extend(builder_methods.to_token_stream());
    output.into()
}
