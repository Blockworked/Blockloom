//! `#[derive(Schema)]`: describes a serde type as a
//! `blockloom_plugin_api::schema::FieldType`, the same model plugin schemas
//! use, so built-in components and settings land in one registry.
//!
//! Field names and enum variants follow the type's serde attributes
//! (`rename`, `rename_all`, `skip`, `flatten`, `tag`). A field whose type has
//! no `Schema` impl is described as `Opaque` rather than refused, so the
//! derive can go on a type before everything under it has one.
//!
//! Field hints go in `#[schema(...)]`: `label = ".."`, `unit = ".."`,
//! `group = ".."`, `min = n`, `max = n`, `step = n`, `slider`, `multiline`,
//! `advanced`, `color`, `text`, `asset = "image"`, `actor`, `opaque`, `skip`,
//! and `show_if = "field"`, `show_if = "field=value"` or
//! `hide_if = "field=value"`. A doc comment becomes the description. A
//! `String` named `color` or `*_color` is a color unless marked `text`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, Fields, Lit, LitStr, Meta, Type, parse_macro_input,
};

#[proc_macro_derive(Schema, attributes(schema))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

const API: &str = "::blockloom_plugin_api::schema";

fn api() -> Tokens {
    API.parse().unwrap()
}

/// What `#[serde(...)]` says on a container, field or variant.
#[derive(Default)]
struct Serde {
    rename: Option<String>,
    rename_all: Option<String>,
    tag: Option<String>,
    content: bool,
    untagged: bool,
    skip: bool,
    flatten: bool,
    transparent: bool,
    /// `with`, `serialize_with`, `from`, `into`: the shape isn't the type's.
    custom: bool,
    /// A field's `default`: `Some(None)` bare, `Some(Some(path))` named.
    default: Option<Option<String>>,
}

fn serde_of(attrs: &[Attribute]) -> syn::Result<Serde> {
    let mut out = Serde::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            let name = meta
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            match name.as_str() {
                "rename" => {
                    // `rename = ".."` or `rename(serialize = "..", ...)`.
                    if meta.input.peek(syn::Token![=]) {
                        out.rename = Some(meta.value()?.parse::<LitStr>()?.value());
                    } else {
                        meta.parse_nested_meta(|inner| {
                            let v = inner.value()?.parse::<LitStr>()?.value();
                            if inner.path.is_ident("serialize") {
                                out.rename = Some(v);
                            }
                            Ok(())
                        })?;
                    }
                }
                "rename_all" => {
                    if meta.input.peek(syn::Token![=]) {
                        out.rename_all = Some(meta.value()?.parse::<LitStr>()?.value());
                    } else {
                        meta.parse_nested_meta(|inner| {
                            let v = inner.value()?.parse::<LitStr>()?.value();
                            if inner.path.is_ident("serialize") {
                                out.rename_all = Some(v);
                            }
                            Ok(())
                        })?;
                    }
                }
                "tag" => out.tag = Some(meta.value()?.parse::<LitStr>()?.value()),
                "content" => {
                    meta.value()?.parse::<LitStr>()?;
                    out.content = true;
                }
                "untagged" => out.untagged = true,
                "default" => {
                    out.default = Some(if meta.input.peek(syn::Token![=]) {
                        Some(meta.value()?.parse::<LitStr>()?.value())
                    } else {
                        None
                    });
                }
                "skip" | "skip_serializing" | "skip_deserializing" => out.skip = true,
                "flatten" => out.flatten = true,
                "transparent" => out.transparent = true,
                "with" | "serialize_with" | "from" | "into" | "try_from" | "remote" => {
                    let _: Expr = meta.value()?.parse()?;
                    out.custom = true;
                }
                _ => {
                    // Everything else (default, with, alias, ...) has no say
                    // in the shape; swallow its value if it has one.
                    if meta.input.peek(syn::Token![=]) {
                        let _: Expr = meta.value()?.parse()?;
                    } else if meta.input.peek(syn::token::Paren) {
                        let _ = meta.parse_nested_meta(|inner| {
                            if inner.input.peek(syn::Token![=]) {
                                let _: Expr = inner.value()?.parse()?;
                            }
                            Ok(())
                        });
                    }
                }
            }
            Ok(())
        })?;
    }
    Ok(out)
}

/// What `#[schema(...)]` says on a field.
#[derive(Default)]
struct Hints {
    label: String,
    unit: String,
    group: String,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    slider: bool,
    multiline: bool,
    advanced: bool,
    color: bool,
    text: bool,
    asset: Option<String>,
    actor: bool,
    opaque: bool,
    skip: bool,
    show_if: Option<String>,
    hide_if: Option<String>,
}

