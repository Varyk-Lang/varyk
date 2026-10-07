//! Async functions (milestone 5b1 spec 2.2, 2.3): where a call to one may
//! be made, what `.await` waits for, and the cycles of calls between them.

use varyk_syntax::{Expr, ExprKind, FixIt, Function, SourceFile, Span};

use super::{ArgRules, FnChecker};
use crate::borrow::returns::components;
use crate::builtins::BuiltinId;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirExpr, HirExprKind, LocalId, MethodRef};
use crate::resolve::{Callee, FnId, Symbols, UserType};
use crate::types::Ty;

/// A place where a task may be made (milestone 5b1 spec 2.3, 2.4),
/// recorded by the span of the expression standing there before it is
/// checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskPlace {
    /// The whole value of a `let` with a name: a started call, a `vec!` of
    /// them, or a collected `map` of them.
    Let,
    /// The receiver of `.detach()`: a started call or a task local.
    Detach,
    /// An element of `vec!`: a started call.
    VecElement,
    /// The whole value of the closure of a chain's last `map`, followed
    /// by `collect`: a started call.
    MapValue,
    /// The argument of `Task::all` or `Task::all_settled`: a `vec!` of
    /// started calls, a collected `map` of them, or a local holding one.
    All,
}

/// The type of a call returning `ret`: `Task<ret>` when it is started.
pub(super) fn started_ty(started: bool, ret: Ty) -> Ty {
    if started {
        Ty::Task(Box::new(ret))
    } else {
        ret
    }
}

/// The help of every V0213 for a single task.
const KEEP_THE_TASK: &str = "wait for it with `.await`, or let it run on its own with `.detach()`";

