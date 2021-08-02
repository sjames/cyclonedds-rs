use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{Field, Ident, ItemStruct, parse_macro_input};

#[proc_macro_derive(Topic, attributes(topic_key))]
pub fn derive_topic(item: TokenStream) -> TokenStream {
    let topic_struct = parse_macro_input!(item as syn::ItemStruct);


    if struct_has_key(&topic_struct) {
        println!("Struct has KEY");    
    }

    let mut ts = build_key_holder_struct(&topic_struct);
    let ts2 = create_keyhash_functions(&topic_struct);

    ts.extend(ts2);
    

    println!("KEYHOLDER:{:?}",ts.clone().to_string());

    ts
}

///Create a key holder struct from the given struct. The key
///fields will be included in this structure. The structure
///will be empty if there are no key fields.
fn build_key_holder_struct(item : &syn::ItemStruct) -> TokenStream {
    let mut key_holder_struct = item;
    
    let mut holder_name = key_holder_struct.ident.to_string();
    let fields = &key_holder_struct.fields;
    holder_name.push_str("KeyHolder_");
    let holder_name = Ident::new(&holder_name, Span::call_site());
    //key_holder_struct.ident = Ident::new(&holder_name,Span::call_site());

    let mut field_idents = Vec::new();
    let mut field_types = Vec::new();
    let mut clone_or_into = Vec::new();
    let mut ref_or_value = Vec::new();

    for mut field in fields {
        if is_key(field) {
            field_idents.push(field.ident.as_ref().unwrap().clone());
            if is_primitive(field) {
                field_types.push(field.ty.clone());
                clone_or_into.push(quote!{clone()});
                ref_or_value.push(quote!{ });
            } else {
                // if the key is another structure (not a primitive),
                // there should be a key holder structure for it. 
                // Change the type
                if let syn::Type::Path(mut type_path) = field.ty.clone() {
                    let last_segment= type_path.path.segments.last_mut().unwrap();
                    let mut ident_string = last_segment.ident.to_string();
                    ident_string.push_str("KeyHolder_");
                    let new_ident = Ident::new(&ident_string,Span::call_site());
                    //replace the ident with the new name
                    last_segment.ident = new_ident;
                    field_types.push(syn::Type::Path(type_path));
                    clone_or_into.push(quote!{into()});
                    ref_or_value.push(quote!{ &});

                } else {
                    panic!("Keys need to be primitives or syn::Type::Path");
                }
            }
        }
    }
   
    let item_ident = &item.ident;
    //println!("Filtered fields:{:?}", &filtered_fields);

    let ts = quote! {
        #[derive(Default, Deserialize, Serialize, PartialEq, Clone, Debug)]
        struct #holder_name {
            #(#field_idents:#field_types,)*
        }

        impl From<& #item_ident> for #holder_name {
            fn from(source: & #item_ident) -> Self {
                Self {
                    #(#field_idents : (#ref_or_value source.#field_idents). #clone_or_into ,)*
                }
            }
        }
    
    };

    ts.into() 
}

// create the keyhash methods for this type
fn create_keyhash_functions(item : &syn::ItemStruct) -> TokenStream {
    let topic_key_ident = &item.ident;
    let topic_key_holder_ident =  quote::format_ident!("{}KeyHolder_",&item.ident);

    let ts = quote!{
        impl Topic for #topic_key_ident {
            /// return the cdr encoding for the key. The encoded string includes the four byte
            /// encapsulation string.
            fn key_cdr(&self) -> Vec<u8> {
                let holder_struct : #topic_key_holder_ident = self.into();
                
                println!("TopicKeyHolder:{:?}  size:{}", &holder_struct,std::mem::size_of::<#topic_key_holder_ident>());
                
                let encoded = cdr::serialize::<_, _, CdrBe>(&holder_struct, cdr::Infinite).expect("Unable to serialize key");
               encoded
            }
            
            fn has_key() -> bool {
                if std::mem::size_of::<#topic_key_holder_ident>() > 0 {
                    true
                } else {
                    false
                } 
            }

            fn use_md5_keyhash() -> bool {
                true
            }
        }
    };

    ts.into()
    
}

fn struct_has_key(it: &ItemStruct) -> bool {
    for field in &it.fields {
        if is_key(field) {
            return true
        }
    }
    false
}

fn is_key(field : &Field) -> bool {
    for attr in &field.attrs {
        if let Some(ident) = attr.path.get_ident() {
            if ident == "topic_key" {
                return true
            }
        } 
    }
    false
}



// check if a field is of a primitive type. We assume anything not primitive
// is a struct
fn is_primitive(field:&Field) -> bool {
    if let syn::Type::Path(type_path) = &field.ty {
        if  type_path.path.is_ident("bool") ||
            type_path.path.is_ident("i8") ||
            type_path.path.is_ident("i16") ||
            type_path.path.is_ident("i32") ||
            type_path.path.is_ident("i64") ||
            type_path.path.is_ident("i128") ||
            type_path.path.is_ident("isize") ||
            type_path.path.is_ident("u8") ||
            type_path.path.is_ident("u16") ||
            type_path.path.is_ident("u32") ||
            type_path.path.is_ident("u64") ||
            type_path.path.is_ident("u128") ||
            type_path.path.is_ident("usize") ||
            type_path.path.is_ident("f32") ||
            type_path.path.is_ident("f64") || 
            type_path.path.is_ident("String")
        {
            true
        } else {
            false
        }
    } else {
        false
    }
}
