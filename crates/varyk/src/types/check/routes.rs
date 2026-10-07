//! The route and hook calls on a `varyk-http` app (milestone 5b4 spec 2.1
//! to 2.4): `app.get(path, f)` and its four siblings, and `app.before(f)`,
//! `app.before_on(prefix, f)`, and `app.after(f)`. Each is a statement of
//! its own, on a local bound to `App::new` in the same function and
//! outside any loop or closure (V0221), so the app's state type is known.
//! A route's path is a literal (V0217) of the spec's shape (V0222); its
//! handler is an async function of this package (V0220, V0105, V0106,
//! V0114), whose parameters each bind by one rule (V0219) and whose return
//! type is one a route can send (V0220). It is lowered to a [`HirRoute`].
//! A hook's function is chosen as a handler is, with a fixed signature
//! whose parameters come by position (V0220), and a `before_on` prefix is
//! a path with no `{name}` (V0222). It is lowered to a [`HirHook`].

use varyk_syntax::{Expr, ExprKind, Ident, Path, Span};

use super::FnChecker;
use super::values::PathOwner;
use crate::builtins::Owner;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    Binding, HirExpr, HirExprKind, HirHook, HirRoute, HirStmt, HookKind, HttpMethod, LocalId,
    ReturnShape, Segment, path_text,
};
use crate::resolve::{Callee, FnId, HttpItems, ModuleId, StructId, display_path};
use crate::types::derives::Direction;
use crate::types::{ParamMode, Ty};

/// A local bound by a `let` to `App::new(..)` (spec 2.1).
#[derive(Debug)]
pub(super) struct AppLocal {
    /// The struct the `Shared` given to `App::new` holds.
    state: Ty,
    /// The routes added to it so far, with where, for V0222 at the same
    /// one again.
    routes: Vec<(HttpMethod, Vec<Segment>, Span)>,
}

/// A statement's expression, or a block's tail, once checked: a route or
/// hook call is a statement of its own.
pub(super) enum Lowered {
    Stmt(Box<HirStmt>),
    Expr(HirExpr),
}

/// A call on an app that adds to it (spec 2.1).
#[derive(Debug, Clone, Copy)]
enum AppCall {
    Route(HttpMethod),
    /// A hook, `true` for `before_on`, which takes a prefix.
    Hook(HookKind, bool),
}

impl AppCall {
    /// The call `name` on an app, if it adds a route or a hook.
    fn of(name: &str) -> Option<AppCall> {
        match name {
            "before" => Some(AppCall::Hook(HookKind::Before, false)),
            "before_on" => Some(AppCall::Hook(HookKind::Before, true)),
            "after" => Some(AppCall::Hook(HookKind::After, false)),
            _ => HttpMethod::of_call(name).map(AppCall::Route),
        }
    }

    fn added(self) -> Added {
        match self {
            AppCall::Route(_) => Added::Route,
            AppCall::Hook(..) => Added::Hook,
        }
    }

    /// The call's arguments, as a note names them.
    fn args(self) -> &'static str {
        match self {
            AppCall::Route(_) => "path, f",
            AppCall::Hook(_, true) => "prefix, f",
            AppCall::Hook(_, false) => "f",
        }
    }
}

/// What a call adds to an app, for the words of its diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Added {
    Route,
    Hook,
}

impl Added {
    /// `route` or `hook`.
    fn word(self) -> &'static str {
        match self {
            Added::Route => "route",
            Added::Hook => "hook",
        }
    }

    /// What the function the call names is: `a route's handler` or `a
    /// hook`.
    fn role(self) -> &'static str {
        match self {
            Added::Route => "a route's handler",
            Added::Hook => "a hook",
        }
    }

    /// A call that adds one, without its `;`.
    fn example(self) -> &'static str {
        match self {
            Added::Route => "app.get(\"/\", home)",
            Added::Hook => "app.before(check)",
        }
    }
}

/// The note listing what a handler's parameter can be.
const PARAMETERS: &str = "a handler's parameter is a `{name}` of the path, the state as a \
                          `Shared`, a `Request`, the body on `post`, `put`, or `patch`, or a \
                          query parameter: an integer, `bool`, `string`, `Time`, `Uuid`, or an \
                          `Option` of one";

