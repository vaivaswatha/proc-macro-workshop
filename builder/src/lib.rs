use core::panic;

use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{parse_macro_input, parse_quote, Data, DeriveInput, Field, Type};

#[proc_macro_derive(Builder)]
pub fn derive(input: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let builder_ident = format_ident!("{}{}", input.ident, "Builder");

    let mut field_idents = vec![];
    let mut field_tys = vec![];
    if let Data::Struct(ref mut data) = input.data {
        let syn::Fields::Named(ref mut fields) = data.fields else {
            unimplemented!("Unnamed fields")
        };
        for field in fields.named.iter_mut() {
            let ty = &field.ty;
            field_tys.push(ty.clone());
            field_idents.push(field.ident.clone().unwrap());
            let optioned_ty: Type = parse_quote! { Option<#ty> };
            *field = Field {
                ty: optioned_ty,
                ..field.clone()
            };
        }
    } else {
        panic!("#[derive(Builder)] only works on structs")
    }

    let mut builder_fn = quote! {
        impl #name {
            fn builder() -> #builder_ident {
                #builder_ident {
                    #( #field_idents : None ), *
                }
            }
        }

        impl #builder_ident {
             #( pub fn #field_idents (&mut self, #field_idents : #field_tys) -> &mut Self { 
                self.#field_idents = Some(#field_idents);
                self
            }) *
        }
    };

    input.ident = builder_ident;
    let builder_struct_def = input.to_token_stream();

    builder_fn.extend(builder_struct_def);
    builder_fn.into()
}
