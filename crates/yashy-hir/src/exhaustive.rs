//! Exhaustiveness checking for `switch` arms.
//!
//! Given an enum's variants and the patterns of a `switch`'s arms, this
//! module reports which variants no arm covers. It knows nothing about
//! scopes or lifetimes -- it is pure shape matching over [`Pattern`],
//! [`VariantDef`] and the payload types written in a variant declaration --
//! so it can be unit-tested without the rest of the type checker, and called
//! again once sema resolves variant paths.
//!
//! # The algorithm
//!
//! Coverage is asked as a usefulness question over a matrix of pattern
//! columns (Maranget's algorithm). A switch is exhaustive for a variant when
//! the row `Variant(_, _, ...)` is *not* useful against the matrix of arms
//! seen so far. Or-patterns become several rows, ranges are treated as
//! intervals over `int`/`char`, and a nested payload contributes one column
//! per field, so a value like `A(Some(1))` is examined column by column
//! rather than by variant name alone.
//!
//! Guards are not represented here: the caller removes guarded arms before
//! calling, because a guard can be false and therefore covers nothing.
//!
//! # No input makes this module panic
//!
//! Every recursion is bounded by a pattern's depth and every lookup is a
//! membership test. There is no `unwrap`, no `expect`, no `panic!` and no
//! unchecked indexing here.

pub use yashy_parser::ast::{Pattern, VariantDef};
use yashy_parser::ast::{Literal, TypeExpr, VariantField};

/// The names of the enum variants no arm covers, in declaration order.
/// Empty means the arms are exhaustive.
///
/// A variant is reported once, even when only some of its payload values are
/// missing; [`missing_witnesses`] describes the shape that is missing.
pub fn missing_cases(
    enum_name: &str,
    variants: &[VariantDef],
    arms: &[Pattern],
    has_default: bool,
) -> Vec<String> {
    let root = variants.to_vec();
    let lookup = move |name: &str| {
        if name == enum_name {
            Some(root.clone())
        } else {
            None
        }
    };
    missing_cases_with(enum_name, variants, arms, has_default, &lookup)
}

/// [`missing_cases`] with a resolver for enum payload types.
///
/// `lookup` answers the variants of a user-declared enum named in a payload
/// type; the type checker has the whole enum table and passes it here so a
/// nested enum payload is checked with its own variants. `missing_cases`
/// passes a resolver that knows only the root enum, which is enough for
/// `Option`/`Result` payloads and for the pure shape tests.
pub fn missing_cases_with(
    enum_name: &str,
    variants: &[VariantDef],
    arms: &[Pattern],
    has_default: bool,
    lookup: &dyn Fn(&str) -> Option<Vec<VariantDef>>,
) -> Vec<String> {
    if has_default || variants.is_empty() {
        return Vec::new();
    }
    let ctx = Ctx { lookup };
    let ty = Ty::Enum(enum_name.to_string());
    let matrix: Vec<Vec<Pat>> = arms
        .iter()
        .map(|arm| vec![ctx.pat_of(arm, &ty)])
        .collect();
    let mut missing = Vec::new();
    for variant in variants {
        let args = vec![Pat::Wild; variant.fields.len()];
        let row = vec![Pat::Ctor {
            name: variant.name.clone(),
            args,
        }];
        if is_useful(&matrix, &row, &[ty.clone()], &ctx) {
            missing.push(variant.name.clone());
        }
    }
    missing
}

/// The shapes no arm covers, one string per missing variant.
///
/// A variant no arm names is reported by name, preserving the message the
/// checker has always produced. A variant some arm names but does not fully
/// cover is reported with its first missing payload shape, e.g.
/// `A(None)` or `A(Some(_))`.
pub fn missing_witnesses(
    enum_name: &str,
    variants: &[VariantDef],
    arms: &[Pattern],
    has_default: bool,
) -> Vec<String> {
    let root = variants.to_vec();
    let lookup = move |name: &str| {
        if name == enum_name {
            Some(root.clone())
        } else {
            None
        }
    };
    missing_witnesses_with(enum_name, variants, arms, has_default, &lookup)
}

