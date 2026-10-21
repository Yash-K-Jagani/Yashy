//! Type expressions: `i64`, `Option[T]`, `Result[T, E]`, `(i64, f64)`, `()`.
//!
//! # Why this lives in Task 4 and not Task 6
//!
//! The plan lists `parse_type_expr` under Task 6, next to `parse_expr`. It is
//! built here because item parsing cannot be written without it: a struct field
//! is `name : type`, a parameter is `name : type`, and a return type is
//! `-> type`. Leaving them out would mean either refusing to parse a struct at
//! all until Task 6, or guessing a type and letting a later stage trust it --
//! which is the one thing the untyped AST exists to prevent (see
//! [`crate::ast`]).
//!
//! So this is the real implementation, not a placeholder, and Task 6 inherits
//! it rather than writing it twice. Two implementations of "what is a type" are
//! two chances to disagree about whether `Option[i64]` is one node or two.
//!
//! # `()` is [`TypeExpr::Unit`]
//!
//! An empty parenthesised type is the unit type, never an empty
//! [`TypeExpr::Tuple`]. Two spellings of one type would make `PartialEq` call
//! identical types different, and would make every AST golden dump depend on
//! which spelling the parser happened to reach for.
//!
//! # Dotted names
//!
//! `pkg.Thing` is one [`TypeExpr::Name`] whose text is `pkg.Thing`, not a tree
//! of segments: the AST has one `String` for a name and no node for a path, and
//! inventing a path type here would be a contract change five stages code
//! against.

use otter_common::Span;
use otter_lexer::TokenKind;

use crate::ast::{PtrKind, TypeExpr};
use crate::error::ParseError;
use crate::parser::{cover, Parser};

