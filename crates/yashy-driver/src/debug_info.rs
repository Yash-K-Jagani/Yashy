//! LLVM debug metadata derived from HIR spans.
//!
//! The backend emits plain IR plus optional comment markers in a debug build:
//! `; yashy-loc:<file>:<start>:<end>` before each checked statement, and
//! `; yashy-var:<kind>:<name>:<slot>:<tag>` where a parameter or `let` binding
//! comes into scope. This pass turns those markers into `!dbg` attachments, a
//! `DILocalVariable` and an `llvm.dbg.declare` per variable, and the
//! `DICompileUnit`, `DIFile`, `DISubprogram` and `DILocation` metadata LLVM
//! needs for a line table. Normal builds emit no markers, so their IR stays
//! byte-identical.

use std::collections::{BTreeMap, BTreeSet};

use yashy_common::{FileId, SourceMap, Span};
use yashy_hir::hir::{HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt, HirSwitchArm};
use yashy_hir::ty::{Type, TypeKind};

/// The source facts one `DISubprogram` needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionLocation {
    pub symbol: String,
    pub name: String,
    pub file: String,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Point {
    file: String,
    line: u32,
    column: u32,
    start: u32,
    end: u32,
    /// The function the marker was emitted in, as an index into
    /// [`functions`]. A `DILocation` has to name its own `DISubprogram`;
    /// scoping every location to the first function makes a debugger attribute
    /// lines to the wrong function even when the addresses are right.
    scope: usize,
    /// The innermost lexical block containing the marker, as an index into
    /// [`blocks`]. `None` is the function's outermost block, whose scope is the
    /// `DISubprogram`.
    block: Option<usize>,
}

/// One lexical block (`{ ... }`) inside a user function.
///
/// DWARF scopes a variable to the block it is declared in, so a debugger shows
/// it only while that block is live. The compiler's HIR has the block tree and
/// its spans; the IR markers carry statement spans, so a marker's innermost
/// containing block is the scope it belongs to.
#[derive(Clone, Debug)]
struct Block {
    scope: usize,
    file: String,
    start: u32,
    end: u32,
    line: u32,
    column: u32,
}

/// The LLVM function that holds a user function's statement markers.
///
/// A normal function keeps its source name. An `async fn` keeps that name for
/// its *factory* — a few instructions that allocate a frame — while the body
/// is lowered into a separate `poll` function. The debugger wants the body:
/// pointing a `DISubprogram` at the factory would leave every location in the
/// poll with no scope at all.
fn body_symbol(function: &HirFn) -> String {
    if function.is_async {
        yashy_codegen::async_lower::lower_async(function).1
    } else {
        yashy_codegen::ir::fn_symbol(&function.name)
    }
}

/// Collect every block nested inside a user function, in source order.
///
/// The function's own body block is not one of them: its scope is the
/// `DISubprogram` itself, which is the scope every parameter already uses.
fn blocks(hir: &HirProgram, sources: &SourceMap, functions: &[FunctionLocation]) -> Vec<Block> {
    let mut out = Vec::new();
    for item in &hir.items {
        let HirItem::Fn(function) = item else {
            continue;
        };
        // Lambdas and other `$`-mangled helpers are emitted as their own
        // functions with no HIR of their own to walk; their markers borrow the
        // enclosing function's source and cannot be scoped here.
        if function.name.contains('$') {
            continue;
        }
        let symbol = body_symbol(function);
        let Some(scope) = functions.iter().position(|entry| entry.symbol == symbol) else {
            continue;
        };
        for stmt in &function.body.stmts {
            walk_stmt(stmt, function.file, scope, sources, &mut out);
        }
    }
    out
}

fn walk_block(
    block: &HirBlock,
    file: FileId,
    scope: usize,
    sources: &SourceMap,
    out: &mut Vec<Block>,
) {
    let location = sources.locate(file, block.span);
    out.push(Block {
        scope,
        file: sources.path(file).to_string(),
        start: block.span.start,
        end: block.span.end,
        line: location.line,
        column: location.col,
    });
    for stmt in &block.stmts {
        walk_stmt(stmt, file, scope, sources, out);
    }
}

fn walk_stmt(
    stmt: &HirStmt,
    file: FileId,
    scope: usize,
    sources: &SourceMap,
    out: &mut Vec<Block>,
) {
    match stmt {
        HirStmt::Expr(expr, _, _) => walk_expr(expr, file, scope, sources, out),
        HirStmt::Const { init, .. } | HirStmt::Var { init, .. } => {
            walk_expr(init, file, scope, sources, out)
        }
        HirStmt::Destructure { init, .. } => walk_expr(init, file, scope, sources, out),
        HirStmt::LetElse {
            init, else_block, ..
        } => {
            walk_expr(init, file, scope, sources, out);
            walk_block(else_block, file, scope, sources, out);
        }
        HirStmt::Assign { value, .. } => walk_expr(value, file, scope, sources, out),
        HirStmt::IndexAssign {
            base, index, value, ..
        } => {
            walk_expr(base, file, scope, sources, out);
            walk_expr(index, file, scope, sources, out);
            walk_expr(value, file, scope, sources, out);
        }
        HirStmt::FieldAssign { value, .. } | HirStmt::FieldPtrAssign { value, .. } => {
            walk_expr(value, file, scope, sources, out)
        }
        HirStmt::Return(expr, _) => {
            if let Some(expr) = expr {
                walk_expr(expr, file, scope, sources, out);
            }
        }
        HirStmt::If {
            init,
            cond,
            then_block,
            else_block,
            ..
        } => {
            if let Some(init) = init {
                walk_stmt(init, file, scope, sources, out);
            }
            walk_expr(cond, file, scope, sources, out);
            walk_block(then_block, file, scope, sources, out);
            if let Some(else_block) = else_block {
                walk_block(else_block, file, scope, sources, out);
            }
        }
        HirStmt::IfLet {
            subject,
            then_block,
            else_block,
            ..
        } => {
            walk_expr(subject, file, scope, sources, out);
            walk_block(then_block, file, scope, sources, out);
            if let Some(else_block) = else_block {
                walk_block(else_block, file, scope, sources, out);
            }
        }
        HirStmt::ForRange {
            start, end, body, ..
        } => {
            walk_expr(start, file, scope, sources, out);
            walk_expr(end, file, scope, sources, out);
            walk_block(body, file, scope, sources, out);
        }
        HirStmt::ForIn { iterable, body, .. } => {
            walk_expr(iterable, file, scope, sources, out);
            walk_block(body, file, scope, sources, out);
        }
        HirStmt::ForC {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(init) = init {
                walk_stmt(init, file, scope, sources, out);
            }
            if let Some(cond) = cond {
                walk_expr(cond, file, scope, sources, out);
            }
            if let Some(update) = update {
                walk_expr(update, file, scope, sources, out);
            }
            walk_block(body, file, scope, sources, out);
        }
        HirStmt::Switch { subject, arms, .. } => {
            walk_expr(subject, file, scope, sources, out);
            for arm in arms {
                walk_arm(arm, file, scope, sources, out);
            }
        }
        HirStmt::Defer { body, .. } | HirStmt::Scope { body, .. } => {
            walk_block(body, file, scope, sources, out)
        }
        HirStmt::Break(_) | HirStmt::Continue(_) | HirStmt::LoopInvariant(_) => {}
    }
}

