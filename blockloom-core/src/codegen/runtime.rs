// What a generated program is built on: a value type, the operators over it,
// and the boundary it asks the world through. Like `script/abi.rs`, this file
// is compiled twice - once into `blockloom-core`, where the tests can hold it
// against the VM's own semantics, and once as text into every program
// `codegen` emits. Editing it changes both at once, which is the point.
//
// Nothing here may use anything but `std`: a generated program is compiled by
// one `rustc` run with no dependencies at all.

/// A value as a generated program carries it. The same three cases
/// `Evaluated` has, with the same coercions - a difference here is a
/// difference in what a game does.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Num(f64),
    Text(String),
    Bool(bool),
}

/// What an expression evaluates to. One error anywhere gives up the whole
/// tree, exactly as `Value::eval` does, and the caller reports it once.
pub type R = Result<Val, String>;

impl Val {
    pub fn as_number(&self) -> Result<f64, String> {
        match self {
            Val::Num(n) => Ok(*n),
            Val::Text(s) => s
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("\"{s}\" is not a number")),
            Val::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0,
            Val::Text(s) => !s.is_empty() && s != "false",
        }
    }

    pub fn as_text(&self) -> String {
        match self {
            Val::Text(s) => s.clone(),
            Val::Num(n) => n.to_string(),
            Val::Bool(b) => b.to_string(),
        }
    }
}

/// What a generated program asks the world to do. One per leaf instruction
/// the blocks have, carrying values that are already evaluated - the host
/// never evaluates anything, just as it never does for the VM.
///
/// Names that are enums elsewhere (a body kind, a camera view) travel as the
/// text they serialize as, so this file stays free of everything but `std`.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Move {
        steps: f32,
    },
    GoTo {
        position: [f32; 3],
    },
    ChangePosition {
        axis: usize,
        by: f32,
    },
    /// Starts a slide; the strand sleeps for exactly as long.
    Glide {
        seconds: f32,
        target: [f32; 3],
    },
    Turn {
        axis: usize,
        degrees: f32,
    },
    SetRotation {
        axis: usize,
        degrees: f32,
    },
    PointTowards {
        target: &'static str,
    },
    SetScale {
        factor: f32,
    },
    SetBody {
        body: &'static str,
    },
    ApplyImpulse {
        impulse: [f32; 3],
    },
    SetVelocity {
        velocity: [f32; 3],
    },
    SetGravity {
        gravity: [f32; 3],
    },
    SetDensity {
        density: f32,
    },
    SetMass {
        mass: f32,
    },
    Say {
        text: String,
    },
    SetVisible {
        visible: bool,
    },
    SetColor {
        color: String,
    },
    SetComponentField {
        component: &'static str,
        field: &'static str,
        value: Val,
    },
    SetCameraView {
        view: &'static str,
    },
    AttachComponent {
        component: &'static str,
    },
    DetachComponent {
        component: &'static str,
    },
    Broadcast {
        name: &'static str,
    },
}

/// One compiled strand, and what starts it. The trigger travels as text for
/// the same reason [`Act`]'s names do.
pub struct Entry {
    pub actor: &'static str,
    pub strand: &'static str,
    /// `Started`, `Key`, `Clicked`, `Collision` or `Message`.
    pub trigger: &'static str,
    /// The key, the other actor or the message name; empty for the rest.
    pub detail: &'static str,
    /// Where this strand starts, in the same step numbering the VM uses.
    pub start: usize,
    /// How many `repeat` counters one run of it needs.
    pub counters: usize,
    pub run: fn(&mut dyn Host, &mut State),
}

impl Entry {
    /// A fresh run of this strand, ready for the first [`Entry::run`].
    pub fn begin(&self) -> State {
        State::new(self.start, self.counters)
    }
}

/// Where a suspended strand is. The same three the VM's scripts have, so a
/// `wait` means the same thing on both sides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Run,
    /// Asleep until this timestamp, in run seconds.
    Sleep(f64),
    Done,
}

/// How deeply reporter blocks may call each other, and how many steps one
/// reporter body may take before it is given up on. Both are the VM's own
/// limits; `codegen`'s tests hold these against them.
pub const MAX_REPORTER_DEPTH: usize = 32;
pub const STEP_BUDGET: usize = 10_000;

/// One custom block being run as a statement: where to carry on afterwards,
/// and what its inputs were bound to.
pub struct CallFrame {
    return_pc: usize,
    params: Vec<Val>,
}

