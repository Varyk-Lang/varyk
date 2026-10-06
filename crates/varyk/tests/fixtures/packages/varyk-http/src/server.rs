// An in-memory stand-in for varyk-http (milestone 5b4 spec 6): the items
// the compiler names, and the Varyk surface of `Request` and `Response`,
// over a router that records routes and hooks and answers `App::request`
// with no network. `serve` binds nothing.

use std::any::Any;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

type Boxed<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type State = Arc<dyn Any + Send + Sync>;

/// A route's adapter, wrapped by `route` for `App::get` and the others.
pub struct Route {
    run: Arc<dyn Fn(Request) -> Boxed<Response> + Send + Sync>,
}

/// A `before` adapter, wrapped by `before_hook`: `None` lets the request
/// through, `Some` answers it.
pub struct BeforeHook {
    run: Arc<dyn Fn(Request) -> Boxed<Option<Response>> + Send + Sync>,
}

/// An `after` adapter, wrapped by `after_hook`: gives the response on.
pub struct AfterHook {
    run: Arc<dyn Fn(Request, Response) -> Boxed<Response> + Send + Sync>,
}

/// The route table: the app's state, its routes, and its hooks, in the
/// order they were registered.
pub struct App {
    state: State,
    routes: Vec<(String, String, Route)>,
    befores: Vec<(String, BeforeHook)>,
    afters: Vec<AfterHook>,
}

/// One request, as a handler, a hook, and `App::request` see it. Cheap to
/// copy: the body is shared.
#[derive(Clone)]
pub struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Arc<String>,
    params: Vec<(String, String)>,
    state: Option<State>,
}

/// One response: what a handler builds and what `App::request` gives.
pub struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

/// The HTTP client; the stand-in has no network, so it does nothing.
pub struct Client {
    _private: (),
}

/// A path or query value: an integer, `bool`, or text.
pub trait Plain: std::str::FromStr {}

impl Plain for i8 {}
impl Plain for i16 {}
impl Plain for i32 {}
impl Plain for i64 {}
impl Plain for u8 {}
impl Plain for u16 {}
impl Plain for u32 {}
impl Plain for u64 {}
impl Plain for usize {}
impl Plain for bool {}
impl Plain for String {}

/// A query parameter: a plain value, required, or an `Option` of one.
pub trait QueryValue: Sized {
    fn from_query(name: &str, found: Option<&str>) -> Result<Self, Response>;
}

impl<T: Plain> QueryValue for T {
    fn from_query(name: &str, found: Option<&str>) -> Result<T, Response> {
        match found {
            Some(text) => read("query parameter", name, text),
            None => Err(error_body(
                400,
                &format!("the query parameter `{name}` is missing"),
            )),
        }
    }
}

impl<T: Plain> QueryValue for Option<T> {
    fn from_query(name: &str, found: Option<&str>) -> Result<Option<T>, Response> {
        match found {
            Some(text) => read("query parameter", name, text).map(Some),
            None => Ok(None),
        }
    }
}

/// A value the package makes for one request, bound by its type.
pub trait Bound: Sized {
    fn bound(request: &Request) -> Result<Self, Response>;
}

impl Bound for Request {
    fn bound(request: &Request) -> Result<Request, Response> {
        Ok(request.clone())
    }
}

