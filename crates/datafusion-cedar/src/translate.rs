//! Translate a Cedar residual (the leftover condition from partial evaluation)
//! into a DataFusion [`Expr`].
//!
//! After partial evaluation with a concrete principal/action/context and an
//! *unknown* resource, the residual condition references only `resource.<attr>`.
//! We map `resource.<attr>` to `col(<attr>)` and Cedar operators to DataFusion
//! expression operators. The grammar is deliberately restricted (equality,
//! comparison, boolean combinators, `like`, membership); anything outside it
//! fails — and the caller treats a failure as fail-closed (deny the row / mask
//! the column).
//!
//! The residual is read via its EST JSON (`Policy::to_json`), whose expression
//! nodes are operator-keyed objects (e.g. `{"==": {"left": .., "right": ..}}`),
//! attribute access `{".": {"left": {"Var": "resource"}, "attr": "region"}}`,
//! and literals `{"Value": ...}`.

use datafusion::common::{Result, plan_datafusion_err};
use datafusion::logical_expr::{Expr, col, lit};
use serde_json::Value;

use cedar_policy::Policy;

/// Translates a Cedar residual policy into a DataFusion predicate.
///
/// A seam: the default [`CedarResidualTranslator`] reads Cedar EST JSON, but a
/// future `CelTranslator` (consuming Policast-style CEL manifests) could
/// implement the same trait without touching the enforcement layer.
pub trait ResidualTranslator: std::fmt::Debug + Send + Sync {
    /// Translate the residual's condition into a row-filter predicate
    /// (`resource.<attr>` mapped to `col(<attr>)`). `None` means the residual
    /// is trivially true (no filter needed).
    fn to_predicate(&self, residual: &Policy) -> Result<Option<Expr>>;
}

/// Reads the Cedar residual's EST JSON and lowers its condition to an [`Expr`].
#[derive(Debug, Default)]
pub struct CedarResidualTranslator;

impl ResidualTranslator for CedarResidualTranslator {
    fn to_predicate(&self, residual: &Policy) -> Result<Option<Expr>> {
        let json = residual
            .to_json()
            .map_err(|e| plan_datafusion_err!("failed to serialize residual: {e}"))?;

        // EST policy: { "effect", "principal", "action", "resource",
        //               "conditions": [ { "kind": "when"|"unless", "body": <expr> } ] }
        let conditions = json
            .get("conditions")
            .and_then(Value::as_array)
            .ok_or_else(|| plan_datafusion_err!("residual has no conditions array"))?;

        let mut predicate: Option<Expr> = None;
        for cond in conditions {
            let kind = cond.get("kind").and_then(Value::as_str).unwrap_or("when");
            let body = cond
                .get("body")
                .ok_or_else(|| plan_datafusion_err!("residual condition has no body"))?;
            let mut expr = translate_expr(body)?;
            // `unless { c }` is equivalent to `when { !c }`.
            if kind == "unless" {
                expr = !expr;
            }
            predicate = Some(match predicate {
                Some(acc) => acc.and(expr),
                None => expr,
            });
        }
        Ok(predicate)
    }
}

/// Translate one EST expression node into a DataFusion [`Expr`].
fn translate_expr(node: &Value) -> Result<Expr> {
    let obj = node
        .as_object()
        .ok_or_else(|| plan_datafusion_err!("expected residual expression object"))?;

    // Leaf: literal value.
    if let Some(value) = obj.get("Value") {
        return translate_value(value);
    }
    // Leaf: variable (resource/principal/...). Only used as the base of a `.`
    // access, handled in `translate_get_attr`; a bare Var is unsupported.
    if obj.contains_key("Var") {
        return Err(plan_datafusion_err!(
            "bare Cedar variable is not translatable to a predicate"
        ));
    }

    // Single-key operator nodes.
    let (op, args) = obj
        .iter()
        .next()
        .ok_or_else(|| plan_datafusion_err!("empty residual expression node"))?;

    match op.as_str() {
        "." => translate_get_attr(args),
        "==" => binary(args, |l, r| l.eq(r)),
        "!=" => binary(args, |l, r| l.not_eq(r)),
        "<" => binary(args, |l, r| l.lt(r)),
        "<=" => binary(args, |l, r| l.lt_eq(r)),
        ">" => binary(args, |l, r| l.gt(r)),
        ">=" => binary(args, |l, r| l.gt_eq(r)),
        "&&" => binary(args, |l, r| l.and(r)),
        "||" => binary(args, |l, r| l.or(r)),
        "!" => {
            let arg = args
                .get("arg")
                .ok_or_else(|| plan_datafusion_err!("`!` missing arg"))?;
            Ok(!translate_expr(arg)?)
        }
        "like" => translate_like(args),
        other => Err(plan_datafusion_err!(
            "unsupported Cedar operator in residual: '{other}'"
        )),
    }
}