fn number(expr: &Expr) -> syn::Result<f64> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(i), ..
        }) => i.base10_parse::<f64>(),
        Expr::Lit(ExprLit {
            lit: Lit::Float(f), ..
        }) => f.base10_parse::<f64>(),
        Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => Ok(-number(&u.expr)?),
        _ => Err(syn::Error::new_spanned(expr, "expected a number")),
    }
}

fn hints_of(attrs: &[Attribute]) -> syn::Result<Hints> {
    let mut out = Hints::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("schema")) {
        attr.parse_nested_meta(|meta| {
            let name = meta
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            let text = |meta: &syn::meta::ParseNestedMeta| -> syn::Result<String> {
                Ok(meta.value()?.parse::<LitStr>()?.value())
            };
            match name.as_str() {
                "label" => out.label = text(&meta)?,
                "unit" => out.unit = text(&meta)?,
                "group" => out.group = text(&meta)?,
                "asset" => {
                    out.asset = Some(if meta.input.peek(syn::Token![=]) {
                        text(&meta)?
                    } else {
                        "any".to_string()
                    })
                }
                "show_if" => out.show_if = Some(text(&meta)?),
                "hide_if" => out.hide_if = Some(text(&meta)?),
                "min" => out.min = Some(number(&meta.value()?.parse()?)?),
                "max" => out.max = Some(number(&meta.value()?.parse()?)?),
                "step" => out.step = Some(number(&meta.value()?.parse()?)?),
                "slider" => out.slider = true,
                "multiline" => out.multiline = true,
                "advanced" => out.advanced = true,
                "color" => out.color = true,
                "text" => out.text = true,
                "actor" => out.actor = true,
                "opaque" => out.opaque = true,
                "skip" => out.skip = true,
                other => return Err(meta.error(format!("unknown schema hint `{other}`"))),
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn doc_of(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("doc")) {
        if let Meta::NameValue(nv) = &attr.meta
            && let Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) = &nv.value
        {
            lines.push(s.value().trim().to_string());
        }
    }
    lines.join(" ").trim().to_string()
}

/// serde's `rename_all` rules, for the ones Blockloom's types use.
fn rename(name: &str, rule: Option<&str>) -> String {
    let words = split_words(name);
    match rule {
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        Some("snake_case") => words.join("_").to_lowercase(),
        Some("SCREAMING_SNAKE_CASE") => words.join("_").to_uppercase(),
        Some("kebab-case") => words.join("-").to_lowercase(),
        Some("SCREAMING-KEBAB-CASE") => words.join("-").to_uppercase(),
        Some("camelCase") => {
            let mut out = String::new();
            for (i, w) in words.iter().enumerate() {
                if i == 0 {
                    out.push_str(&w.to_lowercase());
                } else {
                    out.push_str(&capitalize(w));
                }
            }
            out
        }
        Some("PascalCase") => words.iter().map(|w| capitalize(w)).collect(),
        _ => name.to_string(),
    }
}

fn capitalize(word: &str) -> String {
    let lower = word.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn split_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for c in name.chars() {
        if c == '_' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else if c.is_uppercase() && !current.is_empty() {
            words.push(std::mem::take(&mut current));
            current.push(c);
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// The field type of `ty`: its `Schema` impl when it has one, else `Opaque`.
fn type_of(ty: &Type) -> Tokens {
    let api = api();
    quote! {
        {
            #[allow(unused_imports)]
            use #api::__private::{ViaOpaque as _, ViaSchema as _};
            (&#api::__private::Probe::<#ty>::new()).__field_type()
        }
    }
}

fn condition(spec: &str, negate: bool) -> Tokens {
    let api = api();
    let clauses: Vec<&str> = spec.split("&&").map(str::trim).collect();
    if clauses.len() > 1 {
        let parts: Vec<Tokens> = clauses.iter().map(|c| clause(c, negate)).collect();
        return quote! {
            ::core::option::Option::Some(#api::Condition {
                field: ::std::string::String::new(),
                equals: ::core::option::Option::None,
                not_equals: ::core::option::Option::None,
                one_of: ::std::vec::Vec::new(),
                not_one_of: ::std::vec::Vec::new(),
                all: ::std::vec![#(#parts),*],
            })
        };
    }
    let one = clause(spec, negate);
    quote!(::core::option::Option::Some(#one))
}

/// One `field`, `field = v`, `field != v`, `field in [a, b]` or
/// `field not in [a, b]` test; `negate` flips it (for `hide_if`).
fn clause(spec: &str, negate: bool) -> Tokens {
    let api = api();
    let none = quote!(::core::option::Option::None);
    let list = |items: &str| -> Tokens {
        let values = items
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| quote!(#api::__private::parse_value(#v)));
        quote!(::std::vec![#(#values),*])
    };
    let empty = quote!(::std::vec::Vec::new());
    let (field, equals, not_equals, one_of, not_one_of) =
        if let Some((f, items)) = spec.split_once(" not in ") {
            (f, none.clone(), none.clone(), empty.clone(), list(items))
        } else if let Some((f, items)) = spec.split_once(" in ") {
            (f, none.clone(), none.clone(), list(items), empty.clone())
        } else if let Some((f, v)) = spec.split_once("!=") {
            let v = v.trim();
            let v = quote!(::core::option::Option::Some(#api::__private::parse_value(#v)));
            (f, none.clone(), v, empty.clone(), empty.clone())
        } else if let Some((f, v)) = spec.split_once('=') {
            let v = v.trim();
            let v = quote!(::core::option::Option::Some(#api::__private::parse_value(#v)));
            (f, v, none.clone(), empty.clone(), empty.clone())
        } else if negate {
            // Hidden while truthy: shown while it is one of the falsy values.
            let falsy = list("false, 0, \"\", null");
            (spec, none.clone(), none.clone(), empty.clone(), falsy)
        } else {
            (
                spec,
                none.clone(),
                none.clone(),
                empty.clone(),
                empty.clone(),
            )
        };
    let (equals, not_equals, one_of, not_one_of) = if negate {
        (not_equals, equals, not_one_of, one_of)
    } else {
        (equals, not_equals, one_of, not_one_of)
    };
    let field = field.trim();
    quote! {
        #api::Condition {
            field: #field.to_string(),
            equals: #equals,
            not_equals: #not_equals,
            one_of: #one_of,
            not_one_of: #not_one_of,
            all: ::std::vec::Vec::new(),
        }
    }
}

/// Code pushing one field's schema onto `fields` (or, flattened, its
/// fields).
fn field_code(
    name: &str,
    ty: &Type,
    attrs: &[Attribute],
    rule: Option<&str>,
) -> syn::Result<Tokens> {
    let api = api();
    let serde = serde_of(attrs)?;
    let hints = hints_of(attrs)?;
    if serde.skip || hints.skip {
        return Ok(quote!());
    }
    let base = if hints.opaque || serde.custom {
        quote!(#api::FieldType::Opaque)
    } else {
        type_of(ty)
    };
    if serde.flatten {
        return Ok(quote! {
            match #base {
                #api::FieldType::Struct { fields: inner } => fields.extend(inner),
                _ => {}
            }
        });
    }
    let key = serde.rename.clone().unwrap_or_else(|| rename(name, rule));
    let description = doc_of(attrs);
    let colorish = !hints.text && (name == "color" || name.ends_with("_color"));
    let force_color = hints.color;
    let asset = match &hints.asset {
        Some(kind) => quote!(::core::option::Option::Some(#kind)),
        None => quote!(::core::option::Option::None),
    };
    let actor = hints.actor;
    let opt = |v: Option<f64>| match v {
        Some(n) => quote!(::core::option::Option::Some(#n)),
        None => quote!(::core::option::Option::None),
    };
    let (min, max, step) = (opt(hints.min), opt(hints.max), opt(hints.step));
    let widget = if hints.slider {
        quote!(::core::option::Option::Some(#api::Widget::Slider))
    } else if hints.multiline {
        quote!(::core::option::Option::Some(#api::Widget::Multiline))
    } else {
        quote!(::core::option::Option::None)
    };
    let visible_when = match (&hints.show_if, &hints.hide_if) {
        (Some(s), _) => condition(s, false),
        (None, Some(s)) => condition(s, true),
        (None, None) => quote!(::core::option::Option::None),
    };
    let label = hints.label;
    let unit = hints.unit;
    let group = hints.group;
    let advanced = hints.advanced;
    // What serde fills in for a missing field. The container's own default,
    // when it has one, replaces this later (`fill_field_defaults`).
    let default = match (&serde.default, serde.custom || hints.opaque) {
        (Some(None), false) => {
            quote!(#api::__private::default_of(<#ty as ::core::default::Default>::default()))
        }
        (Some(Some(path)), false) => {
            let path: syn::ExprPath = syn::parse_str(path)?;
            quote!(#api::__private::default_of(#path()))
        }
        _ => quote!(::core::option::Option::None),
    };
    Ok(quote! {
        {
            let ty = #api::__private::refine(
                #base,
                #api::__private::Refine {
                    color: #force_color || (#colorish),
                    asset: #asset,
                    actor: #actor,
                    min: #min,
                    max: #max,
                },
            );
            let ui = #api::FieldUi {
                label: #label.to_string(),
                widget: #widget,
                unit: #unit.to_string(),
                step: #step,
                visible_when: #visible_when,
                group: #group.to_string(),
                advanced: #advanced,
            };
            fields.push(#api::FieldSchema {
                name: #key.to_string(),
                ty,
                default: #default,
                description: #description.to_string(),
                ui: if ui == #api::FieldUi::default() {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(ui)
                },
            });
        }
    })
}

fn named_fields(fields: &syn::FieldsNamed, rule: Option<&str>) -> syn::Result<Tokens> {
    let mut out = Tokens::new();
    for field in &fields.named {
        let name = field.ident.as_ref().unwrap().to_string();
        let name = name.strip_prefix("r#").unwrap_or(&name).to_string();
        out.extend(field_code(&name, &field.ty, &field.attrs, rule)?);
    }
    Ok(out)
}

fn expand(input: &DeriveInput) -> syn::Result<Tokens> {
    let api = api();
    let ident = &input.ident;
    let serde = serde_of(&input.attrs)?;
    let rule = serde.rename_all.as_deref();
    // A type parameter is described by its own impl, so `Override<f32>` has
    // a schema exactly when `f32` does.
    let mut generics = input.generics.clone();
    let params: Vec<syn::Ident> = generics.type_params().map(|p| p.ident.clone()).collect();
    {
        let clause = generics.make_where_clause();
        for p in params {
            clause.predicates.push(syn::parse_quote!(#p: #api::Schema));
        }
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let body = match &input.data {
        _ if serde.custom => quote!(#api::FieldType::Opaque),
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) if !serde.transparent => {
                let pushes = named_fields(named, rule)?;
                quote! {
                    let mut fields: ::std::vec::Vec<#api::FieldSchema> = ::std::vec::Vec::new();
                    #pushes
                    #api::FieldType::Struct { fields }
                }
            }
            Fields::Named(named) => {
                let only = named.named.first().unwrap();
                type_of(&only.ty)
            }
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                type_of(&unnamed.unnamed[0].ty)
            }
            _ => quote!(#api::FieldType::Opaque),
        },
        Data::Enum(data) => {
            if serde.untagged || serde.content {
                quote!(#api::FieldType::Opaque)
            } else if data
                .variants
                .iter()
                .all(|v| matches!(v.fields, Fields::Unit))
            {
                let mut names = Vec::new();
                for v in &data.variants {
                    let vs = serde_of(&v.attrs)?;
                    if vs.skip {
                        continue;
                    }
                    names.push(
                        vs.rename
                            .unwrap_or_else(|| rename(&v.ident.to_string(), rule)),
                    );
                }
                quote! {
                    #api::FieldType::Choice {
                        options: ::std::vec![#(#names.to_string()),*],
                        source: ::std::option::Option::None,
                    }
                }
            } else {
                let tag = serde.tag.clone().unwrap_or_default();
                let mut variants = Tokens::new();
                for v in &data.variants {
                    let vs = serde_of(&v.attrs)?;
                    if vs.skip {
                        continue;
                    }
                    let name = vs
                        .rename
                        .clone()
                        .unwrap_or_else(|| rename(&v.ident.to_string(), rule));
                    let description = doc_of(&v.attrs);
                    let vrule = vs.rename_all.as_deref();
                    let (fields, inner) = match &v.fields {
                        Fields::Unit => (quote!(), quote!(::core::option::Option::None)),
                        Fields::Named(named) => (
                            named_fields(named, vrule)?,
                            quote!(::core::option::Option::None),
                        ),
                        Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                            let t = type_of(&unnamed.unnamed[0].ty);
                            if tag.is_empty() {
                                (
                                    quote!(),
                                    quote!(::core::option::Option::Some(::std::boxed::Box::new(#t))),
                                )
                            } else {
                                // Internally tagged newtype: its fields sit
                                // beside the tag.
                                (
                                    quote! {
                                        if let #api::FieldType::Struct { fields: inner } = #t {
                                            fields.extend(inner);
                                        }
                                    },
                                    quote!(::core::option::Option::None),
                                )
                            }
                        }
                        Fields::Unnamed(_) => (
                            quote!(),
                            quote!(::core::option::Option::Some(::std::boxed::Box::new(
                                #api::FieldType::Opaque
                            ))),
                        ),
                    };
                    variants.extend(quote! {
                        {
                            let mut fields: ::std::vec::Vec<#api::FieldSchema> = ::std::vec::Vec::new();
                            #fields
                            variants.push(#api::VariantSchema {
                                name: #name.to_string(),
                                fields,
                                inner: #inner,
                                description: #description.to_string(),
                            });
                        }
                    });
                }
                quote! {
                    let mut variants: ::std::vec::Vec<#api::VariantSchema> = ::std::vec::Vec::new();
                    #variants
                    #api::FieldType::Enum { tag: #tag.to_string(), variants }
                }
            }
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                ident,
                "Schema can't describe a union",
            ));
        }
    };
    let type_name = ident.to_string();
    Ok(quote! {
        impl #impl_generics #api::Schema for #ident #ty_generics #where_clause {
            fn field_type() -> #api::FieldType {
                #body
            }
            fn type_name() -> &'static str {
                #type_name
            }
        }
    })
}