/// One run of one strand: where it is, and what it was in the middle of.
///
/// A compiled strand is a `match` over the program counter rather than a
/// recursive walk, for the same reason the VM flattens its blocks - a nested
/// body can't be suspended, but a number can be put down and picked up.
pub struct State {
    pub pc: usize,
    pub status: Status,
    /// Seconds since the run started. The scheduler sets it before each call.
    pub now: f64,
    /// `stop all` was reached, and the scheduler should end the whole run.
    pub stopping: bool,
    /// Iterations left, one slot per `repeat` in the actor. Loop nesting is
    /// known when the code is emitted, so this is a flat array rather than
    /// the VM's frame stack. A custom block that could call itself is refused
    /// at compile time, which is what makes one slot per loop enough.
    counters: Vec<i64>,
    /// Custom blocks entered as statements, innermost last.
    calls: Vec<CallFrame>,
}

impl State {
    /// A return address that means "hand the value back to whoever asked"
    /// rather than "carry on at this step": a reporter body's own boundary.
    pub const RETURN: usize = usize::MAX;

    pub fn new(start: usize, counters: usize) -> Self {
        Self {
            pc: start,
            status: Status::Run,
            now: 0.0,
            stopping: false,
            counters: vec![0; counters],
            calls: Vec::new(),
        }
    }

    pub fn enter_call(&mut self, return_pc: usize, params: Vec<Val>) {
        self.calls.push(CallFrame { return_pc, params });
    }

    /// Leaves the innermost custom block, answering the step to carry on at.
    /// `None` ends the run instead: either nothing called this, or what did
    /// was a reporter waiting on the value.
    pub fn resume_at(&mut self) -> Option<usize> {
        match self.calls.pop() {
            Some(frame) if frame.return_pc != Self::RETURN => Some(frame.return_pc),
            _ => None,
        }
    }

    /// One of the innermost call's bound inputs. Anything a caller didn't
    /// pass reads as zero, as an unbound name does in the VM.
    pub fn param(&self, index: usize) -> Val {
        self.calls
            .last()
            .and_then(|frame| frame.params.get(index))
            .cloned()
            .unwrap_or(Val::Num(0.0))
    }

    /// Whether there is anything to do this frame. A sleeping strand wakes on
    /// the first frame at or past its deadline, as the VM's does.
    pub fn resume(&mut self) -> bool {
        match self.status {
            Status::Done => false,
            Status::Sleep(until) => {
                if self.now < until {
                    return false;
                }
                self.status = Status::Run;
                true
            }
            Status::Run => true,
        }
    }

    pub fn done(&self) -> bool {
        self.status == Status::Done
    }

    pub fn sleep(&mut self, seconds: f64) {
        self.status = Status::Sleep(self.now + seconds);
    }

    pub fn finish(&mut self) {
        self.status = Status::Done;
    }

    /// Enters a `repeat` with `n` iterations to go.
    pub fn enter_repeat(&mut self, loop_index: usize, n: i64) {
        self.counters[loop_index] = n;
    }

    /// Counts one iteration off, and answers how many are left.
    pub fn next_iteration(&mut self, loop_index: usize) -> i64 {
        self.counters[loop_index] -= 1;
        self.counters[loop_index]
    }
}

/// Everything a generated program can reach outside itself. Five methods
/// rather than one per verb: a new block is a new [`Act`] or a new `kind`
/// string, never a change to this trait, which is what keeps a program built
/// against an older Blockloom loadable by a newer one.
pub trait Host {
    fn act(&mut self, actor: &str, act: Act);
    /// A sensing reporter, by the name its operator is registered under.
    /// Arguments arrive evaluated, and an error comes back as one, the way
    /// the operator itself would answer.
    fn sense(&mut self, actor: &str, kind: &str, args: &[Val]) -> R;
    fn variable(&mut self, actor: &str, name: &str) -> Val;
    fn set_variable(&mut self, actor: &str, name: &str, value: Val);
    /// A value that wouldn't evaluate. The program carries on with a zero.
    fn error(&mut self, actor: &str, message: &str);
}

// ─── Reading a value at an instruction's slot ───────────────────────────────
// The VM reports a bad slot once and stands a zero in its place; a slot that
// evaluated fine but isn't a number is simply zero, with nothing reported.
// Both halves of that matter, so each has a function here.

pub fn number(host: &mut dyn Host, actor: &str, value: R) -> f32 {
    number_f64(host, actor, value) as f32
}

pub fn number_f64(host: &mut dyn Host, actor: &str, value: R) -> f64 {
    match value {
        Ok(value) => value.as_number().unwrap_or(0.0),
        Err(message) => {
            host.error(actor, &message);
            0.0
        }
    }
}

/// A reporter block nested too deeply to run. The complaint is the VM's, word
/// for word, and the call reads as zero.
pub fn too_deep(host: &mut dyn Host, actor: &str) -> Val {
    host.error(actor, "a reporter block calls itself too deeply");
    Val::Num(0.0)
}

pub fn text(host: &mut dyn Host, actor: &str, value: R) -> String {
    evaluated(host, actor, value).as_text()
}

/// A condition slot: an `if`, a `while`, a `wait until`. A bad one reports
/// itself and reads as false, since a zero isn't true.
pub fn boolean(host: &mut dyn Host, actor: &str, value: R) -> bool {
    evaluated(host, actor, value).as_bool()
}

