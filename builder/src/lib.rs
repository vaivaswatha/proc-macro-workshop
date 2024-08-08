use core::panic;

use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse_macro_input, parse_quote, AngleBracketedGenericArguments, Data, DataStruct, DeriveInput,
    Field, GenericArgument, Path, PathArguments, PathSegment, Type, TypePath,
};

#[proc_macro_derive(Builder)]
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();

    let builder_ident = format_ident!("{}{}", input.ident, "Builder");

    let mut builder_struct = input;
    builder_struct.ident = builder_ident.clone();

    // Right, builder_struct is the same as our input struct.
    // Modify it to add `Option<>` around each field.
    let mut field_idents = vec![];
    let mut field_tys = vec![];
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
                            field_tys.push((inner_ty.clone(), true));
                        }
                    }
                }
            }

            if !is_option {
                field_tys.push((ty.clone(), false));
                // This is not an Option, so add Option wrapper.
                let optioned_ty: Type = parse_quote! { Option<#ty> };
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
                    #( #field_idents : None ), *
                }
            }
        }
    };

    let mut builder_methods = quote! {};
    for field_idx in 0..field_idents.len() {
        let field_ident = field_idents[field_idx].clone();
        let (field_ty, _is_option) = field_tys[field_idx].clone();
        let method = quote! {
            pub fn #field_ident (&mut self, #field_ident : #field_ty) -> &mut Self {
                    self.#field_ident = Some(#field_ident);
                    self
            }
        };
        builder_methods.extend(method);
    }

    let mut uninit_checks = quote! {};
    let mut field_assigns = quote! {};
    for field_idx in 0..field_idents.len() {
        let field_ident = field_idents[field_idx].clone();
        let (_field_ty, is_option) = field_tys[field_idx].clone();
        let (check, assign) = if !is_option {
            (
                quote! {
                    if self.#field_ident.is_none() {
                        return Err(format!("Field {} not initialized", stringify!(#field_ident)).into());
                    }
                },
                quote! {
                    #field_ident: std::mem::replace(&mut self.#field_ident, None).unwrap(),
                },
            )
        } else {
            (
                quote! {},
                quote! {
                    #field_ident: std::mem::replace(&mut self.#field_ident, None),
                },
            )
        };
        uninit_checks.extend(check);
        field_assigns.extend(assign);
    }
    let build_method = quote! {
        pub fn build(&mut self) -> Result<#name, Box<dyn std::error::Error>> {
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