impl App {
    /// An app whose handlers read `state`.
    pub fn new<S: Send + Sync + 'static>(state: Arc<S>) -> App {
        let state: State = state;
        App {
            state,
            routes: Vec::new(),
            befores: Vec::new(),
            afters: Vec::new(),
        }
    }

    pub fn get(&mut self, path: &str, route: Route) {
        self.add("GET", path, route);
    }

    pub fn post(&mut self, path: &str, route: Route) {
        self.add("POST", path, route);
    }

    pub fn put(&mut self, path: &str, route: Route) {
        self.add("PUT", path, route);
    }

    pub fn patch(&mut self, path: &str, route: Route) {
        self.add("PATCH", path, route);
    }

    pub fn delete(&mut self, path: &str, route: Route) {
        self.add("DELETE", path, route);
    }

    pub fn before(&mut self, hook: BeforeHook) {
        self.befores.push(("/".to_string(), hook));
    }

    pub fn before_on(&mut self, prefix: &str, hook: BeforeHook) {
        self.befores.push((prefix.to_string(), hook));
    }

    pub fn after(&mut self, hook: AfterHook) {
        self.afters.push(hook);
    }

    /// Binds nothing: the stand-in has no network. A conflict between
    /// routes is an `Err`, as the real router's is.
    pub async fn serve(&self, _port: u16) -> Result<bool, varyk_std::Error> {
        match self.conflict() {
            Some(conflict) => Err(varyk_std::Error::new(conflict)),
            None => Ok(true),
        }
    }

    /// Runs `req` through the hooks and the route it matches, with no
    /// port, and gives the response the server would have sent.
    pub async fn request(&self, req: Request) -> Response {
        if let Some(conflict) = self.conflict() {
            varyk_std::tracing::error!("{conflict}");
            return internal();
        }
        let mut req = req;
        req.state = Some(Arc::clone(&self.state));
        // A request no route matches runs no `before` hook; one a route
        // matches runs those whose prefix covers the route's pattern.
        let mut response = match self.find(&req.method, &req.path) {
            Found::Route(pattern, route, params) => {
                req.params = params;
                let mut stopped = None;
                for (prefix, hook) in &self.befores {
                    if covers(prefix, pattern) {
                        if let Some(response) = (hook.run)(req.clone()).await {
                            stopped = Some(response);
                            break;
                        }
                    }
                }
                match stopped {
                    Some(response) => response,
                    None => (route.run)(req.clone()).await,
                }
            }
            Found::WrongMethod => error_body(405, "method not allowed"),
            Found::Missing => error_body(404, "not found"),
        };
        for hook in &self.afters {
            response = (hook.run)(req.clone(), response).await;
        }
        sendable(response)
    }

    fn add(&mut self, method: &str, path: &str, route: Route) {
        self.routes
            .push((method.to_string(), path.to_string(), route));
    }

    /// Two routes of one method whose paths match the same requests.
    fn conflict(&self) -> Option<String> {
        for (index, (method, path, _)) in self.routes.iter().enumerate() {
            let taken = self.routes[..index]
                .iter()
                .any(|(other, earlier, _)| other == method && shape(earlier) == shape(path));
            if taken {
                return Some(format!("the route {method} {path} is already taken"));
            }
        }
        None
    }

    fn find(&self, method: &str, path: &str) -> Found<'_> {
        let mut found = Found::Missing;
        for (route_method, pattern, route) in &self.routes {
            if let Some(params) = matched(pattern, path) {
                if route_method == method {
                    return Found::Route(pattern, route, params);
                }
                found = Found::WrongMethod;
            }
        }
        found
    }
}

/// What a request's method and path find among the routes: a route
/// with its pattern as written and the `{name}` values.
enum Found<'a> {
    Route(&'a str, &'a Route, Vec<(String, String)>),
    WrongMethod,
    Missing,
}

/// The segments of `path`, without its leading `/`.
fn segments(path: &str) -> Vec<&str> {
    path.strip_prefix('/').unwrap_or(path).split('/').collect()
}

/// `path` with every `{name}` segment written `{}`.
fn shape(path: &str) -> Vec<&str> {
    segments(path)
        .into_iter()
        .map(|segment| if is_param(segment) { "{}" } else { segment })
        .collect()
}

fn is_param(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}') && segment.len() > 2
}

/// The `{name}` values of `path` when it matches `pattern` whole.
fn matched(pattern: &str, path: &str) -> Option<Vec<(String, String)>> {
    let wanted = segments(pattern);
    let given = segments(path);
    if wanted.len() != given.len() {
        return None;
    }
    let mut params = Vec::new();
    for (want, give) in wanted.iter().zip(given.iter()) {
        if is_param(want) {
            if give.is_empty() {
                return None;
            }
            params.push((want[1..want.len() - 1].to_string(), give.to_string()));
        } else if want != give {
            return None;
        }
    }
    Some(params)
}

/// Whether `prefix` covers the route pattern `pattern`, as written, on
/// whole segments.
fn covers(prefix: &str, pattern: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    let wanted = segments(prefix);
    let given = segments(pattern);
    given.len() >= wanted.len() && wanted.iter().zip(given.iter()).all(|(a, b)| a == b)
}

