//! Items: `import`, `struct`, `enum`, `fn`.
//!
//! # One item is one keyword
//!
//! [`Parser::parse_item`] dispatches on the first token and every arm starts by
//! consuming it, so `parse_program` can rely on two things: an item either
//! consumes at least one token or reports a failure, and a failure always leaves
//! the cursor somewhere recovery can move forward from. Neither is obvious
//! enough to leave to the reader of the loop, which is why
//! [`Parser::parse_program`] asserts the first of them anyway.
//!
//! # Spans
//!
//! Every node built here carries the span of the text it was parsed from, and
//! an item's span runs from its keyword to its last token -- the `;` of an
//! import, the `}` of a struct, enum or function. A field's span is the field,
//! not the struct it is in, so a diagnostic about one field does not underline
//! the whole declaration. That is the whole reason the AST carries spans at all
//! (see [`crate::ast`]), and it is why no span in this file is
//! [`Span::SYNTHETIC`] except where there is genuinely no source.
//!
//! # A positional payload is not a field with no name
//!
//! `Circle(f64)` and `Rect { w: f64 }` are written differently, mean different
//! things, and are matched differently -- [`Pattern::TupleVariant`] against
//! [`Pattern::StructVariant`]. They are therefore two variants of
//! [`VariantField`], not one shape with an optional name: `Positional` and
//! `Named`. Nothing in the parser can build a named field without a name, and
//! nothing can turn something the user wrote a name for into a positional
//! field, because the two are different types rather than the same type with a
//! sentinel inside it.
//!
//! The alternative was a `FieldDef` whose `name` is the empty string for a
//! positional field. That encodes "this has no name" as a *value*, which makes
//! it a convention every later stage has to remember: one forgotten
//! `name.is_empty()` in the type checker is a program that type checks against a
//! field that does not exist. A variant is checked by the compiler.
//!
//! [`FieldDef`] is still the field of a **struct**, where a name is always
//! written and always present.
//!
//! # `extern` is rejected, not ignored
//!
//! [`Item::Extern`] exists so that M5 adds a case rather than changing a
//! contract four stages code against. Until then an `extern` block is a
//! diagnostic naming the milestone it arrives in -- see
//! [`error::extern_arrives_in_m5`]. Silently skipping it would be worse: a
//! program whose FFI declarations vanished would compile, link against nothing,
//! and fail at run time instead of at the `extern` keyword.

use otter_common::Span;
use otter_lexer::TokenKind;

use crate::ast::{FieldDef, Item, Param, VariantDef, VariantField};
use crate::error::{self, ParseError};
use crate::parser::{cover, Parser};

impl<'src> Parser<'src> {
    /// Parses one item, dispatching on its first token.
    ///
    /// # Errors
    ///
    /// A [`ParseError`] for a token that cannot begin an item -- and that
    /// includes `extern`, which is a keyword the language has and this milestone
    /// does not read yet.
    pub fn parse_item(&mut self) -> Result<Item, ParseError> {
        let kind = self.peek();
        match kind {
            TokenKind::Import => self.parse_import(),
            TokenKind::Struct => self.parse_struct(),
            TokenKind::Enum => self.parse_enum(),
            TokenKind::Fn => self.parse_fn(),
            TokenKind::Extern => Err(error::extern_arrives_in_m5(self.peek_span())),
            _ => Err(error::item_expected(kind, self.peek_span())),
        }
    }

    /// `import std.io;` -- a dotted path, one [`String`] per segment.
    ///
    /// The path is read from the tokens rather than from the source text between
    /// them, so a path segment is exactly an identifier: `import 1;` and
    /// `import std..io;` are diagnostics, not paths with holes in them.
    fn parse_import(&mut self) -> Result<Item, ParseError> {
        let keyword = self.expect(TokenKind::Import)?;
        let mut path = vec![self.import_segment()?];
        while self.eat(TokenKind::Dot) {
            path.push(self.import_segment()?);
        }
        let semicolon = self.expect(TokenKind::Semi)?;
        Ok(Item::Import {
            path,
            span: cover(keyword.span, semicolon.span),
        })
    }