pub fn evaluated(host: &mut dyn Host, actor: &str, value: R) -> Val {
    match value {
        Ok(value) => value,
        Err(message) => {
            host.error(actor, &message);
            Val::Num(0.0)
        }
    }
}

/// A sensing reporter, with its arguments evaluated left to right first - an
/// extension operator can't short-circuit, so a bad argument is the answer.
pub fn sense(host: &mut dyn Host, actor: &str, kind: &str, args: Vec<R>) -> R {
    let mut evaluated = Vec::with_capacity(args.len());
    for arg in args {
        evaluated.push(arg?);
    }
    host.sense(actor, kind, &evaluated)
}

// ─── Operators ──────────────────────────────────────────────────────────────

pub fn add(a: R, b: R) -> R {
    Ok(Val::Num(a?.as_number()? + b?.as_number()?))
}

pub fn sub(a: R, b: R) -> R {
    Ok(Val::Num(a?.as_number()? - b?.as_number()?))
}

pub fn mul(a: R, b: R) -> R {
    Ok(Val::Num(a?.as_number()? * b?.as_number()?))
}

pub fn div(a: R, b: R) -> R {
    let (l, r) = (a?.as_number()?, b?.as_number()?);
    if r == 0.0 {
        return Err("division by zero".to_string());
    }
    Ok(Val::Num(l / r))
}

pub fn modulo(a: R, b: R) -> R {
    let (l, r) = (a?.as_number()?, b?.as_number()?);
    if r == 0.0 {
        return Err("mod by zero".to_string());
    }
    Ok(Val::Num(l.rem_euclid(r)))
}

pub fn round(a: R) -> R {
    Ok(Val::Num(a?.as_number()?.round()))
}

pub fn math(function: R, a: R) -> R {
    let function = function?.as_text();
    let n = a?.as_number()?;
    let to_radians = std::f64::consts::PI / 180.0;
    let to_degrees = 180.0 / std::f64::consts::PI;
    let result = match function.as_str() {
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
    Ok(Val::Num(result))
}

pub fn join(parts: Vec<R>) -> R {
    let mut joined = String::new();
    for part in parts {
        joined.push_str(&part?.as_text());
    }
    Ok(Val::Text(joined))
}

pub fn length(a: R) -> R {
    Ok(Val::Num(a?.as_text().chars().count() as f64))
}

pub fn index_of(needle: R, haystack: R) -> R {
    let needle = needle?.as_text();
    let haystack = haystack?.as_text();
    Ok(Val::Num(char_index_of(&haystack, &needle) as f64))
}

pub fn last_index_of(needle: R, haystack: R) -> R {
    let needle = needle?.as_text();
    let haystack = haystack?.as_text();
    Ok(Val::Num(char_last_index_of(&haystack, &needle) as f64))
}

pub fn letter_of(index: R, a: R) -> R {
    let index = index?.as_number()? as i64;
    let text = a?.as_text();
    let chars: Vec<char> = text.chars().collect();
    if index < 1 || index as usize > chars.len() {
        return Err(format!(
            "letter {index} is out of range for a {}-character value",
            chars.len()
        ));
    }
    Ok(Val::Text(chars[index as usize - 1].to_string()))
}

pub fn case(a: R, which: R) -> R {
    let text = a?.as_text();
    let upper = which?.as_text() == "Upper";
    Ok(Val::Text(if upper {
        text.to_uppercase()
    } else {
        text.to_lowercase()
    }))
}

pub fn eq(a: R, b: R) -> R {
    Ok(Val::Bool(values_equal(&a?, &b?)))
}

pub fn neq(a: R, b: R) -> R {
    Ok(Val::Bool(!values_equal(&a?, &b?)))
}

pub fn gt(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? > b?.as_number()?))
}

pub fn lt(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? < b?.as_number()?))
}

pub fn gte(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? >= b?.as_number()?))
}

pub fn lte(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? <= b?.as_number()?))
}

/// `and` and `or` take the second operand unevaluated, because the VM's
/// short-circuit is observable: `false and <a bad slot>` reports nothing.
pub fn and(a: R, b: impl FnOnce() -> R) -> R {
    if !a?.as_bool() {
        return Ok(Val::Bool(false));
    }
    Ok(Val::Bool(b()?.as_bool()))
}

pub fn or(a: R, b: impl FnOnce() -> R) -> R {
    if a?.as_bool() {
        return Ok(Val::Bool(true));
    }
    Ok(Val::Bool(b()?.as_bool()))
}

pub fn not(a: R) -> R {
    Ok(Val::Bool(!a?.as_bool()))
}

/// `Op::Eq`'s rule: numeric when both sides parse as numbers, text otherwise.
fn values_equal(l: &Val, r: &Val) -> bool {
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