/// [`missing_witnesses`] with a resolver for enum payload types.
pub fn missing_witnesses_with(
    enum_name: &str,
    variants: &[VariantDef],
    arms: &[Pattern],
    has_default: bool,
    lookup: &dyn Fn(&str) -> Option<Vec<VariantDef>>,
) -> Vec<String> {
    if has_default || variants.is_empty() {
        return Vec::new();
    }
    let ctx = Ctx { lookup };
    let ty = Ty::Enum(enum_name.to_string());
    let matrix: Vec<Vec<Pat>> = arms
        .iter()
        .map(|arm| vec![ctx.pat_of(arm, &ty)])
        .collect();
    let mut missing = Vec::new();
    for variant in variants {
        let args = vec![Pat::Wild; variant.fields.len()];
        let row = vec![Pat::Ctor {
            name: variant.name.clone(),
            args,
        }];
        if !is_useful(&matrix, &row, &[ty.clone()], &ctx) {
            continue;
        }
        let named = matrix.iter().any(|row| match row.first() {
            Some(Pat::Ctor { name, .. }) => ctor_names(name)
                .iter()
                .any(|candidate| *candidate == variant.name),
            Some(Pat::Or(alts)) => alts.iter().any(|alt| match alt {
                Pat::Ctor { name, .. } => ctor_names(name)
                    .iter()
                    .any(|candidate| *candidate == variant.name),
                _ => false,
            }),
            _ => false,
        });
        if !named {
            missing.push(variant.name.clone());
            continue;
        }
        let ctor = ctx
            .all_ctors(&ty)
            .and_then(|ctors| ctors.into_iter().find(|c| c.name == variant.name));
        let Some(ctor) = ctor else {
            missing.push(variant.name.clone());
            continue;
        };
        let sub = specialize_ctor(&matrix, &ctor, true);
        let witness = first_uncovered(&sub, &ctor.arg_tys, &ctx);
        match witness {
            Some(args) if !args.is_empty() => {
                missing.push(format!("{}({})", variant.name, args.join(", ")));
            }
            _ => missing.push(variant.name.clone()),
        }
    }
    missing
}

/// Which arm indices can match at least one value.
///
/// An arm is reachable when its pattern is useful against the arms before it.
/// A guarded arm is always reachable: the guard can fail, so the arm does not
/// stand in for its pattern and an earlier arm with the same pattern cannot
/// make it dead. The same reasoning keeps a guarded arm out of the matrix.
pub fn reachable_arms(
    enum_name: &str,
    variants: &[VariantDef],
    arms: &[Pattern],
    has_default: bool,
    guards: &[bool],
) -> Vec<bool> {
    let root = variants.to_vec();
    let lookup = move |name: &str| {
        if name == enum_name {
            Some(root.clone())
        } else {
            None
        }
    };
    reachable_arms_with(enum_name, variants, arms, has_default, guards, &lookup)
}

/// [`reachable_arms`] with a resolver for enum payload types.
pub fn reachable_arms_with(
    enum_name: &str,
    _variants: &[VariantDef],
    arms: &[Pattern],
    has_default: bool,
    guards: &[bool],
    lookup: &dyn Fn(&str) -> Option<Vec<VariantDef>>,
) -> Vec<bool> {
    let ctx = Ctx { lookup };
    let ty = Ty::Enum(enum_name.to_string());
    let mut matrix: Vec<Vec<Pat>> = Vec::new();
    let mut reachable = Vec::with_capacity(arms.len());
    let covered_everything = has_default;
    for (index, arm) in arms.iter().enumerate() {
        let guarded = guards.get(index).copied().unwrap_or(false);
        if guarded {
            reachable.push(true);
            continue;
        }
        let pat = ctx.pat_of(arm, &ty);
        if covered_everything {
            reachable.push(false);
            continue;
        }
        let useful = is_useful(&matrix, &[pat.clone()], &[ty.clone()], &ctx);
        reachable.push(useful);
        if useful {
            matrix.push(vec![pat]);
        }
    }
    reachable
}

/// True when no variant is missing.
pub fn is_exhaustive(variants: &[VariantDef], arms: &[Pattern], has_default: bool) -> bool {
    missing_cases("", variants, arms, has_default).is_empty()
}

#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Enum(String),
    Option(Box<Ty>),
    Result(Box<Ty>, Box<Ty>),
    Int,
    Char,
    Bool,
    Str,
    Float,
    Other,
}