    /// One segment of an import path.
    ///
    /// The error here is about the *path*, not about the token: "expected an
    /// identifier, found `;`" is true of `import ;` and tells the reader nothing
    /// about what they meant to write.
    fn import_segment(&mut self) -> Result<String, ParseError> {
        if !self.check(TokenKind::Ident) {
            return Err(error::import_needs_a_path(self.peek_span()));
        }
        let (name, _span) = self.identifier()?;
        Ok(name)
    }

    /// `struct P { x: f64, y: f64 }`
    ///
    /// At least one field is required. An empty struct is legal in plenty of
    /// languages and useless in every program that has one; since this parser is
    /// the stage that reads the braces, it is also the only stage that can say so
    /// while the name of the struct is in hand.
    fn parse_struct(&mut self) -> Result<Item, ParseError> {
        let keyword = self.expect(TokenKind::Struct)?;
        let (name, name_span) = self.identifier()?;
        let open = self.expect(TokenKind::LBrace)?;
        let _ = self.push_open(TokenKind::LBrace, open.span)?;

        let mut fields = Vec::new();
        while !self.check(TokenKind::RBrace) {
            fields.push(self.parse_field_def()?);
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        self.close_open(TokenKind::RBrace);

        if fields.is_empty() {
            return Err(error::struct_needs_a_field(
                cover(keyword.span, name_span),
                &name,
            ));
        }
        Ok(Item::Struct {
            name,
            fields,
            span: cover(keyword.span, close.span),
        })
    }

    /// `x: f64` -- one named field of a struct, or of a struct-shaped enum
    /// variant.
    fn parse_field_def(&mut self) -> Result<FieldDef, ParseError> {
        let (name, name_span) = self.identifier()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type_expr()?;
        Ok(FieldDef {
            name,
            span: cover(name_span, ty_span(&ty)),
            ty,
        })
    }

    /// `enum C { Red, Green }`
    fn parse_enum(&mut self) -> Result<Item, ParseError> {
        let keyword = self.expect(TokenKind::Enum)?;
        let (name, name_span) = self.identifier()?;
        let open = self.expect(TokenKind::LBrace)?;
        let _ = self.push_open(TokenKind::LBrace, open.span)?;

        let mut variants = Vec::new();
        while !self.check(TokenKind::RBrace) {
            variants.push(self.parse_variant_def(&name)?);
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        self.close_open(TokenKind::RBrace);

        if variants.is_empty() {
            return Err(error::enum_needs_a_variant(
                cover(keyword.span, name_span),
                &name,
            ));
        }
        Ok(Item::Enum {
            name,
            variants,
            span: cover(keyword.span, close.span),
        })
    }

    /// One enum variant: `Red`, `Circle(f64)`, or `Rect { w: f64, h: f64 }`.
    ///
    /// The three spellings map onto three shapes and stay that way: no payload
    /// is an empty `fields`, a parenthesised payload is a list of
    /// [`VariantField::Positional`], and a braced payload is a list of
    /// [`VariantField::Named`]. The parser never has to *decide* which kind of
    /// field it is holding, because the two are different types -- there is no
    /// path through this function that can build a named field with no name, or
    /// a positional field for something the user wrote a name for.
    fn parse_variant_def(&mut self, enum_name: &str) -> Result<VariantDef, ParseError> {
        let (name, name_span) = self.identifier()?;

        if self.check(TokenKind::LParen) {
            let (fields, close) = self.parse_positional_payload()?;
            if fields.is_empty() {
                return Err(error::variant_needs_a_field(
                    cover(name_span, close),
                    enum_name,
                    &name,
                ));
            }
            return Ok(VariantDef {
                name,
                fields,
                span: cover(name_span, close),
            });
        }
        if self.check(TokenKind::LBrace) {
            let (fields, close) = self.parse_named_payload()?;
            if fields.is_empty() {
                return Err(error::variant_needs_a_field(
                    cover(name_span, close),
                    enum_name,
                    &name,
                ));
            }
            return Ok(VariantDef {
                name,
                fields,
                span: cover(name_span, close),
            });
        }
        Ok(VariantDef {
            name,
            fields: Vec::new(),
            span: name_span,
        })
    }

    /// `(T, U)` -- the payload of a variant written with parentheses.
    ///
    /// Every field here is [`VariantField::Positional`], because the source
    /// gave no names. That is a property of the shape rather than a value
    /// inside it: there is no name to be empty, so nothing downstream has to
    /// check whether a name is missing.
    fn parse_positional_payload(&mut self) -> Result<(Vec<VariantField>, Span), ParseError> {
        let open = self.expect(TokenKind::LParen)?;
        let _ = self.push_open(TokenKind::LParen, open.span)?;

        let mut fields = Vec::new();
        while !self.check(TokenKind::RParen) {
            let ty = self.parse_type_expr()?;
            fields.push(VariantField::Positional {
                span: ty_span(&ty),
                ty,
            });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RParen)?;
        self.close_open(TokenKind::RParen);
        Ok((fields, close.span))
    }

    /// `{ w: f64, h: f64 }` -- the payload of a variant written with braces.
    ///
    /// Every field here is [`VariantField::Named`], and every name came from an
    /// identifier the source wrote, so a named field cannot have an empty name:
    /// each name comes from [`Parser::identifier`], which refuses
    /// a span that reads as no text at all.
    fn parse_named_payload(&mut self) -> Result<(Vec<VariantField>, Span), ParseError> {
        let open = self.expect(TokenKind::LBrace)?;
        let _ = self.push_open(TokenKind::LBrace, open.span)?;

        let mut fields = Vec::new();
        while !self.check(TokenKind::RBrace) {
            let (name, name_span) = self.identifier()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type_expr()?;
            fields.push(VariantField::Named {
                name,
                span: cover(name_span, ty_span(&ty)),
                ty,
            });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        self.close_open(TokenKind::RBrace);
        Ok((fields, close.span))
    }

    /// `fn d(a: P, b: P) -> f64 { ... }`
    ///
    /// The return type is `None` when the source wrote no `->`. That is a
    /// *missing* annotation, which is not the same fact as one that says
    /// `-> ()`: a function with no return type and one that returns unit behave
    /// the same, and a reader looking at the tree should be able to tell which
    /// one was written.
    fn parse_fn(&mut self) -> Result<Item, ParseError> {
        let keyword = self.expect(TokenKind::Fn)?;
        let (name, _) = self.identifier()?;

        let open = self.expect(TokenKind::LParen)?;
        let _ = self.push_open(TokenKind::LParen, open.span)?;
        let mut params = Vec::new();
        while !self.check(TokenKind::RParen) {
            params.push(self.parse_param()?);
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        // The `)` is consumed and the stack unwound, but the item's span runs to
        // the body's closing `}` rather than to here.
        self.expect(TokenKind::RParen)?;
        self.close_open(TokenKind::RParen);

        let ret = if self.eat(TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        // The body is a seam: `parse_block` is Task 4's half of it and Task 5
        // finishes it. It is what makes the delimiters inside a function body
        // part of this task's stack rather than a hole in it.
        let body = self.parse_block()?;
        let span = cover(keyword.span, body.span);

        Ok(Item::Fn {
            name,
            params,
            ret,
            body,
            span,
        })
    }

    /// `a: P` -- one parameter of a function.
    ///
    /// Always named and always annotated (spec §3.2): the annotation is required
    /// at the boundary, which is exactly where inference has nothing to infer
    /// from.
    fn parse_param(&mut self) -> Result<Param, ParseError> {
        let (name, name_span) = self.identifier()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type_expr()?;
        Ok(Param {
            name,
            span: cover(name_span, ty_span(&ty)),
            ty,
        })
    }
}

/// The span of a type, read back out of the type itself.
///
/// A `TypeExpr` carries its span in each variant rather than beside it, so the
/// span of a field is `cover(name, type)`. Reading it back here rather than
/// threading a second value through every type parser keeps the two from
/// disagreeing: there is one span, it is in the node, and this is where it is
/// picked up.
fn ty_span(ty: &crate::ast::TypeExpr) -> Span {
    match ty {
        crate::ast::TypeExpr::Name(_, span)
        | crate::ast::TypeExpr::Option(_, span)
        | crate::ast::TypeExpr::Ptr(_, span)
        | crate::ast::TypeExpr::Unit(span) => *span,
        crate::ast::TypeExpr::Result(_, _, span) => *span,
        crate::ast::TypeExpr::Tuple(_, span) => *span,
    }
}
