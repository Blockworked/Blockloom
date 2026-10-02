//! Deciding whether a plugin's kernel may run.
//!
//! A kernel is plain WGSL. It passes when naga validates it with no optional
//! capabilities, it has the one compute entry point the schema names with the
//! schema's workgroup size, every resource it declares is a buffer in group 0
//! that the schema lists with the same access, its workgroup memory is within
//! the cap, and every loop is provably bounded: a `for` over a local counter
//! that starts at a constant, is compared with a constant and steps by a
//! constant, with the total of all such iterations within
//! [`MAX_STATIC_COST`]. Out-of-range buffer accesses are clamped by the
//! device, so those loops are the one way a kernel can hang the GPU.

use blockloom_plugin_api::compute::{
    BindingKind, KernelSchema, MAX_STATIC_COST, MAX_WORKGROUP_BYTES,
};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{
    AddressSpace, BinaryOperator, Block, Expression, Function, Handle, Literal, LocalVariable,
    Module, ShaderStage, Statement, StorageAccess,
};
use std::collections::{BTreeMap, BTreeSet};

/// What checking a kernel learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelInfo {
    /// Loop iterations one invocation may run, at most.
    pub cost: u64,
    /// The smallest buffer each binding accepts, in bytes (a runtime-sized
    /// array counts one element), keyed by binding number.
    pub min_sizes: BTreeMap<u32, u64>,
}

/// Checks `source` against `schema`.
pub fn check_kernel(schema: &KernelSchema, source: &str) -> Result<KernelInfo, String> {
    let name = &schema.name;
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| format!("kernel {name}: {}", e.emit_to_string(source)))?;
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|e| format!("kernel {name}: {}", e.emit_to_string(source)))?;

    let entry = module
        .entry_points
        .iter()
        .find(|e| e.name == schema.entry)
        .ok_or_else(|| format!("kernel {name}: no entry point {}", schema.entry))?;
    if entry.stage != ShaderStage::Compute {
        return Err(format!(
            "kernel {name}: {} is not a compute entry point",
            schema.entry
        ));
    }
    if entry.workgroup_size_overrides.is_some() {
        return Err(format!("kernel {name}: a workgroup size is not constant"));
    }
    if entry.workgroup_size != schema.workgroup_size {
        return Err(format!(
            "kernel {name}: the schema says workgroup size {:?}, the entry point {:?}",
            schema.workgroup_size, entry.workgroup_size
        ));
    }

    let min_sizes = check_resources(schema, &module)?;

    let mut walk = Walk {
        module: &module,
        costs: BTreeMap::new(),
    };
    let cost = walk
        .function_body(&entry.function)
        .map_err(|e| format!("kernel {name}: {e}"))?;
    if cost > MAX_STATIC_COST {
        return Err(format!(
            "kernel {name}: loops may run {cost} iterations, at most {MAX_STATIC_COST}"
        ));
    }
    Ok(KernelInfo { cost, min_sizes })
}

fn check_resources(schema: &KernelSchema, module: &Module) -> Result<BTreeMap<u32, u64>, String> {
    let name = &schema.name;
    let mut bound = BTreeMap::new();
    let mut workgroup = 0u64;
    for (_, global) in module.global_variables.iter() {
        let label = global.name.as_deref().unwrap_or("a variable");
        match global.space {
            AddressSpace::Private | AddressSpace::Function => {}
            AddressSpace::WorkGroup => {
                workgroup += u64::from(module.types[global.ty].inner.size(module.to_ctx()));
            }
            AddressSpace::Uniform | AddressSpace::Storage { .. } => {
                let Some(binding) = &global.binding else {
                    return Err(format!("kernel {name}: {label} has no binding"));
                };
                if binding.group != 0 {
                    return Err(format!(
                        "kernel {name}: {label} is in group {}, only group 0 exists",
                        binding.group
                    ));
                }
                let Some(declared) = schema
                    .bindings
                    .iter()
                    .find(|b| b.binding == binding.binding)
                else {
                    return Err(format!(
                        "kernel {name}: {label} is binding {}, which the schema does not list",
                        binding.binding
                    ));
                };
                let kind = match global.space {
                    AddressSpace::Uniform => BindingKind::Uniform,
                    AddressSpace::Storage { access } if access.contains(StorageAccess::STORE) => {
                        BindingKind::ReadWrite
                    }
                    _ => BindingKind::Read,
                };
                if kind != declared.kind {
                    return Err(format!(
                        "kernel {name}: {label} is {kind:?}, the schema says {:?} for {}",
                        declared.kind, declared.name
                    ));
                }
                let size = module.types[global.ty].inner.size(module.to_ctx());
                bound.insert(binding.binding, u64::from(size));
            }
            _ => {
                return Err(format!(
                    "kernel {name}: {label} is a texture, sampler or other resource; \
                     a kernel only has buffers"
                ));
            }
        }
    }
    if workgroup > u64::from(MAX_WORKGROUP_BYTES) {
        return Err(format!(
            "kernel {name}: {workgroup} bytes of workgroup memory, at most {MAX_WORKGROUP_BYTES}"
        ));
    }
    if let Some(missing) = schema
        .bindings
        .iter()
        .find(|b| !bound.contains_key(&b.binding))
    {
        return Err(format!(
            "kernel {name}: the schema lists {} at binding {}, which the source does not declare",
            missing.name, missing.binding
        ));
    }
    Ok(bound)
}

