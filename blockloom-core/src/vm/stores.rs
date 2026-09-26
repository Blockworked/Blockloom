//! List and dict reporters over already-evaluated arguments. blockstitch
//! resolves them over `Value` trees and reads the store's name first; this
//! hands it literals instead, failing with whichever argument it would have
//! read first, so the VM only copies the one list or dict the name picks.

use crate::value::{Evaluated, Value};

type R = Result<Evaluated, String>;

/// The arguments a reporter reads, in the order it reads them. The first is
/// always the list or dict it names.
fn reads(op: &str) -> &'static [usize] {
    match op {
        "ListItem" | "ListItemNumber" | "ListAmount" | "ListItemExists" | "DictValue" => &[1, 0],
        "ListContains" | "DictHasKey" => &[0, 1],
        _ => &[0],
    }
}

/// The store's name and the arguments as literals, or the error the reporter
/// would meet first. An argument it never reads stands in as zero.
pub(super) fn literal_args(op: &str, args: Vec<R>) -> Result<(Option<String>, Vec<Value>), String> {
    let order = reads(op);
    for &index in order {
        match args.get(index) {
            Some(Err(message)) => return Err(message.clone()),
            Some(Ok(_)) => {}
            // blockstitch stops at a missing argument, with its own error.
            None => break,
        }
    }
    let name = args
        .get(order[0])
        .and_then(|name| Some(name.as_ref().ok()?.as_text()));
    let values = args
        .into_iter()
        .map(|arg| arg.unwrap_or(Evaluated::Number(0.0)).into_value())
        .collect();
    Ok((name, values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Op;
    use blockstitch_core::graph::{
        DictEntry, DictItem, ListItem, resolve_dict_reporter, resolve_list_reporter,
    };
    use std::collections::HashMap;

    fn list_ops() -> [&'static str; 8] {
        [
            "ListItem",
            "ListItemNumber",
            "ListAmount",
            "ListItemExists",
            "ListContains",
            "ListLength",
            "ListIsEmpty",
            "ListAsJson",
        ]
    }

    fn dict_ops() -> [&'static str; 6] {
        [
            "DictValue",
            "DictHasKey",
            "DictSize",
            "DictIsEmpty",
            "DictKeys",
            "DictAsJson",
        ]
    }

    /// Argument trees, two of which fail with different messages so the
    /// order they are read in shows.
    fn operands() -> Vec<Value> {
        vec![
            Value::number(2.0),
            Value::number(0.0),
            Value::text("things"),
            Value::text("b"),
            Value::text("nope"),
            Value::op(Op::True, vec![]),
            Value::op(Op::Div, vec![Value::number(1.0), Value::number(0.0)]),
            Value::op(Op::Mod, vec![Value::number(1.0), Value::number(0.0)]),
        ]
    }

    fn arg_lists() -> Vec<Vec<Value>> {
        let operands = operands();
        let mut lists = vec![vec![]];
        for a in &operands {
            lists.push(vec![a.clone()]);
            for b in &operands {
                lists.push(vec![a.clone(), b.clone()]);
            }
        }
        lists
    }

    type Resolve<S> = fn(&str, Vec<Value>, &HashMap<String, S>) -> Result<Value, String>;

    /// The old path's result: blockstitch over the unevaluated trees.
    fn through_trees<S>(
        resolve: Resolve<S>,
        op: &str,
        args: &[Value],
        stores: &HashMap<String, S>,
    ) -> R {
        resolve(op, args.to_vec(), stores)?.eval()
    }

    /// The new one: arguments first, then blockstitch over literals.
    fn through_literals<S: Clone>(
        resolve: Resolve<S>,
        op: &str,
        args: &[Value],
        stores: &HashMap<String, S>,
    ) -> R {
        let evaluated = args.iter().map(Value::eval).collect();
        let (name, literals) = literal_args(op, evaluated)?;
        let only: HashMap<String, S> = name
            .and_then(|name| Some((name.clone(), stores.get(&name)?.clone())))
            .into_iter()
            .collect();
        resolve(op, literals, &only)?.eval()
    }

    #[test]
    fn a_list_reporter_agrees_with_resolving_its_trees() {
        let things = vec![
            ListItem::Text("a".to_string()),
            ListItem::Text("b".to_string()),
            ListItem::Number(2.0),
            ListItem::Text("b".to_string()),
        ];
        let lists = HashMap::from([
            ("things".to_string(), things),
            ("2".to_string(), vec![ListItem::Number(7.0)]),
        ]);
        for op in list_ops() {
            for args in arg_lists() {
                // blockstitch indexes a missing needle outright.
                if matches!(op, "ListItemNumber" | "ListAmount") && args.is_empty()
                    || op == "ListContains" && args.len() < 2
                {
                    continue;
                }
                assert_eq!(
                    through_literals(resolve_list_reporter, op, &args, &lists),
                    through_trees(resolve_list_reporter, op, &args, &lists),
                    "{op} {args:?}"
                );
            }
        }
    }

    #[test]
    fn a_dict_reporter_agrees_with_resolving_its_trees() {
        let entries = vec![
            DictEntry {
                key: "b".to_string(),
                value: DictItem::Number(3.0),
            },
            DictEntry {
                key: "2".to_string(),
                value: DictItem::Text("two".to_string()),
            },
        ];
        let dicts = HashMap::from([("things".to_string(), entries)]);
        for op in dict_ops() {
            for args in arg_lists() {
                assert_eq!(
                    through_literals(resolve_dict_reporter, op, &args, &dicts),
                    through_trees(resolve_dict_reporter, op, &args, &dicts),
                    "{op} {args:?}"
                );
            }
        }
    }
}