impl Request {
    /// A request to send through `App::request`; a query string after
    /// `?` in `path` is its query.
    pub fn new(method: &str, path: &str) -> Request {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let query = query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                (key.to_string(), value.to_string())
            })
            .collect();
        Request {
            method: method.to_ascii_uppercase(),
            path: path.to_string(),
            query,
            headers: Vec::new(),
            body: Arc::new(String::new()),
            params: Vec::new(),
            state: None,
        }
    }

    pub fn method(&self) -> String {
        self.method.clone()
    }

    pub fn path(&self) -> String {
        self.path.clone()
    }

    pub fn header(&self, name: &str) -> Option<String> {
        header(&self.headers, name)
    }

    pub fn cookie(&self, name: &str) -> Option<String> {
        let cookies = self.header("cookie")?;
        cookies.split(';').find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name).then(|| value.to_string())
        })
    }

    pub fn body(&self) -> String {
        self.body.as_ref().clone()
    }

    pub fn set_header(&mut self, name: &str, value: &str) {
        set_header(&mut self.headers, name, value);
    }

    pub fn set_body(&mut self, text: &str) {
        self.body = Arc::new(text.to_string());
    }

    /// The path parameter `name` as a `P`; a 400 naming it when it does
    /// not read as one.
    pub fn param<P: Plain>(&self, name: &str) -> Result<P, Response> {
        let found = self.params.iter().find(|(key, _)| key == name);
        match found {
            Some((_, text)) => read("path parameter", name, text),
            None => Err(error_body(
                400,
                &format!("the path parameter `{name}` is missing"),
            )),
        }
    }

    /// The query parameter `name` as a `Q`.
    pub fn query<Q: QueryValue>(&self, name: &str) -> Result<Q, Response> {
        let found = self.query.iter().find(|(key, _)| key == name);
        Q::from_query(name, found.map(|(_, value)| value.as_str()))
    }

    /// The body read from JSON as a `B`; a 400 naming `name`, the
    /// handler's parameter, when it is not one.
    pub async fn json<B: varyk_std::serde::de::DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<B, Response> {
        varyk_std::json::parse(&self.body).map_err(|err| {
            error_body(
                400,
                &format!("the body `{name}` is not valid: {}", err.message()),
            )
        })
    }

    /// The app's state, as the `S` the routes were checked against.
    pub fn state<S: Send + Sync + 'static>(&self) -> Result<Arc<S>, Response> {
        let Some(state) = &self.state else {
            varyk_std::tracing::error!("this request carries no state");
            return Err(internal());
        };
        Arc::clone(state).downcast::<S>().map_err(|_| {
            varyk_std::tracing::error!("the app's state is not of the type its routes read");
            internal()
        })
    }

    /// A value the package makes for this request, bound by its type.
    pub async fn bind<K: Bound>(&self) -> Result<K, Response> {
        K::bound(self)
    }
}

impl Response {
    /// A 200 with `value` as JSON.
    pub fn json<T: varyk_std::serde::Serialize + ?Sized>(value: &T) -> Response {
        let mut response = Response {
            status: 200,
            headers: Vec::new(),
            body: varyk_std::json::stringify(value),
        };
        response.set_header("content-type", "application/json");
        response
    }

    /// A 200 with `s` as plain text.
    pub fn text(s: &str) -> Response {
        let mut response = Response {
            status: 200,
            headers: Vec::new(),
            body: s.to_string(),
        };
        response.set_header("content-type", "text/plain; charset=utf-8");
        response
    }

    /// A 204 with no body.
    pub fn empty() -> Response {
        Response {
            status: 204,
            headers: Vec::new(),
            body: String::new(),
        }
    }

    /// The file `name` from the folder `dir`: a 404, with the reason
    /// logged, for a name that is absolute, has a `..` segment or one
    /// starting with `.`, or resolves outside `dir`.
    pub fn file(dir: &str, name: &str) -> Response {
        let found = file_path(dir, name).and_then(|path| {
            std::fs::read_to_string(&path).map_err(|err| format!("it cannot be read: {err}"))
        });
        match found {
            Ok(text) => Response::text(&text),
            Err(reason) => {
                varyk_std::tracing::warn!("not sending the file `{name}`: {reason}");
                error_body(404, "not found")
            }
        }
    }

    pub fn set_status(&mut self, code: u16) {
        self.status = code;
    }

    pub fn set_header(&mut self, name: &str, value: &str) {
        set_header(&mut self.headers, name, value);
    }

    /// A cookie with HttpOnly, Secure, and SameSite=Lax set, kept for
    /// `max_age` seconds.
    pub fn set_cookie(&mut self, name: &str, value: &str, max_age: i64) {
        self.headers.push((
            "set-cookie".to_string(),
            format!("{name}={value}; Max-Age={max_age}; HttpOnly; Secure; SameSite=Lax"),
        ));
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn header(&self, name: &str) -> Option<String> {
        header(&self.headers, name)
    }

    pub fn body(&self) -> String {
        self.body.clone()
    }

    /// The body read from JSON as a `T`.
    pub fn read_json<T: varyk_std::serde::de::DeserializeOwned>(
        &self,
    ) -> Result<T, varyk_std::Error> {
        varyk_std::json::parse(&self.body)
    }
}

impl Client {
    pub fn new() -> Client {
        Client { _private: () }
    }
}

impl Default for Client {
    fn default() -> Client {
        Client::new()
    }
}

/// Wraps a route's adapter for `App::get` and the others.
pub fn route<F, Fut>(adapter: F) -> Route
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    let run: Arc<dyn Fn(Request) -> Boxed<Response> + Send + Sync> =
        Arc::new(move |req| Box::pin(adapter(req)));
    Route { run }
}