impl FnChecker<'_> {
    /// `operand.await` (at `span`): only in an async function (V0211) and
    /// not in a closure (V0211); `operand` must be a call of an async
    /// function or a local holding a task (V0212, or V0215 for a `Vec` of
    /// tasks); the value is what the call returns.
    pub(super) fn await_expr(
        &mut self,
        operand: &Expr,
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        let await_span = Span::new(span.file, operand.span.end, span.end);
        if !self.closures.is_empty() {
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0211,
                    await_span,
                    "`.await` cannot be used inside a closure",
                )
                .with_note(
                    "a closure is not an async function, so it cannot wait; wait for the value \
                     before the call that takes the closure",
                ),
            );
            return None;
        }
        let outer = self.await_operand.replace(operand.span);
        let outer_whole = self.await_whole.replace(span);
        let checked = self.expr(operand, expected);
        self.await_operand = outer;
        self.await_whole = outer_whole;
        let operand = checked?;
        if !self.is_async {
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0211,
                    await_span,
                    "`.await` waits for something, so only an `async fn` can use it",
                )
                .with_note(
                    "an ordinary function runs from start to end without stopping; make this \
                     function `async` so it can wait",
                )
                .with_fix_it(self.async_fix_it.clone()),
            );
            return None;
        }
        if self.is_async_call(&operand) {
            return Some(HirExpr {
                ty: operand.ty.clone(),
                kind: HirExprKind::Await(Box::new(operand)),
                span,
            });
        }
        if let (HirExprKind::Local(_), Ty::Task(result)) = (&operand.kind, &operand.ty) {
            return Some(HirExpr {
                ty: (**result).clone(),
                kind: HirExprKind::Await(Box::new(operand)),
                span,
            });
        }
        let message = match self.called_name(&operand) {
            Some(name) => {
                format!("`{name}` is not an async function, so there is nothing to wait for")
            }
            None => "only a call to an async function can be waited for with `.await`".to_string(),
        };
        self.diagnostics.push(
            Diagnostic::new(codes::V0212, await_span, message)
                .with_note("`.await` follows a call to an async function, as in `fetch(1).await`")
                .with_fix_it(FixIt {
                    span: await_span,
                    replacement: String::new(),
                }),
        );
        None
    }

    /// A call, at `span`, of the async function `name` (`callee` when it
    /// is a Varyk function; `None` for a `.rs` import or a built-in): only an async function may make it (V0211).
    /// It is awaited when it is the operand of the `.await` being checked;
    /// any other call is started, which gives `Some(true)`, and may only
    /// stand where its task is kept (V0213).
    pub(super) fn async_call(
        &mut self,
        callee: Option<FnId>,
        name: &str,
        span: Span,
    ) -> Option<bool> {
        if !self.is_async {
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0211,
                    span,
                    format!("`{name}` waits for something, so only an `async fn` can call it"),
                )
                .with_note(
                    "an ordinary function runs from start to end without stopping; make this \
                     function `async` so it can wait",
                )
                .with_fix_it(self.async_fix_it.clone()),
            );
            return None;
        }
        if let Some(id) = callee {
            self.async_callees.push((id, span));
        }
        if self.await_operand == Some(span) {
            return Some(false);
        }
        self.started_call(name, span)?;
        // The task runs on `varyk-std`'s runtime (milestone 5b1 spec 4).
        self.uses_std = true;
        Some(true)
    }

    /// `Task::all(tasks)` or `Task::all_settled(tasks)` (`id`), at `span`
    /// (milestone 5b1 spec 2.5): always awaited (V0212 otherwise), and
    /// given one `Vec` of tasks, which it takes (V0200 for anything else).
    /// `all` gives the tasks' values, or, when they give a `Result`, a
    /// `Result` of them; `all_settled` needs tasks giving a `Result`
    /// (V0200, naming `Task::all`) and gives every one.
    pub(super) fn task_all(&mut self, id: BuiltinId, args: &[Expr], span: Span) -> Option<HirExpr> {
        let full = id.path();
        let [arg] = args else {
            // Only the count is reported: the parameter's type is the
            // argument's.
            self.arguments(&full, &[Ty::Unit], &ArgRules::default(), args, span);
            return None;
        };
        let arg = self.expr_in(arg, None, TaskPlace::All);
        if self.await_operand != Some(span) {
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0212,
                    span,
                    format!("`{full}` waits for its tasks, so it must be followed by `.await`"),
                )
                .with_note(format!("write `{full}(tasks).await`"))
                .with_fix_it(FixIt {
                    span: Span::new(span.file, span.end, span.end),
                    replacement: ".await".to_string(),
                }),
            );
            return None;
        }
        let arg = arg?;
        let Some(result) = listed_result(&arg.ty).cloned() else {
            let message = format!(
                "`{full}` waits for a list of tasks, and this is `{}`",
                self.ty_name(&arg.ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, arg.span, message).with_note(
                    "a list of tasks is made by starting calls in `vec![..]`, as in \
                     `vec![fetch(1), fetch(2)]`, or in a collected `map`, as in \
                     `ids.iter().map(|id| fetch(id)).collect()`",
                ));
            return None;
        };
        let settled = id.get().name == "all_settled";
        let ty = match (&result, settled) {
            (Ty::Result(ok, err), false) => Ty::Result(Box::new(Ty::Vec(ok.clone())), err.clone()),
            (Ty::Result(..), true) | (_, false) => Ty::Vec(Box::new(result)),
            (_, true) => {
                let message = format!(
                    "`Task::all_settled` keeps each task's `Result`, and these tasks give `{}`, \
                     which is not a `Result`",
                    self.ty_name(&result)
                );
                self.diagnostics
                    .push(Diagnostic::new(codes::V0200, span, message).with_note(
                        "wait for tasks that cannot fail with `Task::all(tasks).await`",
                    ));
                return None;
            }
        };
        Some(HirExpr {
            kind: HirExprKind::Call {
                callee: Callee::Builtin(id),
                args: vec![arg],
                trailing: Vec::new(),
                type_arg: None,
                rooted: None,
                started: false,
            },
            ty,
            span,
        })
    }

    /// The place a task made at `span` stands in, if it is one of those of
    /// [`TaskPlace`].
    fn task_place(&self, span: Span) -> Option<TaskPlace> {
        self.task_places
            .iter()
            .rev()
            .find(|(at, _)| *at == span)
            .map(|(_, place)| *place)
    }

    /// Checks `expr` standing in `place`, a place where a task may be
    /// made, so that one made by `expr` itself is kept.
    pub(super) fn expr_in(
        &mut self,
        expr: &Expr,
        expected: Option<Ty>,
        place: TaskPlace,
    ) -> Option<HirExpr> {
        self.task_places.push((expr.span, place));
        let checked = self.expr(expr, expected);
        self.task_places.pop();
        checked
    }

    /// A started call of `name` at `span` (milestone 5b1 spec 2.3): kept
    /// in a named `let`, detached, an element of `vec!`, or the value of a
    /// collected `map`; anywhere else its task would be thrown away
    /// (V0213).
    fn started_call(&mut self, name: &str, span: Span) -> Option<()> {
        if self.task_place(span).is_some() {
            return Some(());
        }
        self.diagnostics.push(
            Diagnostic::new(
                codes::V0213,
                span,
                format!("this starts `{name}` and then throws its task away, which stops it"),
            )
            .with_note(KEEP_THE_TASK)
            .with_fix_it(FixIt {
                span: Span::new(span.file, span.end, span.end),
                replacement: ".await".to_string(),
            }),
        );
        None
    }

    /// `made`, a `Vec` of tasks just made by `vec!` or `collect`: kept in a
    /// named `let` or given to `Task::all` or `Task::all_settled` (spec
    /// 2.5); `.await` on it is V0215, and anywhere else its tasks would be
    /// thrown away (V0213).
    pub(super) fn tasks_made(&mut self, made: HirExpr) -> Option<HirExpr> {
        if listed_result(&made.ty).is_none()
            || matches!(
                self.task_place(made.span),
                Some(TaskPlace::Let | TaskPlace::All)
            )
        {
            return Some(made);
        }
        if self.await_operand == Some(made.span) {
            self.diagnostics.push(await_on_tasks(made.span));
            return None;
        }
        self.diagnostics.push(
            Diagnostic::new(
                codes::V0213,
                made.span,
                "this starts tasks and then throws them away, which stops them",
            )
            .with_note(KEEP_THE_TASKS),
        );
        None
    }

    /// `local`, used at `span`, when it holds a task or a `Vec` of tasks
    /// (milestone 5b1 spec 2.4): a task may only be awaited or detached,
    /// and a `Vec` of tasks only given to `Task::all` or `Task::all_settled`
    /// (spec 2.5); anything else is V0215.
    pub(super) fn task_local(&mut self, local: LocalId, ty: &Ty, span: Span) -> Option<()> {
        if !ty.is_tasks() {
            return Some(());
        }
        if !self.task_uses.contains(&local) {
            self.task_uses.push(local);
        }
        let awaited = self.await_operand == Some(span);
        let name = &self.locals[local.0 as usize].name;
        let diagnostic = match ty {
            Ty::Task(_) if awaited || self.task_place(span) == Some(TaskPlace::Detach) => {
                return Some(());
            }
            Ty::Task(_) => Diagnostic::new(
                codes::V0215,
                span,
                format!(
                    "`{name}` holds a task, which can only be waited for with `.await` or let \
                     go with `.detach()`"
                ),
            )
            .with_note(TASK_STAYS)
            .with_note(format!(
                "wait for it with `{name}.await` and use what it gives instead"
            )),
            _ if self.task_place(span) == Some(TaskPlace::All) => return Some(()),
            _ if awaited => await_on_tasks(span),
            _ => Diagnostic::new(
                codes::V0215,
                span,
                format!(
                    "`{name}` holds tasks, which can only be given to `Task::all` or \
                     `Task::all_settled`"
                ),
            )
            .with_note(TASK_STAYS)
            .with_note(format!(
                "wait for all of them with `Task::all({name}).await` and use what it gives \
                 instead"
            )),
        };
        self.diagnostics.push(diagnostic);
        None
    }

    /// V0213 for every local of the function holding a task, or a `Vec`
    /// of tasks, that nothing uses (milestone 5b1 spec 2.4): its tasks
    /// are stopped when it goes out of scope.
    pub(super) fn unused_tasks(&mut self) {
        for (index, info) in self.locals.iter().enumerate() {
            if !info.ty.is_tasks() || self.task_uses.contains(&LocalId(index as u32)) {
                continue;
            }
            let name = &info.name;
            let diagnostic = match info.ty {
                Ty::Task(_) => Diagnostic::new(
                    codes::V0213,
                    info.span,
                    format!(
                        "the task in `{name}` is never waited for or let go, so it is stopped \
                         as soon as `{name}` goes out of scope"
                    ),
                )
                .with_note(format!(
                    "wait for it with `{name}.await`, or let it run on its own with \
                     `{name}.detach()`"
                )),
                _ => Diagnostic::new(
                    codes::V0213,
                    info.span,
                    format!(
                        "the tasks in `{name}` are never waited for, so they are stopped as \
                         soon as `{name}` goes out of scope"
                    ),
                )
                .with_note(format!(
                    "wait for all of them with `Task::all({name}).await`"
                )),
            };
            self.diagnostics.push(diagnostic);
        }
    }

    /// Whether `expr` is a call of an async function.
    fn is_async_call(&self, expr: &HirExpr) -> bool {
        match &expr.kind {
            HirExprKind::Call {
                callee: Callee::Varyk(id),
                ..
            }
            | HirExprKind::MethodCall {
                method: MethodRef::Varyk(id),
                ..
            } => self.symbols.fns[id.0 as usize].is_async,
            HirExprKind::Call {
                callee: Callee::Imported(id),
                ..
            }
            | HirExprKind::MethodCall {
                method: MethodRef::Imported(id),
                ..
            } => self.symbols.imported[id.0 as usize].is_async,
            HirExprKind::Call {
                callee: Callee::Builtin(id),
                ..
            }
            | HirExprKind::MethodCall {
                method: MethodRef::Builtin(id),
                ..
            } => id.get().is_async,
            _ => false,
        }
    }

    /// The name of the function `expr` calls, as written in a message.
    fn called_name(&self, expr: &HirExpr) -> Option<String> {
        match &expr.kind {
            HirExprKind::Call {
                callee: Callee::Varyk(id),
                ..
            }
            | HirExprKind::MethodCall {
                method: MethodRef::Varyk(id),
                ..
            } => Some(fn_name(self.symbols, *id)),
            HirExprKind::Call {
                callee: Callee::Imported(id),
                ..
            }
            | HirExprKind::MethodCall {
                method: MethodRef::Imported(id),
                ..
            } => Some(self.symbols.imported[id.0 as usize].name.clone()),
            HirExprKind::Call {
                callee: Callee::Builtin(id),
                ..
            }
            | HirExprKind::MethodCall {
                method: MethodRef::Builtin(id),
                ..
            } => Some(id.path()),
            _ => None,
        }
    }
}