/// The segments of the route path or `before_on` prefix `text`, as written
/// between its quotes (spec 2.2), or why it is not one. A path is `/`
/// followed by segments separated by `/`, each a literal of ASCII
/// letters, digits, `-`, `_`, `.`, and `~`, or `{name}` with `name` an
/// identifier, every name distinct; no segment is empty, and only `"/"`
/// itself ends with `/`.
pub(super) fn parse_path(text: &str) -> Result<Vec<Segment>, String> {
    let Some(rest) = text.strip_prefix('/') else {
        return Err("a path starts with `/`, as in `\"/users\"`".to_string());
    };
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    let parts: Vec<&str> = rest.split('/').collect();
    let mut segments = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            let why = if index + 1 == parts.len() {
                "a path does not end with `/`, except `\"/\"` itself"
            } else {
                "a path has no empty segment between two `/`"
            };
            return Err(why.to_string());
        }
        if let Some(name) = part.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
            if !is_identifier(name) {
                return Err(format!(
                    "`{{{name}}}` holds no name: a name starts with a letter or `_` and holds \
                     letters, digits, and `_`"
                ));
            }
            let segment = Segment::Param(name.to_string());
            if segments.contains(&segment) {
                return Err(format!(
                    "`{{{name}}}` appears twice, and each name in a path is different"
                ));
            }
            segments.push(segment);
            continue;
        }
        let odd = part
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')));
        if let Some(c) = odd {
            return Err(format!(
                "`{c}` cannot be in a path: a segment holds letters, digits, `-`, `_`, `.`, \
                 and `~`, or is a `{{name}}`"
            ));
        }
        segments.push(Segment::Literal(part.to_string()));
    }
    Ok(segments)
}

/// Whether `name` is a Varyk identifier other than `_`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let first = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && name != "_"
}

/// Whether a path segment gives a value of `ty` (spec 2.2 rule 1; `Time`
/// and `Uuid`, milestone 5c spec 2.4): read through `FromStr` by the
/// package.
fn is_path_type(ty: &Ty) -> bool {
    matches!(ty, Ty::Int(_) | Ty::Bool | Ty::String | Ty::Time | Ty::Uuid)
}

/// Whether a value of `ty` comes from the query string (spec 2.2 rule 5;
/// `Time` and `Uuid`, milestone 5c spec 2.4).
fn is_query(ty: &Ty) -> bool {
    match ty {
        Ty::Option(inner) => is_path_type(inner),
        other => is_path_type(other),
    }
}

