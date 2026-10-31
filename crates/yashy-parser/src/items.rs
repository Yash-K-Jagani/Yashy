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
//! # `extern` blocks
//!
//! [`Item::Extern`] is read from M5 on: an ABI string, an optional library to
//! link, and one or more declarations. Every declaration must say `unsafe`, so
//! a forgotten marker is a parse diagnostic at the declaration rather than a
//! type-checking rule that has to remember which functions came from C.

use yashy_common::Span;
use yashy_lexer::TokenKind;

use crate::ast::{Contract, ExternDecl, FieldDef, Item, Param, TypeExpr, VariantDef, VariantField};
use crate::error::{self, ParseError};
use crate::parser::{cover, Parser};

impl<'src> Parser<'src> {
    /// Parses one item, dispatching on its first token.
    ///
    /// # Errors
    ///
    /// A [`ParseError`] for a token that cannot begin an item.
    pub fn parse_item(&mut self) -> Result<Item, ParseError> {
        // A stray `link_name` at item position is the clause without a
        // declaration to attach to. Reporting the rule is more useful than
        // "expected an item", and it is the same mistake as `link_name` on a
        // Yashy `fn`.
        if self.at_word("link_name") {
            let span = self.peek_span();
            return Err(error::link_name_outside_extern(span));
        }
        let kind = self.peek();
        match kind {
            TokenKind::Import => self.parse_import(),
            TokenKind::Struct => self.parse_struct(),
            TokenKind::Enum => self.parse_enum(),
            TokenKind::Fn => self.parse_fn(),
            // `const` is a binding keyword everywhere else, so it is an item
            // only when a `fn` follows it. The second token is what says so,
            // and `peek_second` is the only lookahead the parser has.
            TokenKind::Const if self.peek_second() == TokenKind::Fn => self.parse_const_fn(),
            TokenKind::Impl => self.parse_impl(),
            TokenKind::Object => self.parse_object(),
            TokenKind::Extern => self.parse_extern(),
            // `pub` is a modifier, not an item: it wraps whatever item
            // follows it, and the item inside keeps its own shape.
            TokenKind::Pub => self.parse_public_item(),
            // Effect modifiers: `pure`, `io`, `unsafe` before `fn`.
            TokenKind::Pure => self.parse_effect_item(yashy_common::DeclaredEffect::Pure),
            TokenKind::Io => self.parse_effect_item(yashy_common::DeclaredEffect::Io),
            TokenKind::Unsafe => {
                // `unsafe` is only an effect modifier when followed by `fn`.
                // Otherwise it's the extern-block marker.
                if self.peek_second() == TokenKind::Fn {
                    self.parse_effect_item(yashy_common::DeclaredEffect::Unsafe)
                } else {
                    Err(error::item_expected(kind, self.peek_span()))
                }
            }
            _ => Err(error::item_expected(kind, self.peek_span())),
        }
    }
    /// `pub fn f() { ... }` -- an item another module may use.
    ///
    /// The `pub` wraps the item it applies to, so the modifier is one thing
    /// in one place instead of a field on every kind of item. The item inside
    /// is parsed by the ordinary item parser, which means `pub` works on
    /// every kind an item can be.
    fn parse_public_item(&mut self) -> Result<Item, ParseError> {
        let keyword = self.expect(TokenKind::Pub)?;
        let item = self.parse_item()?;
        Ok(Item::Public {
            item: Box::new(item),
            span: keyword.span,
        })
    }