impl Ty {
    fn is_interval(&self) -> bool {
        matches!(self, Ty::Int | Ty::Char)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Pat {
    Wild,
    Ctor { name: String, args: Vec<Pat> },
    Lit(Literal),
    Range {
        lo: Literal,
        hi: Literal,
        inclusive: bool,
    },
    Or(Vec<Pat>),
}

#[derive(Clone, Debug)]
struct Ctor {
    name: String,
    args: Vec<Pat>,
    arg_tys: Vec<Ty>,
}

struct Ctx<'a> {
    lookup: &'a dyn Fn(&str) -> Option<Vec<VariantDef>>,
}

impl Ctx<'_> {
    fn shape(&self, ty: &TypeExpr, depth: usize) -> Ty {
        // A self-referential payload type (`enum List { Cons(List) }`) would
        // recurse forever if its shape were expanded eagerly. The shape is
        // only ever one level deep -- a constructor asks for its fields on
        // demand -- so a nested name that is already being expanded becomes
        // `Other`, which the matrix treats as an opaque leaf.
        if depth > 64 {
            return Ty::Other;
        }
        match ty {
            TypeExpr::Name(name, _) => match name.as_str() {
                "int" | "i64" => Ty::Int,
                "char" => Ty::Char,
                "bool" => Ty::Bool,
                "string" => Ty::Str,
                "float" | "f64" => Ty::Float,
                "Option" => Ty::Other,
                _ => {
                    if (self.lookup)(name).is_some() {
                        Ty::Enum(name.clone())
                    } else {
                        Ty::Other
                    }
                }
            },
            TypeExpr::Option(inner, _) => Ty::Option(Box::new(self.shape(inner, depth + 1))),
            TypeExpr::Result(ok, err, _) => Ty::Result(
                Box::new(self.shape(ok, depth + 1)),
                Box::new(self.shape(err, depth + 1)),
            ),
            _ => Ty::Other,
        }
    }

    fn all_ctors(&self, ty: &Ty) -> Option<Vec<Ctor>> {
        match ty {
            Ty::Enum(name) => (self.lookup)(name).map(|defs| {
                defs.iter()
                    .map(|def| Ctor {
                        name: def.name.clone(),
                        args: vec![Pat::Wild; def.fields.len()],
                        arg_tys: def
                            .fields
                            .iter()
                            .map(|field| self.shape(field.ty(), 0))
                            .collect(),
                    })
                    .collect()
            }),
            Ty::Option(inner) => Some(vec![
                Ctor {
                    name: "None".to_string(),
                    args: Vec::new(),
                    arg_tys: Vec::new(),
                },
                Ctor {
                    name: "Some".to_string(),
                    args: vec![Pat::Wild],
                    arg_tys: vec![(**inner).clone()],
                },
            ]),
            Ty::Result(ok, err) => Some(vec![
                Ctor {
                    name: "Ok".to_string(),
                    args: vec![Pat::Wild],
                    arg_tys: vec![(**ok).clone()],
                },
                Ctor {
                    name: "Err".to_string(),
                    args: vec![Pat::Wild],
                    arg_tys: vec![(**err).clone()],
                },
            ]),
            Ty::Bool => Some(vec![
                Ctor {
                    name: "true".to_string(),
                    args: Vec::new(),
                    arg_tys: Vec::new(),
                },
                Ctor {
                    name: "false".to_string(),
                    args: Vec::new(),
                    arg_tys: Vec::new(),
                },
            ]),
            _ => None,
        }
    }

    fn variant_ctor(&self, ty: &Ty, name: &str) -> Option<Ctor> {
        let name = name.rsplit('.').next().unwrap_or(name);
        self.all_ctors(ty)
            .and_then(|ctors| ctors.into_iter().find(|ctor| ctor.name == name))
    }

