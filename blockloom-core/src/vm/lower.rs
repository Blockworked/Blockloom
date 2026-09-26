//! A slot's `Value` lowered for the VM: variable names interned to slots,
//! temps to indices, and pure operators over constants folded away. Lowering
//! happens once per slot; evaluating the result hashes nothing.

use super::ops;
use super::program::{Program, temp_index};
use super::variables::{VarId, Variables};
use crate::value::{Evaluated, Op, Value};
use blockstitch_core::graph::{is_dict_reporter, is_list_reporter};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;

#[derive(Debug)]
pub(super) enum Expr {
    /// A literal, or pure operators over literals, already evaluated.
    Const(Result<Evaluated, String>),
    Var(VarId),
    /// A reporter result slot; the variable is what it reads when the slot
    /// hasn't been filled, as the name `~tN` always has.
    Temp(usize, VarId),
    Param(Box<str>),
    Call {
        block_id: Box<str>,
        args: Box<[Expr]>,
    },
    Op {
        op: Op,
        args: Box<[Expr]>,
    },
    /// A list or dict reporter, named by its wire name.
    Store {
        op: Box<str>,
        list: bool,
        args: Box<[Expr]>,
    },
}

pub(super) fn lower(value: &Value, variables: &Variables) -> Expr {
    match value {
        Value::Number { value } => Expr::Const(Ok(Evaluated::Number(*value))),
        Value::Text { value } => Expr::Const(Ok(Evaluated::Text(value.clone()))),
        Value::Bool => Expr::Const(Ok(Evaluated::Bool(false))),
        Value::Var { name } => match temp_index(name) {
            Some(index) => Expr::Temp(index, variables.intern(name)),
            None => Expr::Var(variables.intern(name)),
        },
        Value::Param { name } => Expr::Param(name.as_str().into()),
        Value::Call { block_id, args, .. } => Expr::Call {
            block_id: block_id.as_str().into(),
            args: args.iter().map(|arg| lower(arg, variables)).collect(),
        },
        Value::Op {
            op: op @ Op::Ext(name),
            args,
            ..
        } if is_list_reporter(op) || is_dict_reporter(op) => Expr::Store {
            op: name.clone(),
            list: is_list_reporter(op),
            args: args.iter().map(|arg| lower(arg, variables)).collect(),
        },
        Value::Op { op, args, .. } => {
            let args: Box<[Expr]> = args.iter().map(|arg| lower(arg, variables)).collect();
            fold(op, &args).unwrap_or(Expr::Op {
                op: op.clone(),
                args,
            })
        }
    }
}

/// Evaluates an operator now when it can't tell the difference: every
/// argument is a constant and the operator reads nothing but them.
fn fold(op: &Op, args: &[Expr]) -> Option<Expr> {
    if matches!(op, Op::Random | Op::CurrentTime | Op::Ext(_)) {
        return None;
    }
    let values = args
        .iter()
        .map(|arg| match arg {
            Expr::Const(value) => Some(value.clone()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let result = match op {
        Op::Join => ops::apply_many(op, values),
        _ => {
            let mut values = values.into_iter();
            ops::apply(op, values.next(), values.next())
        }
    };
    Some(Expr::Const(result))
}

/// A compiled program plus its slots as lowered so far, keyed by where each
/// `Value` or variable name sits in the program. The program never changes
/// after compiling and lives as long as this does, so an address is a stable
/// key and a clone sharing the program shares the lowering too.
#[derive(Debug, Default)]
pub(super) struct Loaded {
    program: Program,
    /// Custom block id -> where its body starts and its input names.
    blocks: FxHashMap<String, (usize, Rc<[String]>)>,
    exprs: RefCell<FxHashMap<*const Value, Rc<Expr>>>,
    names: RefCell<FxHashMap<*const String, VarId>>,
}

impl Loaded {
    /// `inputs` is each custom block's input names, in declaration order.
    pub(super) fn new(program: Program, mut inputs: FxHashMap<String, Rc<[String]>>) -> Self {
        let blocks = program
            .blocks
            .iter()
            .map(|(id, &start)| {
                let names = inputs.remove(id).unwrap_or_else(|| Rc::from([]));
                (id.clone(), (start, names))
            })
            .collect();
        Self {
            program,
            blocks,
            ..Default::default()
        }
    }

    /// Where a custom block's body starts, and its input names.
    pub(super) fn block(&self, id: &str) -> Option<(usize, Rc<[String]>)> {
        self.blocks
            .get(id)
            .map(|(start, names)| (*start, Rc::clone(names)))
    }

    /// `value`, lowered. It must be a slot inside this program.
    pub(super) fn expr(&self, value: &Value, variables: &Variables) -> Rc<Expr> {
        let key = value as *const Value;
        if let Some(expr) = self.exprs.borrow().get(&key) {
            return Rc::clone(expr);
        }
        let expr = Rc::new(lower(value, variables));
        self.exprs.borrow_mut().insert(key, Rc::clone(&expr));
        expr
    }

    /// A variable name inside this program, interned.
    pub(super) fn var(&self, name: &String, variables: &Variables) -> VarId {
        let key = name as *const String;
        if let Some(&var) = self.names.borrow().get(&key) {
            return var;
        }
        let var = variables.intern(name);
        self.names.borrow_mut().insert(key, var);
        var
    }
}

impl Deref for Loaded {
    type Target = Program;

    fn deref(&self) -> &Program {
        &self.program
    }
}