/// The task-place spans of `closure`'s value, the closure of the `map`
/// a `collect` is called on: its body, and the tail of a body block
/// with nothing else in it.
pub(super) fn map_value_spans(closure: &Expr) -> Vec<Span> {
    let ExprKind::Closure { body, .. } = &closure.kind else {
        return Vec::new();
    };
    let mut spans = vec![body.span];
    if let ExprKind::Block(block) = &body.kind {
        if let (true, Some(tail)) = (block.stmts.is_empty(), &block.tail) {
            spans.push(tail.span);
        }
    }
    spans
}

/// `T`, when `ty` is a `Vec` of tasks giving `T`.
fn listed_result(ty: &Ty) -> Option<&Ty> {
    match ty {
        Ty::Vec(item) => match &**item {
            Ty::Task(result) => Some(result),
            _ => None,
        },
        _ => None,
    }
}

/// Where a task may be used (milestone 5b1 spec 2.4), for the note of
/// a V0215.
const TASK_STAYS: &str = "a task stays where it is made: it cannot be stored, passed, \
                          returned, or looked inside; only the function that started it waits \
                          for it";

/// The help of a V0213 for a `Vec` of tasks.
const KEEP_THE_TASKS: &str =
    "keep them in a `let` and wait for all of them with `Task::all(tasks).await`";