/// The constants of known locals at a point in a block.
type Known = BTreeMap<Handle<LocalVariable>, i64>;

struct Walk<'a> {
    module: &'a Module,
    costs: BTreeMap<Handle<Function>, u64>,
}

impl Walk<'_> {
    /// The loop iterations a block may run, at most, saturating.
    fn block(&mut self, body: &Block, f: &Function, known: &mut Known) -> Result<u64, String> {
        let mut cost = 0u64;
        for statement in body {
            match statement {
                Statement::Store { pointer, value } => {
                    if let Expression::LocalVariable(v) = f.expressions[*pointer] {
                        match self.constant(f, *value) {
                            Some((n, _)) => known.insert(v, n),
                            None => known.remove(&v),
                        };
                    }
                }
                Statement::Block(inner) => {
                    cost = cost.saturating_add(self.block(inner, f, known)?);
                }
                Statement::If { accept, reject, .. } => {
                    let a = self.block(accept, f, &mut known.clone())?;
                    let r = self.block(reject, f, &mut known.clone())?;
                    cost = cost.saturating_add(a).saturating_add(r);
                    forget_stored(f, accept, known);
                    forget_stored(f, reject, known);
                }
                Statement::Switch { cases, .. } => {
                    for case in cases {
                        let c = self.block(&case.body, f, &mut known.clone())?;
                        cost = cost.saturating_add(c);
                        forget_stored(f, &case.body, known);
                    }
                }
                Statement::Loop {
                    body: inner,
                    continuing,
                    break_if,
                } => {
                    let trips = self.trips(f, inner, continuing, break_if.is_some(), known)?;
                    let mut inside = known.clone();
                    forget_stored(f, inner, &mut inside);
                    forget_stored(f, continuing, &mut inside);
                    let per = self
                        .block(inner, f, &mut inside.clone())?
                        .saturating_add(self.block(continuing, f, &mut inside)?);
                    cost = cost.saturating_add(trips.saturating_mul(per.saturating_add(1)));
                    forget_stored(f, inner, known);
                    forget_stored(f, continuing, known);
                }
                Statement::Call { function, .. } => {
                    let callee = self.function(*function)?;
                    cost = cost.saturating_add(callee);
                }
                _ => {}
            }
        }
        Ok(cost)
    }

    fn function(&mut self, handle: Handle<Function>) -> Result<u64, String> {
        if let Some(cost) = self.costs.get(&handle) {
            return Ok(*cost);
        }
        let module = self.module;
        let cost = self.function_body(&module.functions[handle])?;
        self.costs.insert(handle, cost);
        Ok(cost)
    }

    /// A function's cost, starting from the constants its locals begin with.
    fn function_body(&mut self, f: &Function) -> Result<u64, String> {
        let mut known = Known::new();
        for (handle, local) in f.local_variables.iter() {
            if let Some(n) = local.init.and_then(|init| self.constant(f, init)) {
                known.insert(handle, n.0);
            }
        }
        self.block(&f.body, f, &mut known)
    }

    /// An integer literal (or a constant naming one) and whether it is signed.
    fn constant(&self, f: &Function, expr: Handle<Expression>) -> Option<(i64, bool)> {
        match &f.expressions[expr] {
            Expression::Literal(l) => integer(*l),
            Expression::Constant(c) => self.global_constant(self.module.constants[*c].init),
            _ => None,
        }
    }

    /// The same for an expression in the module's own arena.
    fn global_constant(&self, expr: Handle<Expression>) -> Option<(i64, bool)> {
        match &self.module.global_expressions[expr] {
            Expression::Literal(l) => integer(*l),
            Expression::Constant(c) => self.global_constant(self.module.constants[*c].init),
            _ => None,
        }
    }

    /// How many times a `for`-shaped loop runs, or why its bound can't be
    /// shown.
    fn trips(
        &self,
        f: &Function,
        body: &Block,
        continuing: &Block,
        has_break_if: bool,
        known: &Known,
    ) -> Result<u64, String> {
        const HELP: &str = "a loop must be `for (var i = 0u; i < N; i++)` with constant bounds";
        if has_break_if {
            return Err(format!("an unbounded loop ({HELP})"));
        }
        let first = body.iter().position(|s| !matches!(s, Statement::Emit(_)));
        let Some(Statement::If {
            condition,
            accept,
            reject,
        }) = first.map(|i| &body[i])
        else {
            return Err(format!("an unbounded loop ({HELP})"));
        };
        let exits = accept.is_empty() && matches!(&reject[..], [Statement::Break]);
        let Expression::Binary { op, left, right } = f.expressions[*condition] else {
            return Err(format!("a loop condition is not a comparison ({HELP})"));
        };
        let (Some(var), true) = (self.counter(f, left), exits) else {
            return Err(format!("a loop has no counter in its condition ({HELP})"));
        };
        let (limit, signed) = self
            .constant(f, right)
            .ok_or_else(|| format!("a loop's limit is not a constant ({HELP})"))?;
        let start = *known
            .get(&var)
            .ok_or_else(|| format!("a loop's counter does not start at a constant ({HELP})"))?;

        // The only write to the counter is the step in `continuing`.
        let mut step = None;
        for statement in continuing {
            match statement {
                Statement::Emit(_) => {}
                Statement::Store { pointer, value }
                    if f.expressions[*pointer] == Expression::LocalVariable(var) =>
                {
                    let Expression::Binary {
                        op: step_op,
                        left: l,
                        right: r,
                    } = f.expressions[*value]
                    else {
                        return Err(format!("a loop's step is not `i += c` ({HELP})"));
                    };
                    let (by, _) =
                        self.constant(f, r)
                            .filter(|(by, _)| *by > 0)
                            .ok_or_else(|| {
                                format!("a loop's step is not a positive constant ({HELP})")
                            })?;
                    if self.counter(f, l) != Some(var) {
                        return Err(format!("a loop's step is not `i += c` ({HELP})"));
                    }
                    step = Some((step_op, by));
                }
                _ => return Err(format!("a loop's step does more than count ({HELP})")),
            }
        }
        let (step_op, by) = step.ok_or_else(|| format!("a loop never steps ({HELP})"))?;
        let rest = &body[first.unwrap_or(0) + 1..];
        let mut written = BTreeSet::new();
        stored(f, rest, &mut written);
        if written.contains(&var) || writes_through_call(f, rest, var) {
            return Err(format!("a loop's body writes its own counter ({HELP})"));
        }

        let (min, max) = if signed {
            (i64::from(i32::MIN), i64::from(i32::MAX))
        } else {
            (0, i64::from(u32::MAX))
        };
        let count = match (op, step_op) {
            (BinaryOperator::Less | BinaryOperator::LessEqual, BinaryOperator::Add) => {
                let end = limit + i64::from(op == BinaryOperator::LessEqual);
                if end + by > max + 1 {
                    return Err(format!("a loop's counter can wrap ({HELP})"));
                }
                span(end - start, by)
            }
            (BinaryOperator::Greater | BinaryOperator::GreaterEqual, BinaryOperator::Subtract) => {
                let end = limit - i64::from(op == BinaryOperator::GreaterEqual);
                if end - by < min - 1 {
                    return Err(format!("a loop's counter can wrap ({HELP})"));
                }
                span(start - end, by)
            }
            _ => {
                return Err(format!(
                    "a loop must count towards its limit: `<`/`<=` with `+`, or `>`/`>=` with `-` ({HELP})"
                ));
            }
        };
        if count > MAX_STATIC_COST {
            return Err(format!(
                "a loop runs {count} times, at most {MAX_STATIC_COST}"
            ));
        }
        Ok(count)
    }

    /// The local a `load` expression reads.
    fn counter(&self, f: &Function, expr: Handle<Expression>) -> Option<Handle<LocalVariable>> {
        let Expression::Load { pointer } = f.expressions[expr] else {
            return None;
        };
        match f.expressions[pointer] {
            Expression::LocalVariable(v) => Some(v),
            _ => None,
        }
    }
}