impl FnChecker<'_> {
    /// Records `local`, just bound to `value`, as an app local when
    /// `value` is `App::new(state)`.
    pub(super) fn bind_app(&mut self, local: LocalId, value: &HirExpr) {
        let HirExprKind::Call {
            callee: Callee::Builtin(id),
            args,
            ..
        } = &value.kind
        else {
            return;
        };
        if id.get().owner != Owner::App {
            return;
        }
        if let Some(Ty::Shared(state)) = args.first().map(|arg| &arg.ty) {
            let state = (**state).clone();
            let routes = Vec::new();
            self.apps.insert(local, AppLocal { state, routes });
        }
    }

    /// The state type of the app local `local`, the struct its `App::new`
    /// was given in a `Shared`; `None` when `local` is not an app local.
    pub(super) fn app_state(&self, local: LocalId) -> Option<Ty> {
        self.apps.get(&local).map(|app| app.state.clone())
    }

    /// V0221 when `target`, an assignment's, is an app local, or any local
    /// holding an app (a `mut` parameter too, which writes through to the
    /// caller's app): its routes were checked against the state it was
    /// made with (spec 2.1).
    pub(super) fn app_assigned(&mut self, target: &HirExpr) -> Option<()> {
        let HirExprKind::Local(local) = target.kind else {
            return Some(());
        };
        if !self.apps.contains_key(&local) && self.app_of(&target.ty).is_none() {
            return Some(());
        }
        let name = &self.locals[local.0 as usize].name;
        let diagnostic = Diagnostic::new(
            codes::V0221,
            target.span,
            format!("`{name}` holds an app, and cannot be given another value"),
        )
        .with_note(
            "the routes of an app are checked against the state it was made with; make another \
             app with a `let` of its own",
        );
        self.diagnostics.push(diagnostic);
        None
    }

    /// The `varyk-http` app struct `ty` is, through a `Shared` too.
    fn app_of(&self, ty: &Ty) -> Option<StructId> {
        match *ty.reached() {
            Ty::Struct(id) if self.symbols.is_app(id) => Some(id),
            _ => None,
        }
    }

    /// `expr`, a statement's expression or a block's tail: a route when it
    /// is a route call on an app, else checked as [`FnChecker::expr`]
    /// checks it.
    pub(super) fn expr_or_route(&mut self, expr: &Expr, expected: Option<Ty>) -> Option<Lowered> {
        let ExprKind::MethodCall {
            receiver,
            method,
            args,
        } = &expr.kind
        else {
            return self.expr(expr, expected).map(Lowered::Expr);
        };
        let Some(call) = AppCall::of(&method.name) else {
            return self.expr(expr, expected).map(Lowered::Expr);
        };
        let receiver = self.expr(receiver, None)?;
        let Some(app) = self.app_of(&receiver.ty) else {
            // Another value's own method, as `expr` checks it.
            let call = self.method_on(receiver, method, args, expected, expr.span)?;
            if let Some(diagnostic) = self.symbols.unlisted_package(&call.ty, call.span) {
                self.diagnostics.push(diagnostic);
                return None;
            }
            return Some(Lowered::Expr(call));
        };
        let stmt = match call {
            AppCall::Route(http) => self
                .route(app, &receiver, http, method, args, expr.span)
                .map(HirStmt::Route),
            AppCall::Hook(kind, on) => self
                .hook(app, &receiver, (kind, on), method, args, expr.span)
                .map(HirStmt::Hook),
        };
        stmt.map(|stmt| Lowered::Stmt(Box::new(stmt)))
    }

    /// V0221 for the route or hook call `receiver.method(..)` at `span`
    /// used as a value, not as a statement of its own; `None` for any
    /// other call.
    pub(super) fn route_as_value(
        &self,
        receiver: &HirExpr,
        method: &Ident,
        span: Span,
    ) -> Option<Diagnostic> {
        self.app_of(&receiver.ty)?;
        let call = AppCall::of(&method.name)?;
        let added = call.added();
        if !self.closures.is_empty() {
            return Some(in_a_closure(span, added));
        }
        Some(
            Diagnostic::new(
                codes::V0221,
                span,
                format!("a {} is added by a statement of its own", added.word()),
            )
            .with_note(format!(
                "write `{}.{}({});` on a line of its own; it gives no value",
                self.receiver_text(receiver),
                method.name,
                call.args()
            )),
        )
    }

    /// How a message names the receiver of a route or hook call.
    fn receiver_text(&self, receiver: &HirExpr) -> String {
        match receiver.kind {
            HirExprKind::Local(local) => self.locals[local.0 as usize].name.clone(),
            _ => "app".to_string(),
        }
    }

    /// The app local `receiver` is, for a call at `span` that adds a
    /// route or a hook to it: V0221 when it is not one, or when the call
    /// is in a closure, a loop, or a loop's condition, which may run it
    /// more than once (spec 2.1).
    pub(super) fn app_local(
        &mut self,
        receiver: &HirExpr,
        span: Span,
        added: Added,
    ) -> Option<LocalId> {
        let word = added.word();
        let local = match receiver.kind {
            HirExprKind::Local(local) if self.apps.contains_key(&local) => local,
            _ => {
                let name = self.receiver_text(receiver);
                let diagnostic = Diagnostic::new(
                    codes::V0221,
                    span,
                    format!(
                        "{word}s are added to an app made by `App::new` in the same function, \
                         and `{name}` is not one"
                    ),
                )
                .with_note(format!(
                    "make the app and add its {word}s in one function, as in `let mut app = \
                     http::App::new(state);` then `{};`; that function may then return the app",
                    added.example()
                ));
                self.diagnostics.push(diagnostic);
                return None;
            }
        };
        if !self.closures.is_empty() {
            self.diagnostics.push(in_a_closure(span, added));
            return None;
        }
        if self.loop_depth > 0 || self.loop_heads > 0 {
            let note = match added {
                Added::Route => {
                    "add each route once, outside the loop; a route added twice is refused"
                }
                Added::Hook => {
                    "add each hook once, outside the loop; a hook added twice runs twice"
                }
            };
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0221,
                    span,
                    format!(
                        "a {word} cannot be added inside a loop, which would add it each time \
                         around"
                    ),
                )
                .with_note(note),
            );
            return None;
        }
        Some(local)
    }

    /// `receiver.method(path, f)` at `span`, `receiver` of the app struct
    /// `app`: a route of `http` (spec 2.1 to 2.3).
    fn route(
        &mut self,
        app: StructId,
        receiver: &HirExpr,
        http: HttpMethod,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<HirRoute> {
        let local = self.app_local(receiver, span, Added::Route)?;
        let [path, handler] = args else {
            let call = format!("{}.{}", self.receiver_text(receiver), method.name);
            let message = format!(
                "`{call}` takes 2 arguments, a path and a handler, but {} {} given",
                args.len(),
                if args.len() == 1 { "was" } else { "were" },
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0201, span, message));
            return None;
        };
        let path = self.literal_path(path, Added::Route);
        if let Some(path) = &path {
            self.route_taken(local, http, path, span)?;
        }
        let handler = self.handler(handler, Added::Route);
        let (path, handler) = (path?, handler?);
        self.added_from_async(handler, span);
        let symbols = self.symbols;
        let items = symbols.http_of(app)?;
        let state = self.app_state(local)?;
        let params = self.bindings(http, &path, handler, items, &state, span);
        let ret = self.return_shape(handler, items, span);
        Some(HirRoute {
            app: local,
            method: http,
            path,
            handler,
            params: params?,
            ret: ret?,
            span,
        })
    }

    /// `receiver.method(f)`, or `receiver.before_on(prefix, f)` when
    /// `on`, at `span`, `receiver` of the app struct `app`: a hook of
    /// `kind` (spec 2.4).
    fn hook(
        &mut self,
        app: StructId,
        receiver: &HirExpr,
        (kind, on): (HookKind, bool),
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<HirHook> {
        let local = self.app_local(receiver, span, Added::Hook)?;
        let (prefix, f) = match (on, args) {
            (false, [f]) => (None, f),
            (true, [prefix, f]) => (Some(prefix), f),
            _ => {
                let call = format!("{}.{}", self.receiver_text(receiver), method.name);
                let wanted = if on {
                    "2 arguments, a prefix and a hook"
                } else {
                    "1 argument, a hook"
                };
                let message = format!(
                    "`{call}` takes {wanted}, but {} {} given",
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" },
                );
                self.diagnostics
                    .push(Diagnostic::new(codes::V0201, span, message));
                return None;
            }
        };
        let prefix = prefix.map(|prefix| self.literal_path(prefix, Added::Hook));
        let hook = self.handler(f, Added::Hook);
        let prefix = match prefix {
            Some(prefix) => Some(prefix?),
            None => None,
        };
        let hook = hook?;
        self.added_from_async(hook, span);
        let symbols = self.symbols;
        let items = symbols.http_of(app)?;
        let state = self.app_state(local)?;
        let takes_state = self.hook_shape(kind, hook, items, &state, span)?;
        Some(HirHook {
            app: local,
            kind,
            prefix,
            hook,
            state: takes_state,
            span,
        })
    }

    /// Records `f`, added at `span` as a route's handler or a hook, as
    /// called from this function when it is async: the adapter's future
    /// then holds `f`'s, so a handler that gets back to this function is
    /// a cycle of async calls (V0214).
    fn added_from_async(&mut self, f: FnId, span: Span) {
        if self.is_async {
            self.async_callees.push((f, span));
        }
    }

    /// Whether `hook`, a hook of `kind` on an app of `items` whose state
    /// is `state`, takes the state: V0220 at the hook call `span` for the
    /// first way its signature is not the one of spec 2.4, its parameters
    /// by position: the request, not `mut`; for an `after` hook the
    /// response; then the state, if it reads it. A `before` hook returns
    /// `Result<bool, Error>`, and an `after` hook nothing.
    fn hook_shape(
        &mut self,
        kind: HookKind,
        hook: FnId,
        items: &HttpItems,
        state: &Ty,
        span: Span,
    ) -> Option<bool> {
        let symbols = self.symbols;
        let sig = &symbols.fns[hook.0 as usize];
        let f = &sig.name;
        let request = Ty::Struct(items.request);
        let response = Ty::Struct(items.response);
        let shared = Ty::Shared(Box::new(state.clone()));
        let (hook_is, fixed): (&str, Vec<(&Ty, &str)>) = match kind {
            HookKind::Before => ("a `before` hook", vec![(&request, "the request")]),
            HookKind::After => (
                "an `after` hook",
                vec![(&request, "the request"), (&response, "the response")],
            ),
        };
        let signature = match kind {
            HookKind::Before => format!(
                "a `before` hook is `async fn f(req: {}) -> Result<bool, Error>`, with a second \
                 parameter `state: {}` when it reads the state",
                self.ty_name(&request),
                self.ty_name(&shared)
            ),
            HookKind::After => format!(
                "an `after` hook is `async fn f(req: {}, mut res: {})`, with a third parameter \
                 `state: {}` when it reads the state",
                self.ty_name(&request),
                self.ty_name(&response),
                self.ty_name(&shared)
            ),
        };
        let count = fixed.len();
        let problem = if sig.params.len() < count || sig.params.len() > count + 1 {
            let takes = match sig.params.len() {
                0 => "nothing".to_string(),
                1 => "1 parameter".to_string(),
                n => format!("{n} parameters"),
            };
            let wanted = match kind {
                HookKind::Before => "the request",
                HookKind::After => "the request and the response",
            };
            Some((
                format!(
                    "`{f}` takes {takes}, and {hook_is} takes {wanted}, then the state if it \
                     reads it"
                ),
                signature,
            ))
        } else {
            sig.params
                .iter()
                .enumerate()
                .find_map(|(index, (name, ty, mode))| match fixed.get(index) {
                    Some((wanted, what)) if ty != *wanted => Some((
                        format!(
                            "the parameter `{name}` of `{f}` is `{}`, and the {} parameter of \
                             {hook_is} is {what}, a `{}`",
                            self.ty_name(ty),
                            if index == 0 { "first" } else { "second" },
                            self.ty_name(wanted)
                        ),
                        signature.clone(),
                    )),
                    Some((wanted, _))
                        if *wanted == &request && *mode == ParamMode::MutableBorrow =>
                    {
                        Some((
                            format!("the request `{name}` of `{f}` cannot be `mut`"),
                            "a change to the request here does not reach the handler".to_string(),
                        ))
                    }
                    Some(_) => None,
                    None => match ty {
                        Ty::Shared(_) if *ty == shared => None,
                        Ty::Shared(_) => Some((
                            format!(
                                "the parameter `{name}` of `{f}` is a `{}`, but the app's state \
                                 is `{}`",
                                self.ty_name(ty),
                                self.ty_name(state)
                            ),
                            "a hook reads the state given to `App::new`".to_string(),
                        )),
                        _ => Some((
                            format!(
                                "the parameter `{name}` of `{f}` is `{}`, and the last parameter \
                                 of {hook_is}, if it has one more, is the state, a `{}`",
                                self.ty_name(ty),
                                self.ty_name(&shared)
                            ),
                            signature.clone(),
                        )),
                    },
                })
        };
        let problem = problem.or_else(|| {
            let returns = match kind {
                HookKind::Before => sig.ret == Ty::Result(Box::new(Ty::Bool), Box::new(Ty::Error)),
                HookKind::After => sig.ret == Ty::Unit,
            };
            if returns {
                return None;
            }
            let ret = self.ty_name(&sig.ret);
            Some(match kind {
                HookKind::Before => (
                    format!(
                        "`{f}` returns `{ret}`, and a `before` hook returns `Result<bool, Error>`"
                    ),
                    "`Ok(true)` lets the request through, `Ok(false)` refuses it with a 403, and \
                     an `Err` sends the error's response"
                        .to_string(),
                ),
                HookKind::After => (
                    format!("`{f}` returns `{ret}`, and an `after` hook returns nothing"),
                    "an `after` hook changes the response through a `mut` parameter, as in \
                     `res.set_header(\"x-name\", \"value\")`"
                        .to_string(),
                ),
            })
        });
        match problem {
            Some((message, note)) => {
                self.diagnostics
                    .push(Diagnostic::new(codes::V0220, span, message).with_note(note));
                None
            }
            None => Some(sig.params.len() > count),
        }
    }

    /// The segments of `arg`, a route's path or a hook's prefix: a string
    /// literal (V0217) of the shape of [`parse_path`] (V0222), and for a
    /// prefix one with no `{name}`, since it is matched on its segments as
    /// written (spec 2.4).
    fn literal_path(&mut self, arg: &Expr, added: Added) -> Option<Vec<Segment>> {
        let (what, noun, valid, example) = match added {
            Added::Route => ("a route's path", "path", "route path", "\"/users/{id}\""),
            Added::Hook => ("a hook's prefix", "prefix", "prefix", "\"/admin\""),
        };
        let ExprKind::String(text) = &arg.kind else {
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0217,
                    arg.span,
                    format!("{what} must be text written in the program"),
                )
                .with_note(format!("write the {noun} in quotes, as in `{example}`")),
            );
            return None;
        };
        let parsed = parse_path(text).and_then(|segments| {
            let named = segments.iter().find_map(|segment| match segment {
                Segment::Param(name) if added == Added::Hook => Some(name),
                _ => None,
            });
            match named {
                Some(name) => Err(format!(
                    "a prefix is matched on its segments as written, so `{{{name}}}` cannot be \
                     one of them; `\"/users\"` covers `/users/1` and `/users/1/posts`"
                )),
                None => Ok(segments),
            }
        });
        match parsed {
            Ok(segments) => Some(segments),
            Err(why) => {
                self.diagnostics.push(
                    Diagnostic::new(
                        codes::V0222,
                        arg.span,
                        format!("`\"{text}\"` is not a valid {valid}"),
                    )
                    .with_note(why),
                );
                None
            }
        }
    }

    /// V0222 at `span` when the app local `local` already has a route for
    /// `method` and `path`, which is otherwise recorded (spec 2.2).
    fn route_taken(
        &mut self,
        local: LocalId,
        method: HttpMethod,
        path: &[Segment],
        span: Span,
    ) -> Option<()> {
        let name = self.locals[local.0 as usize].name.clone();
        let app = self.apps.get_mut(&local)?;
        let first = app
            .routes
            .iter()
            .find(|(m, p, _)| *m == method && p.as_slice() == path)
            .map(|(_, _, at)| *at);
        let Some(first) = first else {
            app.routes.push((method, path.to_vec(), span));
            return Some(());
        };
        let route = format!("{} \"{}\"", method.call(), path_text(path));
        self.diagnostics.push(
            Diagnostic::new(
                codes::V0222,
                span,
                format!("`{name}` already has the route `{route}`"),
            )
            .with_label(first, "first added here")
            .with_note("each method and path is added once; a second route would never be reached"),
        );
        None
    }

    /// The function `arg` names as a route's handler or as a hook, as
    /// `added` says: the path of an async function of this package (spec
    /// 2.2, 2.4), visible from here (V0105), not `main` (V0106) or a test
    /// (V0114), and not a method, a Rust function, a function of another
    /// package, a closure, or a local (V0220). Resolved as a function,
    /// never as a value.
    pub(super) fn handler(&mut self, arg: &Expr, added: Added) -> Option<FnId> {
        let (role, word) = (added.role(), added.word());
        let (path, name) = match &arg.kind {
            ExprKind::Path { path, name } => (path.as_ref(), name),
            ExprKind::Closure { .. } => {
                let message = format!("{role} is named here, not written as a closure");
                self.diagnostics.push(
                    Diagnostic::new(codes::V0220, arg.span, message)
                        .with_note("move its code into an `async fn` and name that function here"),
                );
                return None;
            }
            _ => {
                let message = format!(
                    "{role} is the name of an async function, as in `{}`",
                    added.example()
                );
                self.diagnostics
                    .push(Diagnostic::new(codes::V0220, arg.span, message));
                return None;
            }
        };
        let full = match path {
            Some(path) => display_path(path, &name.name),
            None => name.name.clone(),
        };
        if path.is_none()
            && self
                .scopes
                .iter()
                .any(|scope| scope.contains_key(&name.name))
        {
            let message =
                format!("`{full}` is a variable here, and {role} is a function named by its path");
            self.diagnostics.push(
                Diagnostic::new(codes::V0220, arg.span, message)
                    .with_note("name an `async fn` of this package here"),
            );
            return None;
        }
        if let Some(owner) = self.path_owner(path, arg.span) {
            let (owner, _) = owner?;
            let message = match owner {
                PathOwner::User(_) => {
                    format!("`{full}` belongs to a type, and {role} is a function of a module")
                }
                PathOwner::Builtin(_) => format!(
                    "`{full}` is part of Varyk, and {role} is an async function of this \
                     package"
                ),
            };
            self.diagnostics.push(
                Diagnostic::new(codes::V0220, arg.span, message)
                    .with_note("write an `async fn` outside any `impl` block, and name it here"),
            );
            return None;
        }
        let found = match self.symbols.lookup_fn(self.module, path, &name.name) {
            Ok(found) => found,
            Err(error) => {
                let hint = path
                    .and_then(|p: &Path| p.segments.last())
                    .map_or(name.name.as_str(), |s| s.name.as_str());
                self.lookup_error(error, "function", &full, arg.span, Some(hint));
                return None;
            }
        };
        let id = match found {
            Callee::Varyk(id) => id,
            Callee::Imported(id) => {
                let message = match &self.symbols.imported[id.0 as usize].package {
                    Some(item) => format!(
                        "`{full}` is a function of the package `{}`, and {role} is a function \
                         of this package",
                        item.name
                    ),
                    None => format!(
                        "`{full}` is a Rust function, and {role} is an async function written \
                         in Varyk"
                    ),
                };
                self.diagnostics
                    .push(Diagnostic::new(codes::V0220, arg.span, message).with_note(
                        "write an `async fn` in this package that calls it, and name it here",
                    ));
                return None;
            }
            Callee::Builtin(_) => {
                let message = format!("`{full}` is not a function of this package");
                self.diagnostics
                    .push(Diagnostic::new(codes::V0220, arg.span, message));
                return None;
            }
        };
        let sig = &self.symbols.fns[id.0 as usize];
        let diagnostic = if sig.name == "main" && sig.owner.is_none() && sig.module == ModuleId(0) {
            Diagnostic::new(
                codes::V0106,
                arg.span,
                format!("`main` is where the program starts and cannot be {role}"),
            )
            .with_note(format!(
                "put what the {word} does in an `async fn` of its own, and name that"
            ))
        } else if sig.is_test {
            Diagnostic::new(
                codes::V0114,
                arg.span,
                format!("`{full}` is a test and cannot be {role}"),
            )
            .with_note(format!(
                "`varyk test` runs each test on its own; put what the {word} does in an `async \
                 fn` without `#[test]`"
            ))
        } else if !sig.is_async {
            Diagnostic::new(
                codes::V0220,
                arg.span,
                format!("`{full}` is not an async function, and {role} is one"),
            )
            .with_note(format!("declare it `async fn {}`", sig.name))
        } else {
            return Some(id);
        };
        self.diagnostics.push(diagnostic);
        None
    }

    /// How each parameter of `handler` binds on a route of `method` with
    /// `path` (spec 2.2 rules 1 to 5), an app of `items` whose state is
    /// `state`: V0219 at the route call `span` for each one that does
    /// not, and for each `{name}` no parameter takes. A body is read from
    /// JSON (V0210 when it cannot be).
    fn bindings(
        &mut self,
        method: HttpMethod,
        path: &[Segment],
        handler: FnId,
        items: &HttpItems,
        state: &Ty,
        span: Span,
    ) -> Option<Vec<Binding>> {
        let symbols = self.symbols;
        let sig = &symbols.fns[handler.0 as usize];
        let f = &sig.name;
        let names: Vec<&str> = path
            .iter()
            .filter_map(|segment| match segment {
                Segment::Param(name) => Some(name.as_str()),
                Segment::Literal(_) => None,
            })
            .collect();
        let mut bindings = Vec::new();
        let mut problems = Vec::new();
        let mut shared: Option<&str> = None;
        let mut body: Option<(&str, &Ty)> = None;
        let mut bound: Vec<(StructId, &str)> = Vec::new();
        for (name, ty, _) in &sig.params {
            let binding = match ty {
                _ if names.contains(&name.as_str()) => {
                    if is_path_type(ty) {
                        Ok(Binding::Path(name.clone()))
                    } else {
                        Err((
                            format!(
                                "the path parameter `{name}` of `{f}` is `{}`, and a path segment \
                                 gives an integer, `bool`, `string`, `Time`, or `Uuid`",
                                self.ty_name(ty)
                            ),
                            format!(
                                "give `{name}` an integer type, `bool`, `string`, `Time`, or \
                                 `Uuid`; a value with more in it, such as a struct, comes from \
                                 the body or the state"
                            ),
                        ))
                    }
                }
                Ty::Shared(inner) if **inner != *state => Err((
                    format!(
                        "the parameter `{name}` of `{f}` is a `{}`, but the app's state is `{}`",
                        self.ty_name(ty),
                        self.ty_name(state)
                    ),
                    "a handler reads the state given to `App::new`".to_string(),
                )),
                Ty::Shared(_) => match shared {
                    Some(first) => Err((
                        format!("`{f}` takes the state twice, as `{first}` and `{name}`"),
                        format!("take one `{}` parameter", self.ty_name(ty)),
                    )),
                    None => {
                        shared = Some(name);
                        Ok(Binding::State)
                    }
                },
                Ty::Struct(id) if items.binds(*id) => {
                    match bound.iter().find(|(other, _)| other == id) {
                        Some((_, first)) => Err((
                            format!(
                                "`{f}` takes two `{}` parameters, `{first}` and `{name}`",
                                self.ty_name(ty)
                            ),
                            "the package gives one for each request".to_string(),
                        )),
                        None => {
                            bound.push((*id, name));
                            Ok(Binding::Package(*id))
                        }
                    }
                }
                Ty::Struct(id) if items.root_name(*id).is_some() => {
                    let note = match items.root_name(*id) {
                        Some("Response") => "a handler returns its response rather than taking one",
                        Some("Client") => {
                            "make a client in the handler with `Client::new()`, or keep one in \
                             the state"
                        }
                        _ => "keep what the handler needs in the state",
                    };
                    Err((
                        format!(
                            "the parameter `{name}` of `{f}` is a `{}`, which a request does not \
                             give",
                            self.ty_name(ty)
                        ),
                        note.to_string(),
                    ))
                }
                Ty::Struct(_) | Ty::Enum(_) | Ty::Vec(_) | Ty::HashMap(..) => {
                    if !method.has_body() {
                        Err((
                            format!(
                                "a `{}` request has no body, so the parameter `{name}` of `{f}` \
                                 gets no value",
                                method.call()
                            ),
                            "a body is read on `post`, `put`, and `patch`".to_string(),
                        ))
                    } else if let Some((first, _)) = body {
                        Err((
                            format!(
                                "`{f}` has two parameters that would each be the body, `{first}` \
                                 and `{name}`"
                            ),
                            "a request has one body; put both in one struct".to_string(),
                        ))
                    } else {
                        body = Some((name, ty));
                        Ok(Binding::Body)
                    }
                }
                _ if is_query(ty) => Ok(Binding::Query(name.clone())),
                _ => Err((
                    format!("the parameter `{name}` of `{f}` gets no value from the request"),
                    PARAMETERS.to_string(),
                )),
            };
            match binding {
                Ok(binding) => bindings.push(binding),
                Err(problem) => problems.push(problem),
            }
        }
        for name in names {
            if !sig.params.iter().any(|(param, _, _)| param == name) {
                problems.push((
                    format!("the path names `{{{name}}}`, but `{f}` has no parameter `{name}`"),
                    format!(
                        "add a parameter `{name}` to `{f}`, of an integer type, `bool`, \
                         `string`, `Time`, or `Uuid`"
                    ),
                ));
            }
        }
        let failed = !problems.is_empty();
        for (message, note) in problems {
            self.diagnostics
                .push(Diagnostic::new(codes::V0219, span, message).with_note(note));
        }
        if let Some((_, ty)) = body {
            self.converted(ty, false, Direction::Deserialize, span)?;
        }
        (!failed).then_some(bindings)
    }

    /// What `handler`, on a route of an app of `items` at `span`, returns
    /// (spec 2.3): V0220 for a type a route cannot send, and V0210 for
    /// one JSON cannot hold, which is otherwise written by serde.
    fn return_shape(
        &mut self,
        handler: FnId,
        items: &HttpItems,
        span: Span,
    ) -> Option<ReturnShape> {
        let symbols = self.symbols;
        let sig = &symbols.fns[handler.0 as usize];
        let ret = &sig.ret;
        let found = match ret {
            Ty::Unit => Some((ReturnShape::Nothing, None)),
            Ty::Result(ok, err) if **err == Ty::Error => sent(ok, items)
                .map(|(shape, written)| (ReturnShape::Result(Box::new(shape)), written)),
            other => sent(other, items),
        };
        let Some((shape, written)) = found else {
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0220,
                    span,
                    format!(
                        "`{}` returns `{}`, which a route cannot send",
                        sig.name,
                        self.ty_name(ret)
                    ),
                )
                .with_note(
                    "a handler returns nothing, a value JSON can hold, an `Option` of one, a \
                     `Response`, or a `Result` of one of those with `Error`",
                ),
            );
            return None;
        };
        if let Some(written) = written {
            self.converted(written, false, Direction::Serialize, span)?;
        }
        Some(shape)
    }
}