impl<'src> Parser<'src> {
    /// Parses one type, in any position a type can appear.
    ///
    /// The whole type grammar:
    ///
    /// ```text
    /// type    := `(` `)`                    -- the unit type
    ///          | `(` type (`,` type)* `)`   -- a tuple type
    ///          | name                       -- `i64`, `f64`, `Point`, `pkg.Thing`
    ///          | `Option` `[` type `]`
    ///          | `Result` `[` type `,` type `]`
    ///          | `*` [`mut` | `const`] type -- a pointer
    ///          | `fn` `(` [type (`,` type)*] `)` [`->` type]
    /// name    := ident (`.` ident)*
    /// ```
    ///
    /// # Errors
    ///
    /// A [`ParseError`] at the offending token. `Option` and `Result` are
    /// recognised by name because they are the only generic spellings the
    /// language has; a `[` after any other name is a diagnostic rather than a
    /// guess, since Otter has no generics (spec §4.6) and silently reading
    /// `Vec[i64]` as a type named `Vec` would move the mistake into the type
    /// checker, where it would be reported as something else entirely.
    pub fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        match self.peek() {
            TokenKind::LParen => self.parse_paren_type(),
            TokenKind::Ident => self.parse_named_type(),
            TokenKind::Star => self.parse_pointer_type(),
            TokenKind::LBracket => self.parse_slice_type(),
            TokenKind::Fn => self.parse_fn_type(),
            _ => Err(self.type_expected_error()),
        }
    }

    /// `fn(T, U) -> R` - a first-class function type.
    ///
    /// The arrow is optional and means `-> ()` when omitted, matching a
    /// function declaration whose return annotation is absent. The type has no
    /// parameter names: a value's type describes the calling convention, not
    /// the names the callee happens to use.
    fn parse_fn_type(&mut self) -> Result<TypeExpr, ParseError> {
        let keyword = self.expect(TokenKind::Fn)?;
        let open = self.expect(TokenKind::LParen)?;
        let _ = self.push_open(TokenKind::LParen, open.span)?;

        let mut params = Vec::new();
        while !self.check(TokenKind::RParen) {
            params.push(self.parse_type_expr()?);
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RParen)?;
        self.close_open(TokenKind::RParen);

        let ret = if self.eat(TokenKind::Arrow) {
            self.parse_type_expr()?
        } else {
            TypeExpr::Unit(close.span)
        };
        Ok(TypeExpr::Fn {
            params,
            ret: Box::new(ret),
            span: cover(keyword.span, self.previous_span()),
        })
    }

    /// `[]T` — a slice. The brackets are empty; the element type follows.
    ///
    /// A non-empty bracket pair in type position is a future fixed-size array,
    /// not a spelling of this type, so it is rejected here rather than read as
    /// something else.
    fn parse_slice_type(&mut self) -> Result<TypeExpr, ParseError> {
        let open = self.expect(TokenKind::LBracket)?;
        let _ = self.push_open(TokenKind::LBracket, open.span)?;
        let close = self.expect(TokenKind::RBracket)?;
        self.close_open(TokenKind::RBracket);
        let element = self.parse_type_expr()?;
        Ok(TypeExpr::Slice(
            Box::new(element),
            cover(open.span, close.span),
        ))
    }

    /// `*T`, `*mut T`, or `*const T`.
    ///
    /// `*mut T` is Otter's owning pointer and predates the FFI edge. `*const T`
    /// and the bare `*T` arrive with C interop; only the checker, which knows
    /// whether the type was written inside an `extern` declaration, can say
    /// whether any of them is a C pointer.
    fn parse_pointer_type(&mut self) -> Result<TypeExpr, ParseError> {
        let star = self.expect(TokenKind::Star)?;
        let kind = match self.peek() {
            // `const` is a keyword, not an identifier, so it is the one
            // mutability word that never reaches `identifier()`.
            TokenKind::Const => {
                self.advance();
                PtrKind::Const
            }
            TokenKind::Ident => {
                let (word, word_span) = self.identifier()?;
                match word.as_str() {
                    "mut" => PtrKind::Mut,
                    "const" => PtrKind::Const,
                    // `*i8` is a bare pointer: the word we just read was the
                    // start of the pointee's name, not a mutability keyword.
                    // Finish that name here (it may be dotted or generic) rather
                    // than trying to rewind a cursor that cannot rewind.
                    other => {
                        let pointee = self.finish_named_type(other.to_string(), word_span)?;
                        return Ok(TypeExpr::Ptr {
                            kind: PtrKind::Bare,
                            pointee: Box::new(pointee),
                            span: cover(star.span, self.previous_span()),
                        });
                    }
                }
            }
            _ => {
                return Err(self.error(
                    self.peek_span(),
                    format!(
                        "expected `mut`, `const`, or a type after `*`, found {}",
                        self.peek().describe()
                    ),
                ))
            }
        };
        let pointee = self.parse_type_expr()?;
        Ok(TypeExpr::Ptr {
            kind,
            pointee: Box::new(pointee),
            span: cover(star.span, self.previous_span()),
        })
    }

    /// `()` or `(T, U)`.
    fn parse_paren_type(&mut self) -> Result<TypeExpr, ParseError> {
        let open = self.expect(TokenKind::LParen)?;
        let _ = self.push_open(TokenKind::LParen, open.span)?;

        if self.eat(TokenKind::RParen) {
            self.close_open(TokenKind::RParen);
            return Ok(TypeExpr::Unit(cover(open.span, self.previous_span())));
        }

        let mut elements = Vec::new();
        loop {
            elements.push(self.parse_type_expr()?);
            // The loop guard is what makes the trailing `|| check(RParen)` clause
            // redundant in the other five comma loops; here there is no guard
            // (`loop {}`), so this clause is the only thing stopping `(i64,)`
            // from reading a second (missing) element.
            if !self.eat(TokenKind::Comma) || self.check(TokenKind::RParen) {
                break;
            }
        }
        let close = self.expect(TokenKind::RParen)?;
        self.close_open(TokenKind::RParen);
        // A parenthesised list of one is a tuple of one. The parens were
        // written, so they are in the type, and the AST is explicit that a
        // tuple is never empty -- which leaves "one element" as the only reading
        // for `(i64)` that is not a second spelling of `i64`.
        Ok(TypeExpr::Tuple(elements, cover(open.span, close.span)))
    }

    /// A named type, and the two generic spellings that hang off one.
    fn parse_named_type(&mut self) -> Result<TypeExpr, ParseError> {
        let (first, start) = self.identifier()?;
        self.finish_named_type(first, start)
    }

    /// The rest of a named type once its first segment has been read.
    ///
    /// Split from [`Parser::parse_named_type`] because a bare pointer
    /// (`*u8`) consumes the first segment of its pointee before it knows the
    /// word was not `mut` or `const`.
    fn finish_named_type(&mut self, first: String, start: Span) -> Result<TypeExpr, ParseError> {
        let mut spelled = first;
        let mut end = start;

        // A dotted path is one name. The `.` loop is bounded by the token
        // stream, and each iteration consumes a `.` and an identifier, so it
        // terminates on anything a lexer can produce.
        while self.check(TokenKind::Dot) {
            self.advance();
            let (segment, segment_span) = self.identifier()?;
            spelled.push('.');
            spelled.push_str(&segment);
            end = segment_span;
        }

        if !self.check(TokenKind::LBracket) {
            return Ok(TypeExpr::Name(spelled, cover(start, end)));
        }

        match spelled.as_str() {
            "Option" => {
                self.open_bracket()?;
                let inner = self.parse_type_expr()?;
                self.close_bracket()?;
                Ok(TypeExpr::Option(
                    Box::new(inner),
                    cover(start, self.previous_span()),
                ))
            }
            "Result" => {
                self.open_bracket()?;
                let ok = self.parse_type_expr()?;
                if !self.eat(TokenKind::Comma) {
                    return Err(self.error(
                        self.peek_span(),
                        format!(
                            "`Result` takes two types, as in `Result[T, E]`, found {}",
                            self.peek().describe()
                        ),
                    ));
                }
                let err = self.parse_type_expr()?;
                self.close_bracket()?;
                Ok(TypeExpr::Result(
                    Box::new(ok),
                    Box::new(err),
                    cover(start, self.previous_span()),
                ))
            }
            "Channel" => {
                self.open_bracket()?;
                let element = self.parse_type_expr()?;
                self.close_bracket()?;
                Ok(TypeExpr::Channel(
                    Box::new(element),
                    cover(start, self.previous_span()),
                ))
            }
            // `map[K]V` - the key in brackets, the value after them. The two
            // types sit on opposite sides of the `]`, so this cannot reuse the
            // `Option[..]` shape.
            "map" => {
                self.open_bracket()?;
                let key = self.parse_type_expr()?;
                self.close_bracket()?;
                let value = self.parse_type_expr()?;
                Ok(TypeExpr::Map(
                    Box::new(key),
                    Box::new(value),
                    cover(start, self.previous_span()),
                ))
            }
            other => Err(self.error(
                self.peek_span(),
                format!("`{other}` is not a generic type: Otter has no generics"),
            )),
        }
    }

    /// The `[` of `Option[..]` or `Result[.., ..]`.
    ///
    /// Open and close are separate because the two brackets hold different
    /// numbers of types: `Option[T]` has one and `Result[T, E]` has a comma
    /// between them. A helper that closed the bracket itself could only have
    /// been right about one of them.
    fn open_bracket(&mut self) -> Result<(), ParseError> {
        let open = self.expect(TokenKind::LBracket)?;
        let _ = self.push_open(TokenKind::LBracket, open.span)?;
        Ok(())
    }

    /// The `]` that closes an `Option[..]` or a `Result[.., ..]`.
    fn close_bracket(&mut self) -> Result<(), ParseError> {
        self.expect(TokenKind::RBracket)?;
        self.close_open(TokenKind::RBracket);
        Ok(())
    }

    /// The failure for a token that cannot begin a type.
    ///
    /// Separate from [`Parser::expected_error`] because the half that helps is
    /// the list: a reader who wrote `x: }` needs to be told what a type can be,
    /// not only what `}` is.
    fn type_expected_error(&self) -> ParseError {
        self.error(
            self.peek_span(),
            format!(
                "expected a type: an identifier, `Option[..]`, `Result[.., ..]`, `(..)`, `()`, or a pointer, found {}",
                self.peek().describe()
            ),
        )
    }
}