    /// `pure fn f() { ... }` / `io fn f() { ... }` / `unsafe fn f() { ... }`.
    fn parse_effect_item(&mut self, effect: yashy_common::DeclaredEffect) -> Result<Item, ParseError> {
        let keyword = match effect {
            yashy_common::DeclaredEffect::Pure => self.expect(TokenKind::Pure)?,
            yashy_common::DeclaredEffect::Io => self.expect(TokenKind::Io)?,
            yashy_common::DeclaredEffect::Unsafe => self.expect(TokenKind::Unsafe)?,
            _ => return Err(error::item_expected(self.peek(), self.peek_span())),
        };
        let item = self.parse_item()?;
        Ok(Item::Effect {
            effect,
            item: Box::new(item),
            span: keyword.span,
        })
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
    ///
    /// A segment may be a keyword as well as an identifier. Module paths name
    /// things, and `std.io` is the standard input/output module: making `io` a
    /// modifier keyword must not rename the module or refuse the file that
    /// imports it. What is recorded is the segment's own text, so `io` here is
    /// the word `io`, not the modifier.
    fn import_segment(&mut self) -> Result<String, ParseError> {
        let kind = self.peek();
        if kind != TokenKind::Ident && !kind.is_keyword() {
            return Err(error::import_needs_a_path(self.peek_span()));
        }
        let token = self.advance();
        let name = self.text(token.span)?;
        if name.is_empty() {
            return Err(error::unreadable_text(token.span));
        }
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

    /// `impl Point { fn sum(self) -> i64 { ... } }`
    ///
    /// The block holds ordinary function items. The type the methods belong
    /// to is named once, and each method's first parameter is the receiver:
    /// `self` is written without a type, because the block already says what
    /// the receiver is.
    fn parse_impl(&mut self) -> Result<Item, ParseError> {
        let keyword = self.expect(TokenKind::Impl)?;
        let (type_name, _) = self.identifier()?;
        let open = self.expect(TokenKind::LBrace)?;
        let _ = self.push_open(TokenKind::LBrace, open.span)?;

        let mut methods = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.at_eof() {
            methods.push(self.parse_item()?);
        }
        let close = self.expect(TokenKind::RBrace)?;
        self.close_open(TokenKind::RBrace);

        Ok(Item::Impl {
            type_name,
            methods,
            span: cover(keyword.span, close.span),
        })
    }

    /// `object Counter { value: i64, fn get(self) -> i64 { ... } }`
    ///
    /// The fields and the methods live in one block, in any order. It is a
    /// struct and an `impl` block written together.
    fn parse_object(&mut self) -> Result<Item, ParseError> {
        let keyword = self.expect(TokenKind::Object)?;
        let (name, _name_span) = self.identifier()?;
        let open = self.expect(TokenKind::LBrace)?;
        let _ = self.push_open(TokenKind::LBrace, open.span)?;

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.at_eof() {
            // A method is a `fn`, with or without a `pub` in front of it. The
            // second token is what says so: `pub` on its own would be a
            // field, and the two are told apart by what follows.
            if self.check(TokenKind::Fn)
                || (self.check(TokenKind::Pub) && self.peek_second() == TokenKind::Fn)
            {
                methods.push(self.parse_item()?);
            } else {
                fields.push(self.parse_field_def()?);
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        self.close_open(TokenKind::RBrace);

        Ok(Item::Object {
            name,
            fields,
            methods,
            span: cover(keyword.span, close.span),
        })
    }

    /// `x: f64` -- one named field of a struct, or of a struct-shaped enum
    /// variant.
    ///
    /// A field may be marked `pub`, which is what lets another module read
    /// and write it. A field with no `pub` is private to the module that
    /// declares it.
    fn parse_field_def(&mut self) -> Result<FieldDef, ParseError> {
        let public = self.eat(TokenKind::Pub);
        let (name, name_span) = self.identifier()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type_expr()?;
        Ok(FieldDef {
            name,
            public,
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
        let type_params = self.parse_type_params()?;

        let open = self.expect(TokenKind::LParen)?;
        let _ = self.push_open(TokenKind::LParen, open.span)?;
        let (params, _) = self.parse_params_after_open()?;
        self.close_open(TokenKind::RParen);

        let ret = if self.eat(TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        // `link_name` belongs to an `extern` declaration. On a Yashy `fn` it is
        // a rule about the language rather than a token the parser cannot read,
        // so it gets its own code and a message that says where the clause goes.
        if self.at_word("link_name") {
            let span = self.peek_span();
            return Err(error::link_name_outside_extern(span));
        }

        // Clauses sit between the signature and the body: `requires` says what
        // the caller must establish, `ensures` what the function promises. Both
        // are optional and repeatable. Either order is accepted and kept as
        // written -- the formatter reprints the tokens rather than reordering
        // them -- and an empty list is the common case, so a function with no
        // clause parses exactly as it did before.
        let mut requires = Vec::new();
        let mut ensures = Vec::new();
        loop {
            match self.peek() {
                TokenKind::Requires => {
                    let _ = self.expect(TokenKind::Requires)?;
                    let expr = self.parse_expr()?;
                    let span = crate::exprs::expr_span(&expr);
                    requires.push(Contract { expr, span });
                }
                TokenKind::Ensures => {
                    let _ = self.expect(TokenKind::Ensures)?;
                    let expr = self.parse_expr()?;
                    let span = crate::exprs::expr_span(&expr);
                    ensures.push(Contract { expr, span });
                }
                _ => break,
            }
        }

        // The body is a seam: `parse_block` is Task 4's half of it and Task 5
        // finishes it. It is what makes the delimiters inside a function body
        // part of this task's stack rather than a hole in it.
        let body = self.parse_block()?;
        let span = cover(keyword.span, body.span);

        Ok(Item::Fn {
            name,
            type_params,
            params,
            ret,
            requires,
            ensures,
            body,
            span,
        })
    }

    /// `const fn d(a: P) -> f64 { ... }` -- a function the compiler runs.
    ///
    /// The body is read exactly as an ordinary function's is; what makes it a
    /// `const fn` is the `const` in front of it, and the checker reads that
    /// as "this body may only use what a compile-time evaluation can do".
    ///
    /// There are no type parameters: a `const fn` is checked once, for one
    /// signature, so a generic one would have to be checked once per
    /// substitution and evaluated per substitution too. The language has no
    /// way to write that call site yet.
    fn parse_const_fn(&mut self) -> Result<Item, ParseError> {
        let const_keyword = self.expect(TokenKind::Const)?;
        let fn_keyword = self.expect(TokenKind::Fn)?;
        let (name, _) = self.identifier()?;

        let open = self.expect(TokenKind::LParen)?;
        let _ = self.push_open(TokenKind::LParen, open.span)?;
        let (params, _) = self.parse_params_after_open()?;
        self.close_open(TokenKind::RParen);

        let ret = if self.eat(TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        // A `const fn` runs at compile time and has no call site to check a
        // clause against, so a clause written here is refused where it is
        // written rather than silently dropped: a contract the compiler ignores
        // is worse than one it rejects.
        if matches!(self.peek(), TokenKind::Requires | TokenKind::Ensures) {
            return Err(error::ParseError::new(
                self.peek_span(),
                "a `const fn` cannot declare a contract: it has no call site to check one against",
            ));
        }

        let body = self.parse_block()?;
        let span = cover(const_keyword.span, body.span);
        let _ = fn_keyword;

        Ok(Item::ConstFn {
            name,
            params,
            ret,
            body,
            span,
        })
    }

    /// `[T, U]` -- the type parameters of a generic function, in order.
    ///
    /// Empty when the name is followed by `(`, which is what makes the
    /// spelling optional without a second token of lookahead: a function's
    /// argument list is the only other thing that can follow its name.
    ///
    /// The names are parsed here and judged by the checker: a duplicate, a
    /// name that shadows a declared type, and a method that writes one are
    /// all its business, not the parser's.
    fn parse_type_params(&mut self) -> Result<Vec<(String, Span)>, ParseError> {
        if !self.check(TokenKind::LBracket) {
            return Ok(Vec::new());
        }
        let open = self.expect(TokenKind::LBracket)?;
        let _ = self.push_open(TokenKind::LBracket, open.span)?;
        let mut params = Vec::new();
        loop {
            let (name, span) = self.identifier()?;
            params.push((name, span));
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let _ = self.expect(TokenKind::RBracket)?;
        self.close_open(TokenKind::RBracket);
        Ok(params)
    }

    /// `a: P` -- one parameter of a function.
    ///
    /// Always named and always annotated (spec §3.2): the annotation is required
    /// at the boundary, which is exactly where inference has nothing to infer
    /// from.
    pub(crate) fn parse_param(&mut self) -> Result<Param, ParseError> {
        let (name, name_span) = self.identifier()?;
        if name == "self" {
            // `self` is the receiver, and its type is the type the `impl`
            // block is for. The name `Self` is what the checker resolves to
            // that type, so the parameter needs no annotation of its own.
            //
            // A receiver *may* carry an annotation when it is written
            // `self: *mut T`: that is a pointer receiver, which receives the
            // object's address so it can mutate it. The spelling is parsed
            // here and judged by the checker, which knows the `impl` block's
            // type.
            if self.eat(TokenKind::Colon) {
                let ty = self.parse_type_expr()?;
                return Ok(Param {
                    span: cover(name_span, ty_span(&ty)),
                    name,
                    ty,
                });
            }
            return Ok(Param {
                name,
                ty: TypeExpr::Name("Self".to_string(), name_span),
                span: name_span,
            });
        }
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type_expr()?;
        Ok(Param {
            name,
            span: cover(name_span, ty_span(&ty)),
            ty,
        })
    }

    /// The parameter list of a function or an `extern` declaration, once its `(`
    /// has been consumed and pushed.
    ///
    /// Consumes the closing `)` but does not pop it: the caller owns the
    /// delimiter stack so that an unclosed `(` is reported against the item that
    /// opened it.
    fn parse_params_after_open(&mut self) -> Result<(Vec<Param>, Span), ParseError> {
        let mut params = Vec::new();
        while !self.check(TokenKind::RParen) {
            // `...` is the C variadic marker; a Yashy `fn` has a fixed
            // signature, so the marker belongs only to an `extern`
            // declaration, which reads its parameters with its own loop.
            if self.check(TokenKind::DotDotDot) {
                return Err(error::variadic_only_on_extern(self.peek_span()));
            }
            params.push(self.parse_param()?);
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RParen)?;
        Ok((params, close.span))
    }

    /// The parameter list of an `extern` declaration, once its `(` has been
    /// consumed and pushed: like a function's, but `...` may end it.
    ///
    /// The marker is accepted only in the last position. Everything before it is
    /// an ordinary named, annotated parameter, because those are the arguments
    /// Yashy can check; what follows the marker is C's business.
    fn parse_extern_params_after_open(&mut self) -> Result<(Vec<Param>, bool, Span), ParseError> {
        let mut params = Vec::new();
        let mut variadic = false;
        while !self.check(TokenKind::RParen) {
            if self.check(TokenKind::DotDotDot) {
                let span = self.peek_span();
                self.advance();
                variadic = true;
                // Nothing may follow `...`: an argument after it has no
                // parameter to be checked against.
                if !self.check(TokenKind::RParen) {
                    return Err(error::variadic_must_be_last(span));
                }
                break;
            }
            params.push(self.parse_param()?);
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RParen)?;
        Ok((params, variadic, close.span))
    }

    /// `extern "C" link "m" { unsafe fn puts(s: *const u8) -> i32; }`
    ///
    /// Both the braced block and the single-declaration form are accepted:
    /// `extern "C" unsafe fn puts(s: *const u8) -> i32;`. The AST has one shape
    /// for both — a list of declarations — so a single declaration is a
    /// one-element list, not a second variant.
    fn parse_extern(&mut self) -> Result<Item, ParseError> {
        let keyword = self.expect(TokenKind::Extern)?;

        let abi = if self.check(TokenKind::Str) {
            Some(self.extern_string("ABI")?)
        } else {
            None
        };
        let link = if self.at_word("link") {
            self.advance();
            Some(self.extern_string("library name")?)
        } else {
            None
        };

        if self.check(TokenKind::LBrace) {
            let open = self.expect(TokenKind::LBrace)?;
            let _ = self.push_open(TokenKind::LBrace, open.span)?;
            let mut decls = Vec::new();
            while !self.check(TokenKind::RBrace) && !self.check(TokenKind::Eof) {
                decls.push(self.parse_extern_decl()?);
            }
            let close = self.expect(TokenKind::RBrace)?;
            self.close_open(TokenKind::RBrace);
            if decls.is_empty() {
                return Err(error::extern_block_needs_a_declaration(cover(
                    keyword.span,
                    close.span,
                )));
            }
            Ok(Item::Extern {
                abi,
                link,
                decls,
                span: cover(keyword.span, close.span),
            })
        } else {
            let decl = self.parse_extern_decl()?;
            let span = cover(keyword.span, decl.span);
            Ok(Item::Extern {
                abi,
                link,
                decls: vec![decl],
                span,
            })
        }
    }

    /// One declaration: `unsafe fn name(params) -> ret link_name "symbol";`.
    ///
    /// The `unsafe` marker is required, not optional. It is checked before the
    /// `fn` is read so the diagnostic points at the declaration the user forgot
    /// it on.
    fn parse_extern_decl(&mut self) -> Result<ExternDecl, ParseError> {
        let start = self.peek_span();
        if !self.check(TokenKind::Unsafe) {
            return Err(error::extern_declaration_needs_unsafe(start));
        }
        self.advance();
        self.expect(TokenKind::Fn)?;
        let (name, _) = self.identifier()?;

        let open = self.expect(TokenKind::LParen)?;
        let _ = self.push_open(TokenKind::LParen, open.span)?;
        let (params, variadic, _) = self.parse_extern_params_after_open()?;
        self.close_open(TokenKind::RParen);

        let ret = if self.eat(TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        // `link_name "symbol"` is optional and sits between the signature and
        // the semicolon, so a declaration that omits it parses exactly as it did
        // before and its name stays the symbol.
        let symbol = if self.at_word("link_name") {
            Some(self.parse_link_name()?)
        } else {
            None
        };
        let semicolon = self.expect(TokenKind::Semi)?;
        Ok(ExternDecl {
            name,
            symbol,
            params,
            variadic,
            ret,
            span: cover(start, semicolon.span),
        })
    }

    /// `link_name "symbol"`: the clause that renames the linker symbol.
    ///
    /// The value has to be a literal: the symbol is decided while the program is
    /// compiled, and an interpolated name is not known until it runs. Empty is
    /// refused here rather than left to the linker, whose message names a symbol
    /// the reader never wrote.
    fn parse_link_name(&mut self) -> Result<String, ParseError> {
        self.advance();
        if !self.check(TokenKind::Str) {
            return Err(error::link_name_needs_a_string(self.peek_span()));
        }
        let token = self.expect(TokenKind::Str)?;
        let raw = self.text(token.span)?;
        let decoded = yashy_lexer::StringLit::decode(&raw)
            .map_err(|_| error::link_name_needs_a_string(token.span))?;
        if decoded.value.is_empty() {
            return Err(error::link_name_must_not_be_empty(token.span));
        }
        if decoded.value.contains("${") {
            return Err(error::link_name_needs_a_string(token.span));
        }
        Ok(decoded.value)
    }

    /// One string literal in an `extern` header: the ABI or a library name.
    fn extern_string(&mut self, what: &str) -> Result<String, ParseError> {
        let token = self.expect(TokenKind::Str)?;
        let raw = self.text(token.span)?;
        let decoded = yashy_lexer::StringLit::decode(&raw).map_err(|_| {
            self.error(
                token.span,
                format!("the {what} must be a valid string literal"),
            )
        })?;
        if decoded.value.is_empty() {
            return Err(self.error(token.span, format!("the {what} must not be empty")));
        }
        if decoded.value.contains("${") {
            return Err(self.error(
                token.span,
                format!("the {what} must be a plain string, not an interpolation"),
            ));
        }
        Ok(decoded.value)
    }

    /// True when the next token is the identifier `word`.
    ///
    /// `link` is not a keyword, so it has to be recognised by its text. Reading
    /// the text through the same span the lexer gave the token avoids any second
    /// opinion about what an identifier is.
    fn at_word(&self, word: &str) -> bool {
        self.check(TokenKind::Ident)
            && self
                .text(self.peek_span())
                .map(|text| text == word)
                .unwrap_or(false)
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
        | crate::ast::TypeExpr::Ptr { span, .. }
        | crate::ast::TypeExpr::Slice(_, span)
        | crate::ast::TypeExpr::FixedArray(_, _, span)
        | crate::ast::TypeExpr::View(_, span)
        | crate::ast::TypeExpr::Unit(span) => *span,
        crate::ast::TypeExpr::Result(_, _, span) => *span,
        crate::ast::TypeExpr::Map(_, _, span) => *span,
        crate::ast::TypeExpr::Channel(_, span) => *span,
        crate::ast::TypeExpr::Fn { span, .. } => *span,
        crate::ast::TypeExpr::Tuple(_, span) => *span,
    }
}