/// `{ "left": <expr>, "attr": "<name>" }` over `resource` -> `col(name)`.
fn translate_get_attr(args: &Value) -> Result<Expr> {
    let left = args
        .get("left")
        .ok_or_else(|| plan_datafusion_err!("`.` missing left"))?;
    let attr = args
        .get("attr")
        .and_then(Value::as_str)
        .ok_or_else(|| plan_datafusion_err!("`.` missing attr"))?;

    let base_is_resource = left
        .as_object()
        .and_then(|o| o.get("Var"))
        .and_then(Value::as_str)
        == Some("resource");

    if base_is_resource {
        Ok(col(attr))
    } else {
        // principal.* should have been folded out by partial evaluation; any
        // remaining non-resource attribute access is not a column reference.
        Err(plan_datafusion_err!(
            "residual references a non-resource attribute '{attr}'; not a column"
        ))
    }
}

/// `{ "left": <expr>, "right": <expr> }` binary op.
fn binary(args: &Value, f: impl Fn(Expr, Expr) -> Expr) -> Result<Expr> {
    let left = args
        .get("left")
        .ok_or_else(|| plan_datafusion_err!("binary op missing left"))?;
    let right = args
        .get("right")
        .ok_or_else(|| plan_datafusion_err!("binary op missing right"))?;
    Ok(f(translate_expr(left)?, translate_expr(right)?))
}

/// `{ "left": <expr>, "pattern": [..] }` -> SQL LIKE.
///
/// Cedar serializes a pattern as an array of elements that are either the
/// string `"Wildcard"` or `{ "Literal": "<char-or-string>" }`. Anything we
/// don't recognize fails closed (the caller denies the row).
fn translate_like(args: &Value) -> Result<Expr> {
    let left = args
        .get("left")
        .ok_or_else(|| plan_datafusion_err!("`like` missing left"))?;
    let pattern = args
        .get("pattern")
        .and_then(Value::as_array)
        .ok_or_else(|| plan_datafusion_err!("`like` missing/!array pattern"))?;

    let mut sql = String::new();
    for elem in pattern {
        match elem {
            Value::String(s) if s == "Wildcard" => sql.push('%'),
            Value::Object(o) => {
                let literal = o
                    .get("Literal")
                    .and_then(Value::as_str)
                    .ok_or_else(|| plan_datafusion_err!("unsupported `like` pattern element"))?;
                // Escape SQL LIKE metacharacters in literal chunks.
                for c in literal.chars() {
                    if c == '%' || c == '_' {
                        sql.push('\\');
                    }
                    sql.push(c);
                }
            }
            _ => return Err(plan_datafusion_err!("unsupported `like` pattern element")),
        }
    }
    Ok(translate_expr(left)?.like(lit(sql)))
}

/// Translate a Cedar literal value (string/long/bool) to a DataFusion literal.
fn translate_value(value: &Value) -> Result<Expr> {
    match value {
        Value::String(s) => Ok(lit(s.clone())),
        Value::Bool(b) => Ok(lit(*b)),
        Value::Number(n) if n.is_i64() => Ok(lit(n.as_i64().unwrap())),
        Value::Number(n) if n.is_u64() => Ok(lit(n.as_u64().unwrap() as i64)),
        _ => Err(plan_datafusion_err!(
            "unsupported Cedar literal in residual: {value}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use datafusion::logical_expr::col;
    use serde_json::json;

    use super::*;

    // `resource.region == "eu"`
    #[test]
    fn translates_resource_eq_literal() {
        let node = json!({
            "==": {
                "left": { ".": { "left": { "Var": "resource" }, "attr": "region" } },
                "right": { "Value": "eu" }
            }
        });
        let expr = translate_expr(&node).unwrap();
        assert_eq!(expr, col("region").eq(lit("eu")));
    }

    // `resource.a == 1 && resource.b == "x"`
    #[test]
    fn translates_conjunction() {
        let node = json!({
            "&&": {
                "left": { "==": { "left": { ".": { "left": { "Var": "resource" }, "attr": "a" } }, "right": { "Value": 1 } } },
                "right": { "==": { "left": { ".": { "left": { "Var": "resource" }, "attr": "b" } }, "right": { "Value": "x" } } }
            }
        });
        let expr = translate_expr(&node).unwrap();
        assert_eq!(expr, col("a").eq(lit(1i64)).and(col("b").eq(lit("x"))));
    }

    // principal.* should be folded out by partial eval; a residual that still
    // references a non-resource attribute is not a column -> error (fail-closed).
    #[test]
    fn rejects_non_resource_attribute() {
        let node = json!({
            "==": {
                "left": { ".": { "left": { "Var": "principal" }, "attr": "role" } },
                "right": { "Value": "admin" }
            }
        });
        assert!(translate_expr(&node).is_err());
    }

    #[test]
    fn rejects_unsupported_operator() {
        let node = json!({ "containsAll": { "left": { "Var": "resource" }, "right": { "Value": "x" } } });
        assert!(translate_expr(&node).is_err());
    }
}