/// V0215 at `span` for `.await` on a `Vec` of tasks.
fn await_on_tasks(span: Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0215,
        span,
        "a list of tasks cannot be waited for with `.await` itself",
    )
    .with_note("wait for every task in it with `Task::all(tasks).await`")
}

/// The fix-it making `decl` async: `async ` before its `fn`, after `pub`.
pub(super) fn async_fix_it(decl: &Function, sources: &[SourceFile]) -> FixIt {
    let span = decl.span;
    let mut at = span.start as usize;
    if decl.is_pub {
        let rest = sources
            .get(span.file.0 as usize)
            .and_then(|source| source.text.get(at + "pub".len()..))
            .unwrap_or_default();
        at += "pub".len() + rest.len() - rest.trim_start().len();
    }
    FixIt {
        span: Span::new(span.file, at as u32, at as u32),
        replacement: "async ".to_string(),
    }
}

/// V0214 for each group of async functions that call each other in a
/// cycle (milestone 5b1 spec 2.2): `calls[f]` is every call function `f`
/// makes to an async function, with its span. Reported once per group,
/// at the first call of the cycle's first function.
pub(super) fn async_cycles(symbols: &Symbols, calls: &[Vec<(FnId, Span)>]) -> Vec<Diagnostic> {
    let graph: Vec<Vec<usize>> = calls
        .iter()
        .map(|calls| {
            let mut callees: Vec<usize> = Vec::new();
            for (id, _) in calls {
                if !callees.contains(&(id.0 as usize)) {
                    callees.push(id.0 as usize);
                }
            }
            callees
        })
        .collect();
    let mut out = Vec::new();
    for component in components(&graph) {
        let start = component[0];
        let Some(cycle) = shortest_cycle(&graph, &component, start) else {
            continue;
        };
        let next = cycle.get(1).copied().unwrap_or(start);
        let Some(&(_, span)) = calls[start].iter().find(|(id, _)| id.0 as usize == next) else {
            continue;
        };
        let name = |node: usize| format!("`{}`", fn_name(symbols, FnId(node as u32)));
        let path = if cycle.len() == 1 {
            format!("{} calls itself", name(start))
        } else {
            let rest: Vec<String> = cycle[1..]
                .iter()
                .chain(std::iter::once(&start))
                .map(|&node| name(node))
                .collect();
            format!("{} calls {}", name(start), rest.join(", which calls "))
        };
        out.push(
            Diagnostic::new(
                codes::V0214,
                span,
                format!("async functions cannot call each other in a cycle: {path}"),
            )
            .with_note(
                "an async function's waiting is planned out before it runs, which cannot be \
                 done for one that waits on itself; make one of them an ordinary function, or \
                 use a loop",
            )
            .with_note("in Rust terms, the future would contain itself and have no fixed size"),
        );
    }
    out
}

