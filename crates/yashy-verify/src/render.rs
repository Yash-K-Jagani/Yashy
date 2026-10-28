//! Printing a checked expression back as source.
//!
//! A proof that fails has to say *what* it could not prove, and the only text
//! the user can check is the clause they wrote. So the renderer prints the HIR
//! expression with the precedence the source had: the printed clause and the
//! written one are the same string for everything the contract language
//! allows.
//!
//! The renderer is for messages, not for round-tripping. The formatter owns the
//! canonical spelling; this owns "what the user wrote, near enough to find".

use yashy_hir::hir::{BinOp, HirExpr, Literal};

/// Binding strength, loosest first. A child is parenthesised when it binds less
/// tightly than the context it sits in.
const OR: u8 = 1;
const AND: u8 = 2;
const COMPARE: u8 = 3;
const ADDITIVE: u8 = 4;
const MULTIPLICATIVE: u8 = 5;
const UNARY: u8 = 6;
const POSTFIX: u8 = 7;
const ATOM: u8 = 8;

/// `expr` as source text.
pub fn expr(expr: &HirExpr) -> String {
    render(expr, 0)
}

fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Or => OR,
        BinOp::And => AND,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => COMPARE,
        BinOp::Add | BinOp::Sub => ADDITIVE,
        BinOp::Mul | BinOp::Div | BinOp::Rem => MULTIPLICATIVE,
    }
}

fn render(node: &HirExpr, parent: u8) -> String {
    let (text, precedence) = match node {
        HirExpr::Literal(literal, ..) => (literal_text(literal), ATOM),
        HirExpr::Ident(path, ..) => (path.join("."), ATOM),
        HirExpr::Binary { op, lhs, rhs, .. } => {
            let strength = precedence(*op);
            // The right operand needs to bind strictly tighter, so `a - (b - c)`
            // keeps its parentheses while `(a - b) - c` does not need them.
            let left = render(lhs, strength);
            let right = render(rhs, strength + 1);
            (format!("{left} {} {right}", op.symbol()), strength)
        }
        HirExpr::Unary { op, operand, .. } => {
            (format!("{}{}", op.symbol(), render(operand, UNARY)), UNARY)
        }
        HirExpr::Move(inner, ..) => (format!("move {}", render(inner, UNARY)), UNARY),
        HirExpr::AddrOf { name, .. } => (format!("&{name}"), UNARY),
        HirExpr::Call { callee, args, .. } => {
            let text = match callee.as_slice() {
                // The checker lowers `xs.len()` to a call, so that is the one
                // spelling the user can have written.
                [name] if (name == "len" || name == "view_len") && args.len() == 1 => {
                    format!("{}.len()", render(&args[0], POSTFIX))
                }
                _ => {
                    let args = args.iter().map(expr).collect::<Vec<_>>().join(", ");
                    format!("{}({args})", callee.join("."))
                }
            };
            (text, POSTFIX)
        }
        HirExpr::FieldAccess { base, field, .. } => {
            (format!("{}.{field}", render(base, POSTFIX)), POSTFIX)
        }
        HirExpr::Index { base, index, .. } => (
            format!("{}[{}]", render(base, POSTFIX), expr(index)),
            POSTFIX,
        ),
        HirExpr::Cast { expr, ty, .. } => (format!("{} as {ty}", render(expr, POSTFIX)), POSTFIX),
        HirExpr::Try { expr, .. } => (format!("{}?", render(expr, POSTFIX)), POSTFIX),
        HirExpr::StructLit { path, fields, .. } => {
            let fields = fields
                .iter()
                .map(|(name, value, _)| format!("{name}: {}", expr(value)))
                .collect::<Vec<_>>()
                .join(", ");
            (format!("{} {{ {fields} }}", path.join(".")), ATOM)
        }
        HirExpr::SliceLit {
            elem_ty, elements, ..
        } => {
            let elements = elements.iter().map(expr).collect::<Vec<_>>().join(", ");
            (format!("[]{elem_ty} {{ {elements} }}"), ATOM)
        }
        HirExpr::FixedArrayLit { elements, .. } => {
            let elements = elements.iter().map(expr).collect::<Vec<_>>().join(", ");
            (format!("[{elements}]"), ATOM)
        }
        HirExpr::TupleLit { elements, .. } => {
            let elements = elements.iter().map(expr).collect::<Vec<_>>().join(", ");
            (format!("({elements})"), ATOM)
        }
        HirExpr::View {
            base, start, end, ..
        } => (
            format!("{}[{}..{}]", render(base, POSTFIX), expr(start), expr(end)),
            POSTFIX,
        ),
        // Everything below is outside the contract language. A clause that
        // reaches one is printed as a shape rather than as a wrong expression:
        // a message that names the wrong source text is worse than one that
        // says it cannot quote it.
        HirExpr::SwitchExpr { .. } => ("switch { .. }".to_string(), ATOM),
        HirExpr::Lambda { .. } => ("|..| { .. }".to_string(), ATOM),
        HirExpr::HigherOrder { op, .. } => (format!("<{op:?} call>"), POSTFIX),
        HirExpr::Capture { .. } => ("<captured value>".to_string(), ATOM),
        HirExpr::CallCapture { args, .. } => {
            let args = args.iter().map(expr).collect::<Vec<_>>().join(", ");
            (format!("<captured function>({args})"), POSTFIX)
        }
    };
    if precedence < parent {
        format!("({text})")
    } else {
        text
    }
}

