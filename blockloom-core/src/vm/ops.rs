//! The built-in operators over already-evaluated arguments. `Value::eval`
//! says what each one means; this is the same thing without a tree to walk,
//! so the VM can evaluate a slot without rebuilding it first. The tests below
//! hold the two against each other.

use crate::value::{Evaluated, Op, Value, ext_operator};

type R = Result<Evaluated, String>;

/// An operator over up to two evaluated arguments. An argument's error is
/// only reported if the operator reads it, which is what makes `false and
/// <a bad slot>` quiet.
pub(super) fn apply(op: &Op, a: Option<R>, b: Option<R>) -> R {
    let a = a.unwrap_or_else(missing);
    let b = b.unwrap_or_else(missing);
    let result = match op {
        Op::Add => Evaluated::Number(a?.as_number()? + b?.as_number()?),
        Op::Sub => Evaluated::Number(a?.as_number()? - b?.as_number()?),
        Op::Mul => Evaluated::Number(a?.as_number()? * b?.as_number()?),
        Op::Div => {
            let (l, r) = (a?.as_number()?, b?.as_number()?);
            if r == 0.0 {
                return Err("division by zero".to_string());
            }
            Evaluated::Number(l / r)
        }
        Op::Mod => {
            let (l, r) = (a?.as_number()?, b?.as_number()?);
            if r == 0.0 {
                return Err("mod by zero".to_string());
            }
            Evaluated::Number(l.rem_euclid(r))
        }
        Op::Random => {
            // The random source is blockstitch's own.
            let (l, r) = (a?.as_number()?, b?.as_number()?);
            return Value::op(Op::Random, vec![Value::number(l), Value::number(r)]).eval();
        }
        Op::CurrentTime => {
            return Value::op(Op::CurrentTime, vec![a?.into_value()]).eval();
        }
        Op::Round => Evaluated::Number(a?.as_number()?.round()),
        Op::Math => {
            let function = a?.as_text();
            math(&function, b?.as_number()?)?
        }
        Op::NewLine => Evaluated::Text("\n".to_string()),
        Op::Tab => Evaluated::Text("\t".to_string()),
        Op::True => Evaluated::Bool(true),
        Op::False => Evaluated::Bool(false),
        Op::Length => Evaluated::Number(a?.as_text().chars().count() as f64),
        Op::IndexOf => {
            let needle = a?.as_text();
            Evaluated::Number(char_index_of(&b?.as_text(), &needle) as f64)
        }
        Op::LastIndexOf => {
            let needle = a?.as_text();
            Evaluated::Number(char_last_index_of(&b?.as_text(), &needle) as f64)
        }
        Op::LetterOf => {
            let index = a?.as_number()? as i64;
            let chars: Vec<char> = b?.as_text().chars().collect();
            if index < 1 || index as usize > chars.len() {
                return Err(format!(
                    "letter {index} is out of range for a {}-character value",
                    chars.len()
                ));
            }
            Evaluated::Text(chars[index as usize - 1].to_string())
        }
        Op::Case => {
            let text = a?.as_text();
            if b?.as_text() == "Upper" {
                Evaluated::Text(text.to_uppercase())
            } else {
                Evaluated::Text(text.to_lowercase())
            }
        }
        Op::Not => Evaluated::Bool(!a?.as_bool()),
        Op::And => Evaluated::Bool(a?.as_bool() && b?.as_bool()),
        Op::Or => Evaluated::Bool(a?.as_bool() || b?.as_bool()),
        Op::Eq => Evaluated::Bool(values_equal(&a?, &b?)),
        Op::Neq => Evaluated::Bool(!values_equal(&a?, &b?)),
        Op::Gt => Evaluated::Bool(a?.as_number()? > b?.as_number()?),
        Op::Lt => Evaluated::Bool(a?.as_number()? < b?.as_number()?),
        Op::Gte => Evaluated::Bool(a?.as_number()? >= b?.as_number()?),
        Op::Lte => Evaluated::Bool(a?.as_number()? <= b?.as_number()?),
        Op::Join | Op::Ext(_) => {
            return apply_many(op, vec![a, b]);
        }
    };
    Ok(result)
}