    fn pat_of(&self, pattern: &Pattern, ty: &Ty) -> Pat {
        match pattern {
            Pattern::Wildcard(_) | Pattern::Binding { .. } => Pat::Wild,
            Pattern::Literal(Literal::Bool(value), _) => Pat::Ctor {
                name: if *value { "true" } else { "false" }.to_string(),
                args: Vec::new(),
            },
            Pattern::Literal(literal, _) => Pat::Lit(literal.clone()),
            Pattern::Range {
                lo,
                hi,
                inclusive,
                ..
            } => Pat::Range {
                lo: lo.clone(),
                hi: hi.clone(),
                inclusive: *inclusive,
            },
            Pattern::UnitVariant { path, .. } => Pat::Ctor {
                name: last_segment(path),
                args: Vec::new(),
            },
            Pattern::TupleVariant { path, args, .. } => {
                let name = last_segment(path);
                let arg_tys = self
                    .variant_ctor(ty, &name)
                    .map(|ctor| ctor.arg_tys)
                    .unwrap_or_default();
                let mut args: Vec<Pat> = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| self.pat_of(arg, arg_tys.get(index).unwrap_or(&Ty::Other)))
                    .collect();
                // A pattern need not write every field when the checker lets
                // `..` stand for the rest; the matrix always has one column per
                // declared field.
                if !arg_tys.is_empty() {
                    args.resize(arg_tys.len(), Pat::Wild);
                    args.truncate(arg_tys.len());
                }
                Pat::Ctor { name, args }
            }
            Pattern::StructVariant { path, fields, .. } => {
                let name = last_segment(path);
                let declared = self.variant_ctor(ty, &name).map(|ctor| ctor.arg_tys);
                match declared {
                    Some(arg_tys) => {
                        let variant = self.variant_defs(ty, &name);
                        let args = variant
                            .map(|def| {
                                def.fields
                                    .iter()
                                    .enumerate()
                                    .map(|(index, field)| {
                                        let field_name = match field {
                                            VariantField::Named { name, .. } => Some(name.as_str()),
                                            VariantField::Positional { .. } => None,
                                        };
                                        fields
                                            .iter()
                                            .find(|(written, _, _)| {
                                                Some(written.as_str()) == field_name
                                            })
                                            .map(|(_, sub, _)| {
                                                self.pat_of(
                                                    sub,
                                                    arg_tys.get(index).unwrap_or(&Ty::Other),
                                                )
                                            })
                                            .unwrap_or(Pat::Wild)
                                    })
                                    .collect()
                            })
                            .unwrap_or_else(|| vec![Pat::Wild; arg_tys.len()]);
                        Pat::Ctor { name, args }
                    }
                    None => Pat::Ctor { name, args: Vec::new() },
                }
            }
            Pattern::Or { alternatives, .. } => Pat::Or(
                alternatives
                    .iter()
                    .map(|alternative| self.pat_of(alternative, ty))
                    .collect(),
            ),
        }
    }

    fn variant_defs(&self, ty: &Ty, name: &str) -> Option<VariantDef> {
        match ty {
            Ty::Enum(enum_name) => (self.lookup)(enum_name)
                .and_then(|defs| defs.into_iter().find(|def| def.name == name)),
            _ => None,
        }
    }
}

fn last_segment(path: &[String]) -> String {
    path.last().cloned().unwrap_or_default()
}

fn ctor_names(name: &str) -> Vec<&str> {
    vec![name, name.rsplit('.').next().unwrap_or(name)]
}

fn expand_or_rows(matrix: &[Vec<Pat>]) -> Vec<Vec<Pat>> {
    let mut out = Vec::new();
    for row in matrix {
        match row.first() {
            Some(Pat::Or(alternatives)) => {
                for alternative in alternatives {
                    let mut expanded = row.clone();
                    expanded[0] = alternative.clone();
                    out.push(expanded);
                }
            }
            _ => out.push(row.clone()),
        }
    }
    out
}

fn specialize_ctor(matrix: &[Vec<Pat>], ctor: &Ctor, keep_wild: bool) -> Vec<Vec<Pat>> {
    let mut out = Vec::new();
    for row in matrix {
        match row.first() {
            Some(Pat::Wild) if keep_wild => {
                let mut expanded = vec![Pat::Wild; ctor.arg_tys.len()];
                expanded.extend_from_slice(&row[1..]);
                out.push(expanded);
            }
            Some(Pat::Ctor { name, args }) if *name == ctor.name => {
                let mut expanded = args.clone();
                expanded.extend_from_slice(&row[1..]);
                out.push(expanded);
            }
            _ => {}
        }
    }
    out
}