fn literal_text(literal: &Literal) -> String {
    match literal {
        Literal::I64(n) => n.to_string(),
        Literal::F64(f) => format!("{f}"),
        Literal::Bool(b) => b.to_string(),
        Literal::Str(s) => format!("{:?}", s.value),
        Literal::Char(b) => format!("'\\x{b:02X}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yashy_common::Span;
    use yashy_hir::ty::{Type, TypeKind};

    fn int() -> Type {
        Type {
            kind: TypeKind::I64,
            span: Span::new(0, 0),
        }
    }

    fn bool_ty() -> Type {
        Type {
            kind: TypeKind::Bool,
            span: Span::new(0, 0),
        }
    }

    fn name(text: &str) -> HirExpr {
        HirExpr::Ident(vec![text.to_string()], Span::new(0, 0), int())
    }

    fn number(n: i64) -> HirExpr {
        HirExpr::Literal(Literal::I64(n), Span::new(0, 0), int())
    }

    fn binary(op: BinOp, lhs: HirExpr, rhs: HirExpr, ty: Type) -> HirExpr {
        HirExpr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span: Span::new(0, 0),
            ty,
            fault: None,
        }
    }

    #[test]
    fn a_clause_is_printed_the_way_it_was_written() {
        let clause = binary(BinOp::Gt, name("n"), number(0), bool_ty());
        assert_eq!(expr(&clause), "n > 0");
    }

    #[test]
    fn arithmetic_keeps_the_grouping_that_changes_its_meaning() {
        let grouped = binary(
            BinOp::Mul,
            binary(BinOp::Add, name("a"), name("b"), int()),
            name("c"),
            int(),
        );
        assert_eq!(expr(&grouped), "(a + b) * c");

        let right = binary(
            BinOp::Sub,
            name("a"),
            binary(BinOp::Sub, name("b"), name("c"), int()),
            int(),
        );
        assert_eq!(expr(&right), "a - (b - c)");

        // Associative operators do not need the parentheses.
        let left = binary(
            BinOp::Add,
            binary(BinOp::Add, name("a"), name("b"), int()),
            name("c"),
            int(),
        );
        assert_eq!(expr(&left), "a + b + c");
    }

    #[test]
    fn a_call_and_a_field_read_print_as_the_source_spelling() {
        let call = HirExpr::Call {
            callee: vec!["len".to_string()],
            args: vec![name("xs")],
            unsafe_call: false,
            span: Span::new(0, 0),
            ty: int(),
        };
        assert_eq!(expr(&call), "xs.len()");

        let field = HirExpr::FieldAccess {
            base: Box::new(name("p")),
            field: "x".to_string(),
            index: 0,
            span: Span::new(0, 0),
            ty: int(),
        };
        assert_eq!(expr(&field), "p.x");
    }
}