/// Wraps a `before` adapter for `App::before` and `App::before_on`.
pub fn before_hook<F, Fut>(adapter: F) -> BeforeHook
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Option<Response>> + Send + 'static,
{
    let run: Arc<dyn Fn(Request) -> Boxed<Option<Response>> + Send + Sync> =
        Arc::new(move |req| Box::pin(adapter(req)));
    BeforeHook { run }
}

/// Wraps an `after` adapter for `App::after`.
pub fn after_hook<F, Fut>(adapter: F) -> AfterHook
where
    F: Fn(Request, Response) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    let run: Arc<dyn Fn(Request, Response) -> Boxed<Response> + Send + Sync> =
        Arc::new(move |req, res| Box::pin(adapter(req, res)));
    AfterHook { run }
}

/// A handler that returns nothing: a 204.
pub fn respond_empty() -> Response {
    Response::empty()
}

/// A handler's value: a 200 with its JSON.
pub fn respond_json<T: varyk_std::serde::Serialize + ?Sized>(value: &T) -> Response {
    Response::json(value)
}

/// A handler's `Option`: a 200 with its JSON, or a 404 for `None`.
pub fn respond_option<T: varyk_std::serde::Serialize>(value: Option<T>) -> Response {
    match value {
        Some(value) => Response::json(&value),
        None => error_body(404, "not found"),
    }
}

/// A handler's `Err`: with a status from 400 to 599, that status with
/// `{"error": message}`; otherwise a 500 whose message is logged and
/// never sent.
pub fn respond_error(error: varyk_std::Error) -> Response {
    match error.status() {
        Some(status) if (400..=599).contains(&status) => error_body(status, error.message()),
        _ => {
            varyk_std::tracing::error!("{}", error.message());
            internal()
        }
    }
}

/// A `before` hook's result: `None` lets the request through.
pub fn respond_before(result: Result<bool, varyk_std::Error>) -> Option<Response> {
    match result {
        Ok(true) => None,
        Ok(false) => Some(error_body(403, "forbidden")),
        Err(error) => Some(respond_error(error)),
    }
}

/// `{"error": message}` with `status`.
fn error_body(status: u16, message: &str) -> Response {
    let mut body = BTreeMap::new();
    body.insert("error", message);
    let mut response = Response::json(&body);
    response.status = status;
    response
}

/// The fixed body of a 500, whose message is only logged.
fn internal() -> Response {
    error_body(500, "internal error")
}

/// `text` read as the `T` of the parameter `name`; a 400 naming it.
fn read<T: Plain>(what: &str, name: &str, text: &str) -> Result<T, Response> {
    text.parse::<T>().map_err(|_| {
        error_body(
            400,
            &format!("the {what} `{name}` cannot be read from `{text}`"),
        )
    })
}

fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    headers.retain(|(key, _)| !key.eq_ignore_ascii_case(name));
    headers.push((name.to_ascii_lowercase(), value.to_string()));
}

/// `response`, or a 500 with the reason logged when a header cannot be
/// sent as HTTP.
fn sendable(response: Response) -> Response {
    let bad = response.headers.iter().find(|(name, value)| {
        name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0)
    });
    match bad {
        Some((name, _)) => {
            varyk_std::tracing::error!("the header `{name}` cannot be sent");
            internal()
        }
        None => response,
    }
}

/// The file `name` under `dir`, or why it is not sent.
fn file_path(dir: &str, name: &str) -> Result<PathBuf, String> {
    let relative = Path::new(name);
    if name.is_empty() || relative.is_absolute() || name.starts_with(['/', '\\']) {
        return Err("the name is absolute".to_string());
    }
    if name.split(['/', '\\']).any(|segment| segment.starts_with('.')) {
        return Err("the name has a segment starting with `.`".to_string());
    }
    let root = std::fs::canonicalize(dir).map_err(|err| format!("the folder: {err}"))?;
    let full = std::fs::canonicalize(root.join(relative)).map_err(|err| err.to_string())?;
    if !full.starts_with(&root) {
        return Err("it resolves outside the folder".to_string());
    }
    Ok(full)
}