fn walk_arm(
    arm: &HirSwitchArm,
    file: FileId,
    scope: usize,
    sources: &SourceMap,
    out: &mut Vec<Block>,
) {
    if let Some(guard) = &arm.guard {
        walk_expr(guard, file, scope, sources, out);
    }
    walk_expr(&arm.body, file, scope, sources, out);
}

fn walk_expr(
    expr: &HirExpr,
    file: FileId,
    scope: usize,
    sources: &SourceMap,
    out: &mut Vec<Block>,
) {
    match expr {
        HirExpr::UnsafeBlock { body, .. } => walk_block(body, file, scope, sources, out),
        HirExpr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, file, scope, sources, out);
            walk_expr(rhs, file, scope, sources, out);
        }
        HirExpr::Unary { operand, .. } | HirExpr::Move(operand, _, _) => {
            walk_expr(operand, file, scope, sources, out)
        }
        HirExpr::Call { args, .. } => {
            for arg in args {
                walk_expr(arg, file, scope, sources, out);
            }
        }
        HirExpr::StructLit { fields, .. } => {
            for (_, value, _) in fields {
                walk_expr(value, file, scope, sources, out);
            }
        }
        HirExpr::SliceLit { elements, .. } | HirExpr::FixedArrayLit { elements, .. } => {
            for element in elements {
                walk_expr(element, file, scope, sources, out);
            }
        }
        HirExpr::FieldAccess { base, .. } | HirExpr::Cast { expr: base, .. } => {
            walk_expr(base, file, scope, sources, out)
        }
        HirExpr::Index { base, index, .. } => {
            walk_expr(base, file, scope, sources, out);
            walk_expr(index, file, scope, sources, out);
        }
        HirExpr::View {
            base, start, end, ..
        } => {
            walk_expr(base, file, scope, sources, out);
            walk_expr(start, file, scope, sources, out);
            walk_expr(end, file, scope, sources, out);
        }
        HirExpr::TupleLit { elements, .. } => {
            for element in elements {
                walk_expr(element, file, scope, sources, out);
            }
        }
        HirExpr::Try { expr, .. } | HirExpr::Await { expr, .. } => {
            walk_expr(expr, file, scope, sources, out)
        }
        HirExpr::Spawn { callee, args, .. } => {
            walk_expr(callee, file, scope, sources, out);
            for arg in args {
                walk_expr(arg, file, scope, sources, out);
            }
        }
        HirExpr::SwitchExpr { subject, arms, .. } => {
            walk_expr(subject, file, scope, sources, out);
            for arm in arms {
                walk_arm(arm, file, scope, sources, out);
            }
        }
        // A lambda is emitted as its own function, so its body's blocks belong
        // to a different `DISubprogram` than the one being walked.
        HirExpr::Lambda { .. } => {}
        HirExpr::CallCapture { args, .. } => {
            for arg in args {
                walk_expr(arg, file, scope, sources, out);
            }
        }
        HirExpr::HigherOrder {
            receiver,
            func,
            init,
            ..
        } => {
            walk_expr(receiver, file, scope, sources, out);
            walk_expr(func, file, scope, sources, out);
            if let Some(init) = init {
                walk_expr(init, file, scope, sources, out);
            }
        }
        HirExpr::Literal(..)
        | HirExpr::Ident(..)
        | HirExpr::AddrOf { .. }
        | HirExpr::Capture { .. } => {}
    }
}

/// The innermost block that contains `[start, end)` in one function.
fn innermost_block(
    blocks: &[Block],
    scope: usize,
    file: &str,
    start: u32,
    end: u32,
) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (index, block) in blocks.iter().enumerate() {
        if block.scope != scope || block.file != file {
            continue;
        }
        if block.start <= start && end <= block.end {
            let better = match best {
                None => true,
                Some(current) => {
                    (block.end - block.start) < (blocks[current].end - blocks[current].start)
                }
            };
            if better {
                best = Some(index);
            }
        }
    }
    best
}

/// The block that contains `blocks[index]`, or `None` when the function body
/// is the parent.
fn parent_block(blocks: &[Block], index: usize) -> Option<usize> {
    let child = &blocks[index];
    let mut best: Option<usize> = None;
    for (candidate, block) in blocks.iter().enumerate() {
        if candidate == index
            || block.scope != child.scope
            || block.file != child.file
            || block.start > child.start
            || child.end > block.end
        {
            continue;
        }
        let larger = block.end - block.start > child.end - child.start;
        if !larger {
            continue;
        }
        let better = match best {
            None => true,
            Some(current) => {
                (block.end - block.start) < (blocks[current].end - blocks[current].start)
            }
        };
        if better {
            best = Some(candidate);
        }
    }
    best
}