fn integer(literal: Literal) -> Option<(i64, bool)> {
    match literal {
        Literal::U32(n) => Some((i64::from(n), false)),
        Literal::I32(n) => Some((i64::from(n), true)),
        Literal::AbstractInt(n) => Some((n, true)),
        _ => None,
    }
}

/// How many steps of `by` fit in `distance`, rounding up; none if negative.
fn span(distance: i64, by: i64) -> u64 {
    if distance <= 0 {
        0
    } else {
        ((distance + by - 1) / by) as u64
    }
}

/// Every local a block stores to, nested blocks included.
fn stored(f: &Function, body: &[Statement], out: &mut BTreeSet<Handle<LocalVariable>>) {
    for statement in body {
        match statement {
            Statement::Store { pointer, .. } => {
                if let Expression::LocalVariable(v) = f.expressions[*pointer] {
                    out.insert(v);
                }
            }
            Statement::Block(inner) => stored(f, inner, out),
            Statement::If { accept, reject, .. } => {
                stored(f, accept, out);
                stored(f, reject, out);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    stored(f, &case.body, out);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                stored(f, body, out);
                stored(f, continuing, out);
            }
            _ => {}
        }
    }
}

fn forget_stored(f: &Function, body: &[Statement], known: &mut Known) {
    let mut written = BTreeSet::new();
    stored(f, body, &mut written);
    for v in written {
        known.remove(&v);
    }
}

