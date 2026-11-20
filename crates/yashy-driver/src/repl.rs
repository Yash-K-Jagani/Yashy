//! The REPL's state machine, independent of the terminal and the toolchain.
//!
//! This is a *compiling* REPL, not a second implementation of Yashy. Each line
//! is checked by the real checker; an expression is turned into a small program
//! that prints it, compiled and run by the same pipeline a file build uses.
//! Declarations are retained as source text, so the next line sees them. The
//! limits are stated rather than hidden: no FFI, threads, async, imports or
//! filesystem writes, and no interpreter for the whole language.

use yashy_hir::hir::{expr_ty, HirExpr, HirItem, HirProgram, HirStmt};

/// What one input line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Empty,
    Exit,
    Help,
    Clear,
    Declarations,
    Declaration,
    Expression,
}

/// State kept between lines.
#[derive(Clone, Debug, Default)]
pub struct Repl {
    declarations: String,
    accepted: usize,
}

impl Repl {
    pub fn new() -> Self {
        Self::default()
    }

    /// The source prefix every generated program starts with.
    pub fn declarations(&self) -> &str {
        &self.declarations
    }

    pub fn accepted(&self) -> usize {
        self.accepted
    }

    pub fn clear(&mut self) {
        self.declarations.clear();
        self.accepted = 0;
    }

    pub fn accept(&mut self, line: &str) {
        self.declarations.push_str(line.trim());
        self.declarations.push('\n');
        self.accepted += 1;
    }

    /// Classify a line, refusing the forms the REPL deliberately does not
    /// support. Refusals name the compiler verb that does support them.
    pub fn classify(&self, line: &str) -> Result<Line, String> {
        let text = without_strings(line.trim());
        if text.is_empty() {
            return Ok(Line::Empty);
        }
        match text.as_str() {
            ":quit" | ":exit" => return Ok(Line::Exit),
            ":help" => return Ok(Line::Help),
            ":clear" => return Ok(Line::Clear),
            ":decls" => return Ok(Line::Declarations),
            _ => {}
        }
        for (word, alternative) in [
            ("extern", "an `extern` block"),
            ("unsafe", "an `unsafe` call"),
            ("import", "an import"),
            ("async", "an `async fn` or `.await`"),
            ("thread", "a thread"),
            ("thread_spawn", "a spawned task or thread"),
            ("spawn", "a spawned task or thread"),
            ("write_file", "a write to the filesystem"),
            ("delete_file", "a delete on the filesystem"),
            ("shell", "a shell call"),
        ] {
            if has_word(&text, word) {
                return Err(format!(
                    "the REPL cannot evaluate {alternative}; use `yashyc run` on a file instead"
                ));
            }
        }
        for word in [
            "fn", "struct", "enum", "trait", "impl", "object", "macro", "const", "let", "cfg",
            "repr",
        ] {
            if has_word_prefix(&text, word) {
                return Ok(Line::Declaration);
            }
        }
        Ok(Line::Expression)
    }

    /// A whole program for one expression: the retained declarations plus a
    /// fresh `main` that prints the expression's value.
    pub fn program_for(&self, expression: &str) -> String {
        format!(
            "{}fn main() {{\n    print({});\n}}\n",
            self.declarations,
            expression.trim()
        )
    }

    /// A whole program for checking a declaration before it is retained.
    pub fn program_for_declaration(&self, declaration: &str) -> String {
        format!(
            "{}{}\nfn main() {{}}\n",
            self.declarations,
            declaration.trim()
        )
    }