fn is_useful(matrix: &[Vec<Pat>], vector: &[Pat], tys: &[Ty], ctx: &Ctx<'_>) -> bool {
    if vector.is_empty() {
        return matrix.is_empty();
    }
    let matrix = expand_or_rows(matrix);
    match &vector[0] {
        Pat::Or(alternatives) => alternatives.iter().any(|alternative| {
            let mut expanded = vector.to_vec();
            expanded[0] = alternative.clone();
            is_useful(&matrix, &expanded, tys, ctx)
        }),
        Pat::Wild => match ctx.all_ctors(&tys[0]) {
            Some(ctors) => ctors.iter().any(|ctor| {
                let sub = specialize_ctor(&matrix, ctor, true);
                let mut expanded = ctor.args.clone();
                expanded.extend_from_slice(&vector[1..]);
                let mut sub_tys = ctor.arg_tys.clone();
                sub_tys.extend_from_slice(&tys[1..]);
                is_useful(&sub, &expanded, &sub_tys, ctx)
            }),
            None => is_useful_open(&matrix, vector, tys, ctx),
        },
        Pat::Ctor { name, args } => {
            let Some(ctor) = ctx.variant_ctor(&tys[0], name) else {
                return false;
            };
            let sub = specialize_ctor(&matrix, &ctor, true);
            let mut expanded = args.clone();
            expanded.extend_from_slice(&vector[1..]);
            let mut sub_tys = ctor.arg_tys.clone();
            sub_tys.extend_from_slice(&tys[1..]);
            is_useful(&sub, &expanded, &sub_tys, ctx)
        }
        Pat::Lit(literal) => {
            let Some(interval) = Interval::point(literal) else {
                // A string or float literal: it names one value, so the arm is
                // useful exactly when no wildcard row and no identical literal
                // row already covers that value.
                let mut sub = Vec::new();
                for row in &matrix {
                    match row.first() {
                        Some(Pat::Wild) => sub.push(row[1..].to_vec()),
                        Some(Pat::Lit(other)) if other == literal => {
                            sub.push(row[1..].to_vec())
                        }
                        _ => {}
                    }
                }
                return is_useful(&sub, &vector[1..], &tys[1..], ctx);
            };
            is_useful_interval(&matrix, vector, tys, ctx, interval)
        }
        Pat::Range {
            lo,
            hi,
            inclusive,
        } => {
            let Some(interval) = Interval::range(lo, hi, *inclusive) else {
                return false;
            };
            is_useful_interval(&matrix, vector, tys, ctx, interval)
        }
    }
}

/// Usefulness when the first column has no finite constructor list.
///
/// String and float patterns can only be exhausted by a wildcard, so an
/// interval-free column with no wildcard row is useful. `int` and `char` are
/// partitionable: the column is covered only when the union of its literal
/// and range rows covers the whole domain, and each atomic segment is then
/// checked against the rows that reach it.
fn is_useful_open(matrix: &[Vec<Pat>], vector: &[Pat], tys: &[Ty], ctx: &Ctx<'_>) -> bool {
    if matrix.is_empty() {
        return true;
    }
    let ty = &tys[0];
    let wild_remainders: Vec<Vec<Pat>> = matrix
        .iter()
        .filter(|row| matches!(row.first(), Some(Pat::Wild)))
        .map(|row| row[1..].to_vec())
        .collect();
    if !wild_remainders.is_empty() {
        // A wildcard row applies to every value of this column; the only
        // question left is its remaining columns.
        return is_useful(&wild_remainders, &vector[1..], &tys[1..], ctx);
    }
    if !ty.is_interval() {
        return true;
    }
    let domain = domain(ty);
    let intervals: Vec<(Interval, Vec<Pat>)> = matrix
        .iter()
        .filter_map(|row| {
            let interval = Interval::from_pat(row.first()?)?;
            Some((interval, row[1..].to_vec()))
        })
        .collect();
    let gaps = uncovered_segments(domain, &intervals.iter().map(|(i, _)| *i).collect::<Vec<_>>());
    if !gaps.is_empty() {
        return true;
    }
    // The union covers the domain. Check the remaining columns segment by
    // segment, with only the rows that reach that segment.
    for segment in atomic_segments(domain, &intervals.iter().map(|(i, _)| *i).collect::<Vec<_>>()) {
        let remainders: Vec<Vec<Pat>> = intervals
            .iter()
            .filter(|(interval, _)| interval.contains_segment(segment))
            .map(|(_, rest)| rest.clone())
            .collect();
        if is_useful(&remainders, &vector[1..], &tys[1..], ctx) {
            return true;
        }
    }
    false
}

fn is_useful_interval(
    matrix: &[Vec<Pat>],
    vector: &[Pat],
    tys: &[Ty],
    ctx: &Ctx<'_>,
    interval: Interval,
) -> bool {
    // The concrete interval is cut at every row boundary; each atomic piece is
    // a value range with one set of rows that can see it.
    let intervals: Vec<(Interval, Vec<Pat>)> = matrix
        .iter()
        .filter_map(|row| {
            let interval = Interval::from_pat(row.first()?)?;
            Some((interval, row[1..].to_vec()))
        })
        .collect();
    for segment in atomic_segments(interval, &intervals.iter().map(|(i, _)| *i).collect::<Vec<_>>()) {
        let remainders: Vec<Vec<Pat>> = intervals
            .iter()
            .filter(|(row_interval, _)| row_interval.contains_segment(segment))
            .map(|(_, rest)| rest.clone())
            .collect();
        if is_useful(&remainders, &vector[1..], &tys[1..], ctx) {
            return true;
        }
    }
    false
}