/// Whether a call in the block is handed a pointer to `var`, which the
/// callee could write through.
fn writes_through_call(f: &Function, body: &[Statement], var: Handle<LocalVariable>) -> bool {
    body.iter().any(|statement| match statement {
        Statement::Call { arguments, .. } => arguments
            .iter()
            .any(|a| f.expressions[*a] == Expression::LocalVariable(var)),
        Statement::Block(inner) => writes_through_call(f, inner, var),
        Statement::If { accept, reject, .. } => {
            writes_through_call(f, accept, var) || writes_through_call(f, reject, var)
        }
        Statement::Switch { cases, .. } => {
            cases.iter().any(|c| writes_through_call(f, &c.body, var))
        }
        Statement::Loop {
            body, continuing, ..
        } => writes_through_call(f, body, var) || writes_through_call(f, continuing, var),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_plugin_api::compute::KernelBinding;

    fn schema(bindings: &[(&str, u32, BindingKind)]) -> KernelSchema {
        KernelSchema {
            name: "k".to_string(),
            file: "k.wgsl".to_string(),
            entry: "main".to_string(),
            workgroup_size: [64, 1, 1],
            bindings: bindings
                .iter()
                .map(|(name, binding, kind)| KernelBinding {
                    name: name.to_string(),
                    binding: *binding,
                    kind: *kind,
                })
                .collect(),
            description: String::new(),
        }
    }

    fn io() -> KernelSchema {
        schema(&[
            ("input", 0, BindingKind::Read),
            ("output", 1, BindingKind::ReadWrite),
        ])
    }

    const HEAD: &str = "
        @group(0) @binding(0) var<storage, read> input: array<f32>;
        @group(0) @binding(1) var<storage, read_write> output: array<f32>;
    ";

    fn kernel(body: &str) -> String {
        format!(
            "{HEAD}
            @compute @workgroup_size(64)
            fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
                {body}
            }}"
        )
    }

    fn check(body: &str) -> Result<KernelInfo, String> {
        check_kernel(&io(), &kernel(body))
    }

    #[test]
    fn a_plain_kernel_passes() {
        let info = check("output[id.x] = input[id.x] * 2.0;").unwrap();
        assert_eq!(info.cost, 0);
        // A runtime-sized array needs one element of each binding.
        assert_eq!(info.min_sizes, BTreeMap::from([(0, 4), (1, 4)]));
    }

    #[test]
    fn a_for_loop_with_constant_bounds_is_counted() {
        let info = check(
            "var sum = 0.0;
             for (var i = 0u; i < 16u; i++) { sum += input[id.x + i]; }
             output[id.x] = sum;",
        )
        .unwrap();
        assert_eq!(info.cost, 16);
        // Nested loops multiply, and a constant by name works.
        let nested = "
            const N = 8u;
            @group(0) @binding(0) var<storage, read> input: array<f32>;
            @group(0) @binding(1) var<storage, read_write> output: array<f32>;
            @compute @workgroup_size(64)
            fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                var s = 0.0;
                for (var i = 0u; i < N; i++) {
                    for (var j = 0u; j <= 3u; j += 2u) { s += input[i + j]; }
                }
                output[id.x] = s;
            }";
        // j runs twice (0 and 2) inside 8 trips of i: 8 * (1 + 2) + 2 * ... counted per body.
        let info = check_kernel(&io(), nested).unwrap();
        assert!(info.cost >= 16, "{info:?}");
    }

    #[test]
    fn a_counting_down_loop_is_bounded_too() {
        let info = check("for (var i = 10u; i > 0u; i--) { output[id.x] += 1.0; }").unwrap();
        assert_eq!(info.cost, 10);
    }

    #[test]
    fn an_unbounded_loop_is_refused() {
        for body in [
            "loop { if output[id.x] > 1.0 { break; } output[id.x] += 1.0; }",
            "var i = 0u; while (input[i] < 5.0) { i++; }",
            "for (var i = 0u; i < arrayLength(&input); i++) { output[i] = 1.0; }",
            "var n = 4u; for (var i = 0u; i < n; i++) { output[id.x] = 1.0; }",
            "for (var i = 0u; i < 4u; i++) { i = 0u; }",
            "for (var i = 0u; i < 4u; i--) { output[id.x] = 1.0; }",
            "for (var i = 0u; i < 4000000u; i++) { output[id.x] = 1.0; }",
            "for (var i = 4294967290u; i < 4294967295u; i += 3u) { output[id.x] = 1.0; }",
        ] {
            assert!(check(body).is_err(), "{body}");
        }
        let error = check("loop { }").unwrap_err();
        assert!(error.contains("for (var i = 0u"), "{error}");
    }

    #[test]
    fn a_loop_inside_a_function_counts_where_it_is_called() {
        let source = format!(
            "{HEAD}
            fn spin(x: f32) -> f32 {{
                var s = x;
                for (var i = 0u; i < 100u; i++) {{ s += 1.0; }}
                return s;
            }}
            @compute @workgroup_size(64)
            fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
                output[id.x] = spin(input[id.x]);
            }}"
        );
        assert_eq!(check_kernel(&io(), &source).unwrap().cost, 100);
    }

    #[test]
    fn the_entry_point_and_workgroup_size_must_match_the_schema() {
        let source = kernel("output[id.x] = 1.0;");
        let mut wrong = io();
        wrong.workgroup_size = [32, 1, 1];
        assert!(
            check_kernel(&wrong, &source)
                .unwrap_err()
                .contains("workgroup size")
        );
        let mut wrong = io();
        wrong.entry = "run".to_string();
        assert!(
            check_kernel(&wrong, &source)
                .unwrap_err()
                .contains("no entry point")
        );
        let fragment = format!(
            "{HEAD} @fragment fn main() -> @location(0) vec4<f32> {{ return vec4<f32>(1.0); }}"
        );
        assert!(
            check_kernel(&io(), &fragment)
                .unwrap_err()
                .contains("not a compute")
        );
    }

    #[test]
    fn bindings_must_match_the_schema_exactly() {
        let source = kernel("output[id.x] = input[id.x];");
        // Wrong access.
        let wrong = schema(&[
            ("input", 0, BindingKind::ReadWrite),
            ("output", 1, BindingKind::ReadWrite),
        ]);
        assert!(
            check_kernel(&wrong, &source)
                .unwrap_err()
                .contains("schema says")
        );
        // A binding the source never declares.
        let extra = schema(&[
            ("input", 0, BindingKind::Read),
            ("output", 1, BindingKind::ReadWrite),
            ("more", 2, BindingKind::Read),
        ]);
        assert!(
            check_kernel(&extra, &source)
                .unwrap_err()
                .contains("does not declare")
        );
        // A binding the schema never lists.
        let fewer = schema(&[("output", 1, BindingKind::ReadWrite)]);
        assert!(
            check_kernel(&fewer, &source)
                .unwrap_err()
                .contains("does not list")
        );
        // Another group.
        let grouped = source.replace("@group(0) @binding(1)", "@group(1) @binding(1)");
        assert!(
            check_kernel(&io(), &grouped)
                .unwrap_err()
                .contains("group 1")
        );
    }

    #[test]
    fn textures_and_bad_source_are_refused() {
        let texture = format!(
            "@group(0) @binding(2) var t: texture_2d<f32>;
             {HEAD}
             @compute @workgroup_size(64) fn main() {{ }}"
        );
        assert!(check_kernel(&io(), &texture).is_err());
        let error = check_kernel(&io(), "fn main( {").unwrap_err();
        assert!(error.starts_with("kernel k:"), "{error}");
    }

    #[test]
    fn workgroup_memory_is_capped() {
        let source = format!(
            "{HEAD}
            var<workgroup> tile: array<f32, 8192>;
            @compute @workgroup_size(64)
            fn main(@builtin(local_invocation_index) i: u32) {{ tile[i] = 1.0; }}"
        );
        assert!(
            check_kernel(&io(), &source)
                .unwrap_err()
                .contains("workgroup memory")
        );
    }
}