    /// The type the checker gave the expression printed by `main`.
    pub fn result_type(hir: &HirProgram) -> Option<String> {
        let main = hir.items.iter().find_map(|item| match item {
            HirItem::Fn(function) if function.name == "main" => Some(function),
            _ => None,
        })?;
        let printed = main
            .body
            .stmts
            .iter()
            .rev()
            .find_map(|statement| match statement {
                HirStmt::Expr(expression, _, _) => match expression {
                    // The program for an expression ends in `print(value)`.
                    // Pick that call, not a later cleanup call whose first
                    // argument is a temporary string.
                    HirExpr::Call { callee, args, .. }
                        if callee
                            .last()
                            .is_some_and(|name| name.contains("print")) =>
                    {
                        args.first()
                    }
                    _ => None,
                },
                _ => None,
            })?;
        let mut result = expr_ty(printed).to_string();
        // `print(3)` lowers to a string-print call whose argument is already
        // `str_from_i64(3)`. The REPL reports the expression's type, not the
        // temporary string the printer made from it.
        if result == "string" {
            if let HirExpr::Call { callee, args, .. } = printed {
                if callee
                    .last()
                    .is_some_and(|name| name.contains("str_from"))
                {
                    if let Some(value) = args.first() {
                        result = expr_ty(value).to_string();
                    }
                }
            }
        }
        Some(result)
    }

    pub fn help() -> &'static str {
        "\
:help     this page
:decls    print the declarations kept so far
:clear    forget every declaration
:quit     leave the REPL

A declaration (fn, struct, const, let, ...) is checked and kept. An expression
is compiled into a small program that prints its value; the value's type comes
from the checker. FFI, threads, async, imports and filesystem writes are not
supported here: put those in a file and use `yashyc run`."
    }
}

/// Drop the contents of string literals before keyword checks: a word inside
/// a string is data, not a form the REPL has to refuse.
fn without_strings(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for character in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
            out.push('"');
        } else {
            out.push(character);
        }
    }
    if in_string {
        // Keep a placeholder so a line that is only a string still classifies
        // as an expression rather than as an empty line.
        out.push_str("s\"");
    }
    out
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|part| part == word)
}

fn has_word_prefix(text: &str, word: &str) -> bool {
    text.split_whitespace().next().is_some_and(|first| {
        if first == word {
            return true;
        }
        first.starts_with(word)
            && first[word.len()..]
                .chars()
                .next()
                .is_some_and(|next| matches!(next, '(' | '{' | '['))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_lines_are_classified() {
        let repl = Repl::new();
        assert_eq!(repl.classify("").unwrap(), Line::Empty);
        assert_eq!(repl.classify(":quit").unwrap(), Line::Exit);
        assert_eq!(repl.classify(":help").unwrap(), Line::Help);
        assert_eq!(repl.classify("1 + 2").unwrap(), Line::Expression);
        assert_eq!(
            repl.classify("fn add(int a, int b) -> int { return a + b; }")
                .unwrap(),
            Line::Declaration
        );
        assert_eq!(
            repl.classify("struct Point { x: int }").unwrap(),
            Line::Declaration
        );
    }

    #[test]
    fn unsupported_forms_are_refused_with_the_compiler_alternative() {
        let repl = Repl::new();
        for line in [
            "extern fn puts(string s) -> int;",
            "async fn work() -> Task[int] { return 1; }",
            "thread_spawn(work)",
        ] {
            let error = repl.classify(line).unwrap_err();
            assert!(error.contains("yashyc run"), "{error}");
        }
        // A word inside a string literal is data, not a keyword.
        assert_eq!(
            repl.classify("\"extern is a word in this string\"")
                .unwrap(),
            Line::Expression
        );
    }

    #[test]
    fn declarations_are_retained_and_clear_forgets_them() {
        let mut repl = Repl::new();
        repl.accept("fn double(int x) -> int { return x * 2; }");
        assert_eq!(repl.accepted(), 1);
        assert!(repl.program_for("double(21)").contains("fn double"));
        assert!(repl.program_for("double(21)").contains("print(double(21))"));
        repl.clear();
        assert_eq!(repl.accepted(), 0);
        assert!(!repl.program_for("double(21)").contains("fn double"));
    }
}