/// Collect one location per named user function in source order.
///
/// For an `async fn` the location names the lowered `poll` function, which is
/// where the body lives; the factory that returns the task frame has no source
/// statements of its own. The debugger therefore stops in the body at the
/// async function's source line when the task is polled.
pub fn functions(hir: &HirProgram, sources: &SourceMap) -> Vec<FunctionLocation> {
    let mut out = Vec::new();
    for item in &hir.items {
        let HirItem::Fn(function) = item else {
            continue;
        };
        if function.name.contains('$') {
            continue;
        }
        let symbol = body_symbol(function);
        let line = sources.locate(function.file, function.span).line;
        out.push(FunctionLocation {
            symbol,
            name: function.name.clone(),
            file: sources.path(function.file).to_string(),
            line,
        });
    }
    out
}

/// One user type and the facts LLVM needs to describe it.
#[derive(Clone, Debug)]
struct TypeEntry {
    name: String,
    line: u32,
    tag: &'static str,
    layout: yashy_hir::layout::Layout,
    /// Struct fields: name, field type and byte offset.
    fields: Vec<(String, TypeKind, usize)>,
    /// Enum variant names, in declaration order.
    variants: Vec<String>,
}

/// Collect declared types and their layout from the same tables the checker
/// and `inspect --layout` use. A type the checker cannot lay out is skipped:
/// emitting a wrong size would be worse than leaving it out of the table.
fn types(hir: &HirProgram, sources: &SourceMap, tags: &[String]) -> Vec<TypeEntry> {
    let decls = crate::layout::HirDecls::from_program(hir);
    let entry = FileId::from_index(0);
    let mut out = Vec::new();
    for item in &hir.items {
        let (name, tag, fields, variants) = match item {
            HirItem::Struct(structure) => (
                structure.name.clone(),
                "DW_TAG_structure_type",
                structure
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.kind.clone()))
                    .collect::<Vec<_>>(),
                Vec::new(),
            ),
            HirItem::Enum(enumeration) => (
                enumeration.name.clone(),
                "DW_TAG_enumeration_type",
                Vec::new(),
                enumeration
                    .variants
                    .iter()
                    .map(|variant| variant.name.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => continue,
        };
        let Some(layout) = crate::layout::find(&name, &decls) else {
            continue;
        };
        let offsets = layout.offsets.clone();
        let fields = fields
            .into_iter()
            .enumerate()
            .map(|(index, (field, kind))| (field, kind, offsets.get(index).copied().unwrap_or(0)))
            .collect();
        out.push(TypeEntry {
            name,
            line: sources.locate(entry, item_span(item)).line,
            tag,
            layout,
            fields,
            variants,
        });
    }
    // `Option` and `Result` are compiler-known: no HIR item declares them, so
    // the walk above never sees them. Synthesize a structure entry for each
    // spelling a variable actually uses, with the field names the shipped LLDB
    // summaries read. The tag is the first byte and the payloads follow, which
    // is the layout the backend emits.
    let span = Span::new(0, 0);
    let mut seen = BTreeSet::new();
    for tag in tags {
        let (name, field_names): (&str, &[&str]) = if tag.starts_with("option<") {
            ("Option", &["tag", "payload"])
        } else if tag.starts_with("result<") {
            ("Result", &["tag", "ok", "err"])
        } else {
            continue;
        };
        if !seen.insert((name.to_string(), tag.clone())) || out.iter().any(|e| e.name == name) {
            continue;
        }
        let Some(kind) = kind_from_tag(tag, span) else {
            continue;
        };
        let Some(layout) = yashy_hir::layout::layout(&kind, &decls) else {
            continue;
        };
        let field_kinds: Vec<TypeKind> = match &kind {
            TypeKind::Option(inner) => vec![TypeKind::CInt(8), inner.kind.clone()],
            TypeKind::Result(ok, err) => {
                vec![TypeKind::CInt(8), ok.kind.clone(), err.kind.clone()]
            }
            _ => Vec::new(),
        };
        let fields = field_names
            .iter()
            .zip(field_kinds)
            .enumerate()
            .map(|(index, (field, kind))| {
                (
                    field.to_string(),
                    kind,
                    layout.offsets.get(index).copied().unwrap_or(0),
                )
            })
            .collect();
        out.push(TypeEntry {
            name: name.to_string(),
            line: 0,
            tag: "DW_TAG_structure_type",
            layout,
            fields,
            variants: Vec::new(),
        });
    }
    out
}

/// The inverse of `yashy_codegen::module::debug_tag`, for synthetic DI entries.
fn kind_from_tag(tag: &str, span: Span) -> Option<TypeKind> {
    if let Some(inner) = tag
        .strip_prefix("option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return Some(TypeKind::Option(Box::new(Type {
            kind: kind_from_tag(inner, span)?,
            span,
        })));
    }
    if let Some(inner) = tag
        .strip_prefix("result<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let (ok, err) = inner.split_once(',')?;
        return Some(TypeKind::Result(
            Box::new(Type {
                kind: kind_from_tag(ok, span)?,
                span,
            }),
            Box::new(Type {
                kind: kind_from_tag(err, span)?,
                span,
            }),
        ));
    }
    let kind = match tag {
        "int" => TypeKind::I64,
        "float" => TypeKind::F64,
        "bool" => TypeKind::Bool,
        "char" => TypeKind::Char,
        "string" => TypeKind::Str,
        "ptr" => TypeKind::Ptr(Box::new(Type {
            kind: TypeKind::Unit,
            span,
        })),
        other => {
            if let Some(name) = other.strip_prefix("struct ") {
                TypeKind::Struct(name.to_string())
            } else if let Some(name) = other.strip_prefix("enum ") {
                TypeKind::Enum(name.to_string())
            } else {
                return None;
            }
        }
    };
    Some(kind)
}