/// The shortest cycle through `start` inside `component`, as the
/// functions on it from `start`; `None` when there is none (a single
/// function that does not call itself).
fn shortest_cycle(graph: &[Vec<usize>], component: &[usize], start: usize) -> Option<Vec<usize>> {
    // Breadth first from `start`, remembering how each node was reached.
    let mut came_from: Vec<Option<usize>> = vec![None; graph.len()];
    let mut queue = std::collections::VecDeque::from([start]);
    let mut seen = vec![false; graph.len()];
    seen[start] = true;
    while let Some(node) = queue.pop_front() {
        for &callee in &graph[node] {
            if !component.contains(&callee) {
                continue;
            }
            if callee == start {
                let mut cycle = vec![node];
                let mut current = node;
                while let Some(previous) = came_from[current] {
                    cycle.push(previous);
                    current = previous;
                }
                cycle.reverse();
                return Some(cycle);
            }
            if !seen[callee] {
                seen[callee] = true;
                came_from[callee] = Some(node);
                queue.push_back(callee);
            }
        }
    }
    None
}

/// Function `id`'s name as a message writes it: `Type::name` for one of
/// an `impl` block.
fn fn_name(symbols: &Symbols, id: FnId) -> String {
    let sig = &symbols.fns[id.0 as usize];
    match sig.owner {
        Some(UserType::Struct(owner)) => {
            format!("{}::{}", symbols.structs[owner.0 as usize].name, sig.name)
        }
        Some(UserType::Enum(owner)) => {
            format!("{}::{}", symbols.enums[owner.0 as usize].name, sig.name)
        }
        None => sig.name.clone(),
    }
}