/// How a route sends a value of `ty`, a handler's return or its `Ok`
/// (spec 2.3), with the type written as JSON, if any; `None` when it
/// cannot: nothing inside a `Result`, a `Result` or `Option` inside, or a
/// struct of the package other than its `Response` (`Request`, `App`).
fn sent<'t>(ty: &'t Ty, items: &HttpItems) -> Option<(ReturnShape, Option<&'t Ty>)> {
    let package_struct = |ty: &Ty| matches!(ty, Ty::Struct(id) if items.root_name(*id).is_some());
    match ty {
        Ty::Struct(id) if *id == items.response => Some((ReturnShape::Response, None)),
        Ty::Unit | Ty::Result(..) => None,
        _ if package_struct(ty) => None,
        Ty::Option(inner) => match &**inner {
            Ty::Unit | Ty::Option(_) | Ty::Result(..) => None,
            inner if package_struct(inner) => None,
            inner => Some((ReturnShape::Option, Some(inner))),
        },
        _ => Some((ReturnShape::Json, Some(ty))),
    }
}

/// V0221 at `span` for a route or hook call inside a closure (spec 2.1).
fn in_a_closure(span: Span, added: Added) -> Diagnostic {
    let word = added.word();
    Diagnostic::new(
        codes::V0221,
        span,
        format!("a {word} cannot be added inside a closure, which may run more than once"),
    )
    .with_note(format!(
        "add each {word} once, in the function that makes the app"
    ))
}