fn item_span(item: &HirItem) -> Span {
    match item {
        HirItem::Struct(structure) => structure.span,
        HirItem::Enum(enumeration) => enumeration.span,
        HirItem::Fn(function) => function.span,
        HirItem::Import(import) => import.span,
        HirItem::Extern(block) => block.span,
    }
}

/// The five basic types the member walk maps field kinds onto.
const BASIC_TYPES: [(&str, &str, &str); 5] = [
    ("int", "64", "DW_ATE_signed"),
    ("float", "64", "DW_ATE_float"),
    ("bool", "8", "DW_ATE_boolean"),
    ("char", "8", "DW_ATE_unsigned_char"),
    ("ptr", "64", "DW_ATE_address"),
];

/// Escape a path or name for an LLVM metadata string.
fn llvm_string(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\0A"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn parse_marker(line: &str, sources: &SourceMap, scope: usize) -> Option<Point> {
    let rest = line.trim().strip_prefix("; yashy-loc:")?;
    let mut fields = rest.split(':');
    let file = fields.next()?.parse::<u32>().ok()?;
    let start = fields.next()?.parse::<u32>().ok()?;
    let end = fields.next()?.parse::<u32>().ok()?;
    let id = FileId::from_index(file);
    let at = sources.locate(id, Span::new(start, end));
    Some(Point {
        file: sources.path(id).to_string(),
        line: at.line,
        column: at.col,
        start,
        end,
        scope,
        block: None,
    })
}

/// Whether an IR line can carry an instruction-level `!dbg` attachment.
fn attachable(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with(';')
        && !trimmed.starts_with('}')
        && !trimmed.ends_with(':')
        && !trimmed.starts_with("define ")
        && !trimmed.starts_with("declare ")
}

/// A variable marker: one parameter or one `let` binding in a function.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Variable {
    scope: usize,
    arg: Option<usize>,
    name: String,
    slot: String,
    tag: String,
    /// The source line of the binding, taken from the statement marker that
    /// preceded it. `0` only when nothing preceded the marker (a parameter).
    line: u32,
    /// The lexical block the binding is declared in, or `None` for the
    /// function's own scope (every parameter, and a binding at the top level).
    block: Option<usize>,
    /// The statement marker the binding followed, so the `llvm.dbg.declare`
    /// call carries that statement's own location rather than the function's
    /// first line.
    decl: Option<Point>,
}

fn parse_variable(line: &str) -> Option<(Option<usize>, String, String, String)> {
    let rest = line.trim().strip_prefix("; yashy-var:")?;
    let (kind, mut rest) = rest.split_once(':')?;
    let arg = match kind {
        "local" => None,
        // The emitter writes `arg:<index>`, so the index is the next field.
        "arg" => {
            let (index, remaining) = rest.split_once(':')?;
            rest = remaining;
            Some(index.parse::<usize>().ok()?)
        }
        _ => return None,
    };
    let (name, rest) = rest.split_once(':')?;
    let (slot, tag) = rest.split_once(':')?;
    Some((arg, name.to_string(), slot.to_string(), tag.to_string()))
}