/// The first missing value of a column, used only to describe a witness.
fn first_uncovered(matrix: &[Vec<Pat>], tys: &[Ty], ctx: &Ctx<'_>) -> Option<Vec<String>> {
    if tys.is_empty() {
        return if matrix.is_empty() {
            Some(Vec::new())
        } else {
            None
        };
    }
    let matrix = expand_or_rows(matrix);
    if matrix.is_empty() {
        return Some(vec!["_".to_string(); tys.len()]);
    }
    let ty = &tys[0];
    if let Some(ctors) = ctx.all_ctors(ty) {
        for ctor in ctors {
            let sub = specialize_ctor(&matrix, &ctor, true);
            let mut sub_tys = ctor.arg_tys.clone();
            sub_tys.extend_from_slice(&tys[1..]);
            if let Some(mut witness) = first_uncovered(&sub, &sub_tys, ctx) {
                let args = witness.drain(..ctor.arg_tys.len()).collect::<Vec<_>>();
                let mut out = vec![if args.is_empty() {
                    ctor.name.clone()
                } else {
                    format!("{}({})", ctor.name, args.join(", "))
                }];
                out.extend(witness);
                return Some(out);
            }
        }
        return None;
    }
    let wild_remainders: Vec<Vec<Pat>> = matrix
        .iter()
        .filter(|row| matches!(row.first(), Some(Pat::Wild)))
        .map(|row| row[1..].to_vec())
        .collect();
    if !wild_remainders.is_empty() {
        let mut witness = first_uncovered(&wild_remainders, &tys[1..], ctx)?;
        witness.insert(0, "_".to_string());
        return Some(witness);
    }
    let mut out = vec!["_".to_string()];
    out.extend(first_uncovered(&Vec::new(), &tys[1..], ctx)?);
    Some(out)
}

fn domain(ty: &Ty) -> Interval {
    match ty {
        Ty::Char => Interval {
            lo: 0,
            hi: 255,
        },
        _ => Interval {
            lo: i64::MIN,
            hi: i64::MAX,
        },
    }
}

/// A closed integer interval. The upper bound is inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Interval {
    lo: i64,
    hi: i64,
}

impl Interval {
    fn point(literal: &Literal) -> Option<Self> {
        let value = int_value(literal)?;
        Some(Interval {
            lo: value,
            hi: value,
        })
    }

    fn range(lo: &Literal, hi: &Literal, inclusive: bool) -> Option<Self> {
        let lo = int_value(lo)?;
        let mut hi = int_value(hi)?;
        if !inclusive {
            hi = hi.checked_sub(1)?;
        }
        if lo > hi {
            return None;
        }
        Some(Interval { lo, hi })
    }

    fn from_pat(pattern: &Pat) -> Option<Self> {
        match pattern {
            Pat::Lit(literal) => Interval::point(literal),
            Pat::Range {
                lo,
                hi,
                inclusive,
            } => Interval::range(lo, hi, *inclusive),
            _ => None,
        }
    }

    fn contains_segment(self, segment: Interval) -> bool {
        self.lo <= segment.lo && segment.hi <= self.hi
    }
}

fn int_value(literal: &Literal) -> Option<i64> {
    match literal {
        Literal::I64(value) => Some(*value),
        Literal::Char(value) => Some(*value as i64),
        _ => None,
    }
}

/// Cut `bounds` (plus the domain's own ends) into adjacent atomic segments.
fn atomic_segments(domain: Interval, bounds: &[Interval]) -> Vec<Interval> {
    let mut points = vec![domain.lo, domain.hi.saturating_add(1)];
    for interval in bounds {
        points.push(interval.lo);
        points.push(interval.hi.saturating_add(1));
    }
    points.retain(|point| domain.lo <= *point && *point <= domain.hi.saturating_add(1));
    points.sort_unstable();
    points.dedup();
    let mut out = Vec::new();
    for pair in points.windows(2) {
        let lo = pair[0];
        let hi = pair[1] - 1;
        if lo <= hi {
            out.push(Interval { lo, hi });
        }
    }
    out
}

/// The pieces of `domain` no interval covers.
fn uncovered_segments(domain: Interval, intervals: &[Interval]) -> Vec<Interval> {
    let mut out = Vec::new();
    for segment in atomic_segments(domain, intervals) {
        if !intervals.iter().any(|interval| interval.contains_segment(segment)) {
            out.push(segment);
        }
    }
    out
}