/// The operators that read every argument: `join`, and the host's own.
pub(super) fn apply_many(op: &Op, args: Vec<R>) -> R {
    match op {
        Op::Join => {
            let mut joined = String::new();
            for arg in args {
                joined.push_str(&arg?.as_text());
            }
            Ok(Evaluated::Text(joined))
        }
        Op::Ext(name) => {
            let operator = ext_operator(name).ok_or_else(|| {
                format!("unknown operator '{name}' - was the host's operator registry installed?")
            })?;
            let args = args.into_iter().collect::<Result<Vec<_>, _>>()?;
            (operator.eval)(&args)
        }
        _ => {
            let mut args = args.into_iter();
            apply(op, args.next(), args.next())
        }
    }
}

/// Stands in for an argument a malformed tree left out.
fn missing() -> R {
    Err("an operator is missing an argument".to_string())
}

fn math(function: &str, n: f64) -> R {
    let to_radians = std::f64::consts::PI / 180.0;
    let to_degrees = 180.0 / std::f64::consts::PI;
    let result = match function {
        "Abs" => n.abs(),
        "Floor" => n.floor(),
        "Ceiling" => n.ceil(),
        "Sign" => {
            if n > 0.0 {
                1.0
            } else if n < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        "Sqrt" => n.sqrt(),
        "Sin" => (n * to_radians).sin(),
        "Cos" => (n * to_radians).cos(),
        "Tan" => (n * to_radians).tan(),
        "Asin" => n.asin() * to_degrees,
        "Acos" => n.acos() * to_degrees,
        "Atan" => n.atan() * to_degrees,
        "Ln" => n.ln(),
        "Log" => n.log10(),
        "Log2" => n.log2(),
        "EPower" => n.exp(),
        "TenPower" => 10.0_f64.powf(n),
        other => return Err(format!("unknown math function '{other}'")),
    };
    Ok(Evaluated::Number(result))
}

/// `Op::Eq`'s rule: numeric when both sides parse as numbers, text otherwise.
fn values_equal(l: &Evaluated, r: &Evaluated) -> bool {
    match (l.as_number(), r.as_number()) {
        (Ok(l), Ok(r)) => l == r,
        _ => l.as_text() == r.as_text(),
    }
}

/// 1-based char index of the first `needle` in `haystack`, 0 for none.
fn char_index_of(haystack: &str, needle: &str) -> usize {
    let h: Vec<char> = haystack.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() || n.len() > h.len() {
        return 0;
    }
    (0..=h.len() - n.len())
        .find(|&i| h[i..i + n.len()] == n[..])
        .map_or(0, |i| i + 1)
}

/// [`char_index_of`] from the other end.
fn char_last_index_of(haystack: &str, needle: &str) -> usize {
    let h: Vec<char> = haystack.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() || n.len() > h.len() {
        return 0;
    }
    (0..=h.len() - n.len())
        .rev()
        .find(|&i| h[i..i + n.len()] == n[..])
        .map_or(0, |i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Evaluates `op` over `args` the way the VM now does: each argument on
    /// its own, then the operator.
    fn direct(op: &Op, args: &[Value]) -> R {
        let evaluated: Vec<R> = args.iter().map(Value::eval).collect();
        match op {
            Op::Join | Op::Ext(_) => apply_many(op, evaluated),
            _ => {
                let mut args = evaluated.into_iter();
                apply(op, args.next(), args.next())
            }
        }
    }

    /// Equality that holds NaN to itself, since `sqrt(-1)` is NaN both ways.
    #[track_caller]
    fn assert_same(direct: R, tree: R, context: impl std::fmt::Debug) {
        assert_eq!(format!("{direct:?}"), format!("{tree:?}"), "{context:?}");
    }

    /// Operands that exercise every coercion, plus two that fail to evaluate.
    fn operands() -> Vec<Value> {
        vec![
            Value::number(7.0),
            Value::number(-2.5),
            Value::number(0.0),
            Value::number(3.0),
            Value::text("12"),
            Value::text(" 4.5 "),
            Value::text("hello"),
            Value::text(""),
            Value::text("false"),
            Value::text("Upper"),
            Value::text("Sqrt"),
            Value::text("ll"),
            Value::op(Op::True, vec![]),
            Value::op(Op::False, vec![]),
            Value::Bool,
            Value::op(Op::Div, vec![Value::number(1.0), Value::number(0.0)]),
            Value::Var {
                name: "unresolved".to_string(),
            },
        ]
    }

    #[test]
    fn every_builtin_agrees_with_value_eval() {
        let unary = [Op::Round, Op::Length, Op::Not];
        let binary = [
            Op::Add,
            Op::Sub,
            Op::Mul,
            Op::Div,
            Op::Mod,
            Op::Math,
            Op::IndexOf,
            Op::LastIndexOf,
            Op::LetterOf,
            Op::Case,
            Op::Eq,
            Op::Neq,
            Op::Gt,
            Op::Lt,
            Op::Gte,
            Op::Lte,
            Op::And,
            Op::Or,
        ];
        let operands = operands();
        for op in [Op::True, Op::False, Op::NewLine, Op::Tab] {
            assert_same(direct(&op, &[]), Value::op(op.clone(), vec![]).eval(), &op);
        }
        for op in unary {
            for a in &operands {
                let args = [a.clone()];
                let tree = Value::op(op.clone(), args.to_vec());
                assert_same(direct(&op, &args), tree.eval(), format!("{op:?} {a:?}"));
            }
        }
        for op in binary {
            for a in &operands {
                for b in &operands {
                    let args = [a.clone(), b.clone()];
                    let tree = Value::op(op.clone(), args.to_vec());
                    assert_same(
                        direct(&op, &args),
                        tree.eval(),
                        format!("{op:?} {a:?} {b:?}"),
                    );
                }
            }
        }
        for a in &operands {
            for b in &operands {
                let args = [a.clone(), b.clone(), Value::text("!")];
                let tree = Value::op(Op::Join, args.to_vec());
                assert_same(
                    direct(&Op::Join, &args),
                    tree.eval(),
                    format!("join {a:?} {b:?}"),
                );
            }
        }
    }

    #[test]
    fn random_and_the_clock_fail_the_way_value_eval_does() {
        let bad = Value::op(Op::Div, vec![Value::number(1.0), Value::number(0.0)]);
        let word = Value::text("hello");
        for args in [
            [bad.clone(), Value::number(1.0)],
            [Value::number(1.0), bad.clone()],
            [word.clone(), bad.clone()],
            [Value::number(1.0), word.clone()],
        ] {
            let tree = Value::op(Op::Random, args.to_vec());
            assert_same(direct(&Op::Random, &args), tree.eval(), &args);
        }
        let roll = direct(&Op::Random, &[Value::number(3.0), Value::number(3.0)]);
        assert_eq!(roll, Ok(Evaluated::Number(3.0)));
        for arg in [bad, Value::text("Nope")] {
            let tree = Value::op(Op::CurrentTime, vec![arg.clone()]);
            assert_same(
                direct(&Op::CurrentTime, std::slice::from_ref(&arg)),
                tree.eval(),
                arg,
            );
        }
    }

    #[test]
    fn a_host_operator_is_looked_up_before_its_arguments() {
        let bad = Value::op(Op::Div, vec![Value::number(1.0), Value::number(0.0)]);
        let unknown = Op::Ext("NoSuchOperator".into());
        let tree = Value::op(unknown.clone(), vec![bad.clone()]);
        assert_same(direct(&unknown, &[bad]), tree.eval(), unknown);
    }
}