/// Attach function-level and per-statement debug metadata to a whole module.
pub fn annotate(ir: &str, hir: &HirProgram, sources: &SourceMap) -> String {
    let functions = functions(hir, sources);
    let mut scopes = BTreeMap::new();
    // Simpler and stable: one DISubprogram per function entry, in order.
    for (index, function) in functions.iter().enumerate() {
        scopes.insert(function.symbol.clone(), index);
    }

    // Pass one: collect the unique files, source points and variables so
    // metadata ids can be assigned before any line is rewritten. Every marker
    // belongs to the function whose `define` precedes it; that is what gives a
    // location the right scope and keeps a trailing marker from attaching to
    // the next function.
    let blocks = blocks(hir, sources, &functions);
    let mut points: Vec<Point> = Vec::new();
    let mut variables: Vec<Variable> = Vec::new();
    let mut scope: Option<usize> = None;
    let mut last_point: Option<Point> = None;
    for line in ir.lines() {
        if let Some(rest) = line.strip_prefix("define ") {
            scope = definition_symbol(rest).and_then(|symbol| scopes.get(symbol).copied());
            last_point = None;
        }
        // A synthesized function -- a lambda body, a task resume, a thread
        // entry -- has no `DISubprogram`, so its lines are left without source
        // locations rather than borrowing the previous function's.
        let Some(enclosing) = scope else {
            continue;
        };
        if let Some(mut point) = parse_marker(line, sources, enclosing) {
            point.block = innermost_block(&blocks, enclosing, &point.file, point.start, point.end);
            if !points.contains(&point) {
                points.push(point.clone());
            }
            last_point = Some(point);
        }
        if let Some((arg, name, slot, tag)) = parse_variable(line) {
            let block = last_point.as_ref().and_then(|point| point.block);
            let declared_line = last_point
                .as_ref()
                .map(|point| point.line)
                .unwrap_or_else(|| functions[enclosing].line);
            let variable = Variable {
                scope: enclosing,
                arg,
                name,
                slot,
                tag,
                line: declared_line,
                block,
                decl: last_point.clone(),
            };
            if !variables.contains(&variable) {
                variables.push(variable);
            }
        }
    }
    let mut files: Vec<String> = functions.iter().map(|entry| entry.file.clone()).collect();
    for point in &points {
        if !files.contains(&point.file) {
            files.push(point.file.clone());
        }
    }

    // Metadata ids: 0 compile unit, 1 module flags, 2 subroutine type,
    // 3 empty type list, then DIFile, DISubprogram and DILocation blocks.
    let file_id = |path: &str| 4 + files.iter().position(|entry| entry == path).unwrap();
    let subprogram_base = 4 + files.len();
    let location_base = subprogram_base + functions.len();

    // The type table continues after the locations and the lexical blocks, so
    // adding blocks does not renumber any metadata an existing debug build
    // already emitted past them.
    let decls = crate::layout::HirDecls::from_program(hir);
    let tags: Vec<String> = variables
        .iter()
        .map(|variable| variable.tag.clone())
        .collect();
    let type_entries = types(hir, sources, &tags);
    let block_base = location_base + points.len();
    let basic_base = block_base + blocks.len();
    let element_base = basic_base + BASIC_TYPES.len();
    let element_count: usize = type_entries
        .iter()
        .map(|entry| entry.fields.len() + entry.variants.len())
        .sum();
    let composite_base = element_base + element_count;
    let composites: BTreeMap<String, usize> = type_entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.name.clone(), composite_base + index))
        .collect();

    // `string` is an `i8*` to the runtime's `yashy_str`, so its debug type is
    // the struct behind the pointer and the variable's expression dereferences.
    // That is what lets the shipped LLDB summaries read `data` instead of
    // printing an address.
    let string_pointer = composite_base + type_entries.len();
    let string_member = string_pointer + 1;
    let char_pointer = string_member + 4;
    let string_type = char_pointer + 1;
    let deref_expression = string_type + 1;
    let empty_expression = deref_expression + 1;
    // Every `llvm.dbg.declare` needs a location: LLVM's record form carries one
    // and a record with a null location is discarded, taking the whole
    // function's variable list with it. A function's own first line is the
    // honest answer for a binding whose exact span is not tracked.
    let scope_location = empty_expression + 1;
    let var_base = scope_location + functions.len();

    let resolve = |tag: &str| -> Option<usize> {
        match tag {
            "int" => Some(basic_base),
            "float" => Some(basic_base + 1),
            "bool" => Some(basic_base + 2),
            "char" => Some(basic_base + 3),
            "ptr" => Some(basic_base + 4),
            "string" => Some(string_type),
            other if other.starts_with("option<") => composites.get("Option").copied(),
            other if other.starts_with("result<") => composites.get("Result").copied(),
            other => other
                .strip_prefix("struct ")
                .or_else(|| other.strip_prefix("enum "))
                .and_then(|name| composites.get(name).copied()),
        }
    };
    // A variable whose type has no DI type is dropped rather than described
    // wrongly: an absent variable is a missing convenience, a mistyped one
    // makes every read in the debugger a lie.
    variables.retain(|variable| resolve(&variable.tag).is_some());

    // Pass two: drop markers and attach !dbg to the next instruction.
    let mut out: Vec<String> = Vec::new();
    let mut pending: Option<Point> = None;
    let mut current: Option<usize> = None;
    for line in ir.lines() {
        if line.trim_start().starts_with("; yashy-loc:") {
            pending = current.and_then(|scope| parse_marker(line, sources, scope));
            continue;
        }
        if let Some((_, name, slot, _)) = parse_variable(line) {
            let declared = current.and_then(|index| {
                variables.iter().position(|variable| {
                    variable.scope == index && variable.name == name && variable.slot == slot
                })
            });
            if let Some(index) = declared {
                let variable = &variables[index];
                let expression = if variable.tag == "string" {
                    deref_expression
                } else {
                    empty_expression
                };
                let location = variable
                    .decl
                    .as_ref()
                    .map(|point| point_id(point, &points, location_base))
                    .unwrap_or(scope_location + variable.scope);
                out.push(format!(
                    "call void @llvm.dbg.declare(metadata ptr {}, metadata !{}, metadata !{}), !dbg !{}",
                    variable.slot,
                    var_base + index,
                    expression,
                    location
                ));
            }
            continue;
        }
        let mut emitted = line.to_string();
        if let Some(rest) = line.strip_prefix("define ") {
            pending = None;
            current = definition_symbol(rest).and_then(|symbol| scopes.get(symbol).copied());
            if let Some(index) = current {
                if let Some(head) = emitted.strip_suffix(" {") {
                    emitted = format!("{head} !dbg !{} {{", subprogram_base + index);
                }
            }
        } else if attachable(line) {
            if let Some(point) = pending.take() {
                emitted.push_str(&format!(
                    ", !dbg !{}",
                    point_id(&point, &points, location_base)
                ));
            }
        }
        out.push(emitted);
    }

    out.push(String::new());
    out.push("declare void @llvm.dbg.declare(metadata, metadata, metadata)".to_string());
    out.push("!llvm.dbg.cu = !{!0}".to_string());
    out.push("!llvm.module.flags = !{!1}".to_string());
    let retained = composites
        .values()
        .map(|id| format!("!{id}"))
        .collect::<Vec<_>>()
        .join(", ");
    out.push(format!(
        "!0 = distinct !DICompileUnit(language: DW_LANG_C, file: !4, producer: \"yashyc\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug, retainedTypes: !{{{retained}}})"
    ));
    out.push("!1 = !{i32 2, !\"Debug Info Version\", i32 3}".to_string());
    out.push("!2 = !DISubroutineType(types: !3)".to_string());
    out.push("!3 = !{}".to_string());
    for (index, path) in files.iter().enumerate() {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        out.push(format!(
            "!{} = !DIFile(filename: {}, directory: \".\")",
            4 + index,
            llvm_string(&name)
        ));
    }
    for (index, function) in functions.iter().enumerate() {
        out.push(format!(
            "!{} = distinct !DISubprogram(name: {}, scope: !{}, file: !{}, line: {}, type: !2, scopeLine: {}, spFlags: DISPFlagDefinition, unit: !0)",
            subprogram_base + index,
            llvm_string(&function.name),
            file_id(&function.file),
            file_id(&function.file),
            function.line,
            function.line
        ));
    }
    for (index, point) in points.iter().enumerate() {
        let scope = point
            .block
            .map(|block| block_base + block)
            .unwrap_or(subprogram_base + point.scope);
        out.push(format!(
            "!{} = !DILocation(line: {}, column: {}, scope: !{})",
            location_base + index,
            point.line,
            point.column,
            scope
        ));
    }
    for (index, block) in blocks.iter().enumerate() {
        let parent = parent_block(&blocks, index)
            .map(|parent| block_base + parent)
            .unwrap_or(subprogram_base + block.scope);
        out.push(format!(
            "!{} = distinct !DILexicalBlock(scope: !{}, file: !{}, line: {}, column: {})",
            block_base + index,
            parent,
            file_id(&block.file),
            block.line,
            block.column
        ));
    }
    for (index, (name, size, encoding)) in BASIC_TYPES.iter().enumerate() {
        out.push(format!(
            "!{} = !DIBasicType(name: {}, size: {}, encoding: {})",
            basic_base + index,
            llvm_string(name),
            size,
            encoding
        ));
    }

    // One member or enumerator per declared field/variant. Field offsets and
    // sizes come from the checker's layout walk, in bits as LLVM requires.
    let mut element = element_base;
    let mut composite_elements: Vec<Vec<usize>> = Vec::new();
    for entry in &type_entries {
        let mut ids = Vec::new();
        for (field, kind, offset) in &entry.fields {
            let base = match kind {
                TypeKind::I64 | TypeKind::CInt(64) => basic_base,
                TypeKind::F64 | TypeKind::CFloat(64) => basic_base + 1,
                TypeKind::Bool => basic_base + 2,
                TypeKind::Char | TypeKind::CUInt(8) => basic_base + 3,
                TypeKind::Struct(name) | TypeKind::Enum(name) => {
                    *composites.get(name).unwrap_or(&(basic_base + 4))
                }
                _ => basic_base + 4,
            };
            let size = yashy_hir::layout::layout(kind, &decls)
                .map(|layout| layout.size * 8)
                .unwrap_or(64);
            out.push(format!(
                "!{} = !DIDerivedType(tag: DW_TAG_member, name: {}, scope: !{}, file: !{}, baseType: !{}, size: {}, offset: {})",
                element,
                llvm_string(field),
                composites[&entry.name],
                4,
                base,
                size,
                offset * 8
            ));
            ids.push(element);
            element += 1;
        }
        for (index, variant) in entry.variants.iter().enumerate() {
            out.push(format!(
                "!{} = !DIEnumerator(name: {}, value: {})",
                element,
                llvm_string(variant),
                index
            ));
            ids.push(element);
            element += 1;
        }
        composite_elements.push(ids);
    }
    for (index, entry) in type_entries.iter().enumerate() {
        let elements = composite_elements[index]
            .iter()
            .map(|id| format!("!{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        let extra = if entry.tag == "DW_TAG_enumeration_type" {
            format!(", baseType: !{}", basic_base)
        } else {
            String::new()
        };
        out.push(format!(
            "!{} = distinct !DICompositeType(tag: {}, name: {}, scope: !4, file: !4, line: {}, size: {}, align: {}, elements: !{{{}}}{})",
            composite_base + index,
            entry.tag,
            llvm_string(&entry.name),
            entry.line,
            entry.layout.size * 8,
            entry.layout.align * 8,
            elements,
            extra
        ));
    }

    // The runtime's string header. `refcount`, `len` and `cap` are `long long`;
    // `data` is the byte pointer the summaries read. The layout is the one
    // `runtime.c` documents, and it is only reachable through a `string`
    // variable's pointer, which is why the type is a pointer to this struct.
    out.push(format!(
        "!{} = !DIDerivedType(tag: DW_TAG_pointer_type, name: \"yashy_str *\", baseType: !{}, size: 64)",
        string_pointer, string_type
    ));
    for (index, name) in ["refcount", "len", "cap", "data"].iter().enumerate() {
        // `data` is `char *`: a pointer to the first byte, not the struct.
        let base = if index == 3 {
            format!("!{char_pointer}")
        } else {
            format!("!{basic_base}")
        };
        out.push(format!(
            "!{} = !DIDerivedType(tag: DW_TAG_member, name: {}, scope: !{}, file: !4, baseType: {}, size: 64, offset: {})",
            string_member + index,
            llvm_string(name),
            string_type,
            base,
            index * 64
        ));
    }
    // The pointer `data` points at, named so its spelling is not lost.
    out.push(format!(
        "!{char_pointer} = !DIDerivedType(tag: DW_TAG_pointer_type, name: \"char *\", baseType: !{}, size: 64)",
        basic_base + 3
    ));
    out.push(format!(
        "!{} = distinct !DICompositeType(tag: DW_TAG_structure_type, name: \"yashy_str\", scope: !4, file: !4, size: 256, align: 64, elements: !{{!{}, !{}, !{}, !{}}})",
        string_type,
        string_member,
        string_member + 1,
        string_member + 2,
        string_member + 3
    ));
    out.push(format!("!{deref_expression} = !DIExpression(DW_OP_deref)"));
    out.push(format!("!{empty_expression} = !DIExpression()"));
    for (index, function) in functions.iter().enumerate() {
        out.push(format!(
            "!{} = !DILocation(line: {}, column: 1, scope: !{})",
            scope_location + index,
            function.line,
            subprogram_base + index
        ));
    }
    for (index, variable) in variables.iter().enumerate() {
        let ty = resolve(&variable.tag).unwrap_or(basic_base + 4);
        let arg = variable
            .arg
            .map(|index| format!("arg: {index}, "))
            .unwrap_or_default();
        let scope = variable
            .block
            .map(|block| block_base + block)
            .unwrap_or(subprogram_base + variable.scope);
        out.push(format!(
            "!{} = !DILocalVariable(name: {}, {}scope: !{}, file: !4, line: {}, type: !{})",
            var_base + index,
            llvm_string(&variable.name),
            arg,
            scope,
            variable.line,
            ty
        ));
    }
    out.push(String::new());
    out.join("\n")
}

/// The symbol a `define` line defines, without its `@` or signature.
fn definition_symbol(rest: &str) -> Option<&str> {
    rest.split_once('@')
        .map(|(_, rest)| rest.split(['(', ' ']).next().unwrap_or(""))
}

fn point_id(point: &Point, points: &[Point], location_base: usize) -> usize {
    location_base + points.iter().position(|entry| entry == point).unwrap_or(0)
}

/// The function-only form used by the unit tests: no statements, no markers.
pub fn annotate_basic(ir: &str, functions: &[FunctionLocation]) -> String {
    // The basic form is kept simple: function attachments without locations.
    let mut out: Vec<String> = ir.lines().map(str::to_string).collect();
    for line in &mut out {
        if let Some(rest) = line.strip_prefix("define ") {
            let symbol = rest
                .split_once('@')
                .map(|(_, rest)| rest.split(['(', ' ']).next().unwrap_or(""))
                .unwrap_or("");
            if let Some(index) = functions.iter().position(|entry| entry.symbol == symbol) {
                if let Some(head) = line.strip_suffix(" {") {
                    *line = format!("{head} !dbg !{} {{", 4 + functions.len() + index);
                }
            }
        }
    }
    out.push(String::new());
    out.push("!llvm.dbg.cu = !{!0}".to_string());
    out.push("!1 = !{i32 2, !\"Debug Info Version\", i32 3}".to_string());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use yashy_hir::hir::{HirBlock, HirFn};
    use yashy_hir::ty::Type;

    fn location(symbol: &str, name: &str, line: u32) -> FunctionLocation {
        FunctionLocation {
            symbol: symbol.to_string(),
            name: name.to_string(),
            file: "main.ya".to_string(),
            line,
        }
    }

    #[test]
    fn a_function_definition_gets_a_subprogram() {
        let ir = "define i32 @main() {\nentry:\n  ret i32 0\n}\n";
        let annotated = annotate_basic(ir, &[location("main", "main", 3)]);
        assert!(
            annotated.contains("define i32 @main() !dbg !5 {"),
            "{annotated}"
        );
        assert!(annotated.contains("!llvm.dbg.cu = !{!0}"), "{annotated}");
    }

    #[test]
    fn declared_types_get_a_table_in_the_compile_unit() {
        use yashy_hir::hir::{HirFieldDef, HirProgram, HirStruct};
        use yashy_hir::ty::{Type, TypeKind};

        let mut sources = SourceMap::new();
        sources.add_file(
            "main.ya".to_string(),
            "struct Point { x: int }\n".to_string(),
        );
        let span = Span::new(0, 22);
        let hir = HirProgram {
            items: vec![HirItem::Struct(HirStruct {
                name: "Point".to_string(),
                derives: Vec::new(),
                fields: vec![HirFieldDef {
                    name: "x".to_string(),
                    ty: Type {
                        kind: TypeKind::I64,
                        span,
                    },
                    span,
                }],
                repr: None,
                span,
            })],
            instantiations: 0,
            uses_threads: false,
            uses_async: false,
        };
        let annotated = annotate(
            "define i32 @main() {\nentry:\n  ret i32 0\n}\n",
            &hir,
            &sources,
        );
        assert!(annotated.contains("DW_TAG_structure_type"), "{annotated}");
        assert!(annotated.contains("name: \"Point\""), "{annotated}");
        assert!(annotated.contains("DW_TAG_member"), "{annotated}");
        assert!(annotated.contains("retainedTypes: !{!"), "{annotated}");
    }

    #[test]
    fn markers_are_removed_and_locations_are_parsed() {
        let mut sources = SourceMap::new();
        sources.add_file(
            "main.ya".to_string(),
            "// line one\nfn main() {\n  let x = 1;\n}\n".to_string(),
        );
        let point = parse_marker("; yashy-loc:0:13:24", &sources, 2).unwrap();
        assert_eq!(point.line, 2);
        assert_eq!(point.scope, 2);
        assert!(parse_marker("  ret i32 0", &sources, 0).is_none());
    }

    #[test]
    fn a_variable_marker_names_its_kind_name_slot_and_type() {
        assert_eq!(
            parse_variable("; yashy-var:arg:2:limit:%slot7:int"),
            Some((
                Some(2),
                "limit".to_string(),
                "%slot7".to_string(),
                "int".to_string()
            ))
        );
        assert_eq!(
            parse_variable("; yashy-var:local:p:%slot3:struct Point"),
            Some((
                None,
                "p".to_string(),
                "%slot3".to_string(),
                "struct Point".to_string()
            ))
        );
        assert!(parse_variable("; yashy-loc:0:1:2").is_none());
        assert!(parse_variable("; yashy-var:arg:x:%slot0:int").is_none());
    }

    fn program(functions: Vec<HirFn>) -> HirProgram {
        HirProgram {
            items: functions.into_iter().map(HirItem::Fn).collect(),
            instantiations: 0,
            uses_threads: false,
            uses_async: false,
        }
    }

    fn function(name: &str, span: Span) -> HirFn {
        HirFn {
            name: name.to_string(),
            params: Vec::new(),
            ret: Type {
                kind: TypeKind::I64,
                span,
            },
            body: HirBlock {
                stmts: Vec::new(),
                span,
                arena: false,
            },
            effect: yashy_common::Effect::PURE,
            declared: yashy_common::DeclaredEffect::Inferred,
            is_async: false,
            requires: Vec::new(),
            ensures: Vec::new(),
            captures: Vec::new(),
            docs: Vec::new(),
            file: FileId::from_index(0),
            span,
        }
    }

    #[test]
    fn a_parameter_and_a_local_become_debug_variables() {
        let mut sources = SourceMap::new();
        sources.add_file(
            "main.ya".to_string(),
            "fn work(int n) -> int {\n  let s = n;\n  s\n}\n".to_string(),
        );
        let hir = program(vec![function("work", Span::new(0, 43))]);
        let ir = "define i64 @yashy_fn_work(i64 %arg0) {\nentry:\n  %slot0 = alloca i64\n; yashy-var:arg:1:n:%slot0:int\n  store i64 %arg0, ptr %slot0\n  %slot1 = alloca i64\n  store i64 %arg0, ptr %slot1\n; yashy-var:local:s:%slot1:int\n  ret i64 %arg0\n}\n";
        let annotated = annotate(ir, &hir, &sources);
        assert!(
            annotated.contains("call void @llvm.dbg.declare(metadata ptr %slot0, metadata !"),
            "{annotated}"
        );
        assert!(
            annotated.contains("call void @llvm.dbg.declare(metadata ptr %slot1, metadata !"),
            "{annotated}"
        );
        assert!(
            annotated.contains("= !DILocalVariable(name: \"n\", arg: 1,"),
            "{annotated}"
        );
        assert!(
            annotated.contains("= !DILocalVariable(name: \"s\", scope: !"),
            "{annotated}"
        );
        assert!(!annotated.contains("yashy-var"), "{annotated}");
    }

    #[test]
    fn a_string_variable_reads_the_runtime_header() {
        let mut sources = SourceMap::new();
        sources.add_file(
            "main.ya".to_string(),
            "fn show() {\n  let name = \"ada\";\n}\n".to_string(),
        );
        let hir = program(vec![function("show", Span::new(0, 37))]);
        let ir = "define void @yashy_fn_show() {\nentry:\n  %slot0 = alloca i8*\n  store i8* null, ptr %slot0\n; yashy-var:local:name:%slot0:string\n  ret void\n}\n";
        let annotated = annotate(ir, &hir, &sources);
        assert!(
            annotated.contains("!DICompositeType(tag: DW_TAG_structure_type, name: \"yashy_str\""),
            "{annotated}"
        );
        assert!(
            annotated.contains("name: \"data\""),
            "the printer reads `data`: {annotated}"
        );
        let deref_line = annotated
            .lines()
            .find(|line| line.contains("DW_OP_deref"))
            .expect("a string variable dereferences its header");
        let id = deref_line
            .trim_start_matches('!')
            .split(' ')
            .next()
            .expect("a metadata id");
        let declare = annotated
            .lines()
            .find(|line| line.contains("llvm.dbg.declare(metadata ptr %slot0"))
            .expect("the declare call");
        assert!(declare.contains(&format!("metadata !{id})")), "{annotated}");
    }

    #[test]
    fn option_and_result_variables_get_the_printer_structs() {
        let mut sources = SourceMap::new();
        sources.add_file(
            "main.ya".to_string(),
            "fn show() {\n  let a = Some(1);\n  let b = Ok(2);\n}\n".to_string(),
        );
        let hir = program(vec![function("show", Span::new(0, 41))]);
        let ir = "define void @yashy_fn_show() {\nentry:\n  %slot0 = alloca { i8*, i64 }\n; yashy-var:local:a:%slot0:option<int>\n  %slot1 = alloca { i8*, i64, i64 }\n; yashy-var:local:b:%slot1:result<int,string>\n  ret void\n}\n";
        let annotated = annotate(ir, &hir, &sources);
        assert!(
            annotated.contains("name: \"Option\""),
            "the printer registers `Option`: {annotated}"
        );
        assert!(
            annotated.contains("name: \"Result\""),
            "the printer registers `Result`: {annotated}"
        );
        for field in ["tag", "payload", "ok", "err"] {
            assert!(
                annotated.contains(&format!("name: \"{field}\"")),
                "the printer reads `{field}`: {annotated}"
            );
        }
        assert!(
            annotated.contains("llvm.dbg.declare(metadata ptr %slot0")
                && annotated.contains("llvm.dbg.declare(metadata ptr %slot1"),
            "both variables have a declared type: {annotated}"
        );
    }

    #[test]
    fn each_functions_locations_are_scoped_to_it() {
        let mut sources = SourceMap::new();
        sources.add_file(
            "main.ya".to_string(),
            "fn one() {\n  1\n}\nfn two() {\n  2\n}\n".to_string(),
        );
        let hir = program(vec![
            function("one", Span::new(0, 18)),
            function("two", Span::new(19, 38)),
        ]);
        let ir = "define void @yashy_fn_one() {\nentry:\n; yashy-loc:0:13:14\n  ret void\n}\ndefine void @yashy_fn_two() {\nentry:\n; yashy-loc:0:30:31\n  ret void\n}\n";
        let annotated = annotate(ir, &hir, &sources);
        // One file, two functions: subprograms are !6 and !7, locations !8.. .
        assert!(
            annotated.contains("= !DILocation(line: 2, column: 3, scope: !6)"),
            "{annotated}"
        );
        assert!(
            annotated.contains("= !DILocation(line: 5, column: 3, scope: !7)"),
            "{annotated}"
        );
    }

    #[test]
    fn a_variable_of_an_unmapped_type_is_left_out() {
        let mut sources = SourceMap::new();
        sources.add_file("main.ya".to_string(), "fn list() {\n}\n".to_string());
        let hir = program(vec![function("list", Span::new(0, 13))]);
        let ir = "define void @yashy_fn_list() {\nentry:\n  %slot0 = alloca [3 x i64]\n; yashy-var:local:xs:%slot0:tuple\n  ret void\n}\n";
        let annotated = annotate(ir, &hir, &sources);
        assert!(
            !annotated.contains("call void @llvm.dbg.declare"),
            "{annotated}"
        );
        assert!(!annotated.contains("yashy-var"), "{annotated}");
    }
}
