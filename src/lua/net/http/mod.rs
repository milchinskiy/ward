#![allow(clippy::unnecessary_wraps)]

use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use mlua::{Lua, LuaSerdeExt, Table, UserData, UserDataFields, UserDataMethods, Value};
use reqwest::redirect;
use reqwest::{Client, Response};
use serde_json::Value as JsonValue;

/// Cached `reqwest::Client` instances by effective client configuration.
static CLIENT_CACHE: OnceLock<Mutex<HashMap<ClientKey, Client>>> = OnceLock::new();

#[derive(Copy, Clone, Debug, Eq)]
struct ClientKey {
    timeout_nanos: Option<u64>,
    follow_redirects: bool,
}

impl PartialEq for ClientKey {
    fn eq(&self, other: &Self) -> bool {
        self.timeout_nanos == other.timeout_nanos && self.follow_redirects == other.follow_redirects
    }
}

impl Hash for ClientKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.timeout_nanos.hash(state);
        self.follow_redirects.hash(state);
    }
}

fn client_cache() -> &'static Mutex<HashMap<ClientKey, Client>> {
    CLIENT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn duration_to_nanos_u64(d: Duration) -> u64 {
    // Duration::as_nanos() is u128. Saturate to u64 for hashing.
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

fn get_or_build_client(options: &HttpOptions) -> mlua::Result<Client> {
    let key = ClientKey {
        timeout_nanos: options.timeout.map(duration_to_nanos_u64),
        follow_redirects: options.follow_redirects,
    };

    // Fast path: cache hit.
    if let Ok(guard) = client_cache().lock()
        && let Some(existing) = guard.get(&key)
    {
        return Ok(existing.clone());
    }

    // Build outside of the lock.
    let mut builder = Client::builder();
    if let Some(timeout) = options.timeout {
        builder = builder.timeout(timeout);
    }

    builder = if options.follow_redirects {
        builder.redirect(redirect::Policy::limited(10))
    } else {
        builder.redirect(redirect::Policy::none())
    };

    let client = builder.build().map_err(mlua::Error::external)?;

    // Cache insert (best-effort; if poisoned/contended, still return the client).
    if let Ok(mut guard) = client_cache().lock() {
        guard.entry(key).or_insert_with(|| client.clone());
    }

    Ok(client)
}

/// Initializes the `http` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let http_table = lua.create_table()?;

    http_table.set(
        "get",
        lua.create_async_function(|_, (url, opts): (String, Value)| async move {
            request_async("GET", &url, opts, Body::None).await
        })?,
    )?;

    http_table.set(
        "post",
        lua.create_async_function(|lua, (url, opts): (String, Value)| async move {
            let body = Body::from_opts(&lua, &opts)?;
            request_async("POST", &url, opts, body).await
        })?,
    )?;

    http_table.set(
        "put",
        lua.create_async_function(|lua, (url, opts): (String, Value)| async move {
            let body = Body::from_opts(&lua, &opts)?;
            request_async("PUT", &url, opts, body).await
        })?,
    )?;

    http_table.set(
        "delete",
        lua.create_async_function(|_, (url, opts): (String, Value)| async move {
            request_async("DELETE", &url, opts, Body::None).await
        })?,
    )?;

    http_table.set(
        "options",
        lua.create_async_function(|_, (url, opts): (String, Value)| async move {
            request_async("OPTIONS", &url, opts, Body::None).await
        })?,
    )?;

    Ok(http_table)
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<String>,
}

impl UserData for HttpResponse {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("is_ok", |_, this, ()| Ok((200..=299).contains(&this.status)));
        methods.add_method("get_status", |_, this, ()| Ok(this.status));
        methods.add_method("get_headers", |lua, this, ()| headers_table(lua, &this.headers));
        methods.add_method("get_body", |_, this, ()| Ok(this.body.clone()));
    }

    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("status", |_, this| Ok(this.status));
        fields.add_field_method_get("headers", |lua, this| headers_table(lua, &this.headers));
        fields.add_field_method_get("body", |_, this| Ok(this.body.clone()));
    }
}

fn headers_table(lua: &Lua, headers: &[(String, String)]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (key, value) in headers {
        table.set(key.as_str(), value.as_str())?;
    }
    Ok(table)
}

#[derive(Default)]
pub struct HttpOptions {
    pub(crate) query: Vec<(String, String)>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) follow_redirects: bool,
    pub(crate) allow_error: bool,
    pub(crate) body: Body,
}

#[derive(Default, Clone)]
pub enum Body {
    Json(JsonValue),
    Form(Vec<(String, String)>),
    #[default]
    None,
}

impl Body {
    pub(crate) fn from_opts(lua: &Lua, opts: &Value) -> mlua::Result<Self> {
        let Value::Table(table) = opts else {
            return Ok(Self::None);
        };

        if let Ok(json_value) = table.get::<Value>("json")
            && !json_value.is_nil()
        {
            let value: JsonValue = lua.from_value(json_value)?;
            return Ok(Self::Json(value));
        }

        if let Ok(form_table) = table.get::<Table>("form") {
            let mut form = Vec::new();
            for pair in form_table.pairs::<String, String>() {
                let (key, value) = pair.map_err(mlua::Error::external)?;
                form.push((key, value));
            }
            if !form.is_empty() {
                return Ok(Self::Form(form));
            }
        }

        Ok(Self::None)
    }
}

fn parse_options(value: Value, body: Body) -> mlua::Result<HttpOptions> {
    let Value::Table(table) = value else {
        return Ok(HttpOptions {
            follow_redirects: true,
            body,
            ..HttpOptions::default()
        });
    };

    let mut options = HttpOptions {
        follow_redirects: table.get::<Option<bool>>("follow_redirects")?.unwrap_or(true),
        allow_error: table.get::<Option<bool>>("allow_error")?.unwrap_or(false),
        body,
        ..HttpOptions::default()
    };

    if let Ok(query_table) = table.get::<Table>("query") {
        for pair in query_table.pairs::<String, Value>() {
            let (key, value) = pair.map_err(mlua::Error::external)?;
            let value_str = value_to_string(value)?;
            options.query.push((key, value_str));
        }
    }

    if let Ok(headers_table) = table.get::<Table>("headers") {
        for pair in headers_table.pairs::<String, String>() {
            let (key, value) = pair.map_err(mlua::Error::external)?;
            options.headers.push((key, value));
        }
    }

    if let Ok(timeout) = table.get::<Option<f64>>("timeout")
        && let Some(t) = timeout.filter(|t| t.is_sign_positive() && t.is_finite())
    {
        options.timeout = Some(Duration::from_secs_f64(t));
    }

    Ok(options)
}

fn value_to_string(value: Value) -> mlua::Result<String> {
    match value {
        Value::String(s) => Ok(s.to_str()?.to_owned()),
        Value::Integer(i) => Ok(i.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Boolean(b) => Ok(b.to_string()),
        other => Err(mlua::Error::external(format!(
            "query parameter must be string, number, or boolean, got {other:?}"
        ))),
    }
}

async fn request_async(method: &str, url: &str, opts: Value, body: Body) -> mlua::Result<HttpResponse> {
    let options = parse_options(opts, body)?;
    let client = get_or_build_client(&options)?;

    // Build request
    let method = method.parse().map_err(mlua::Error::external)?;
    let mut req = client.request(method, url);

    for (k, v) in &options.headers {
        req = req.header(k, v);
    }

    if !options.query.is_empty() {
        req = req.query(&options.query);
    }

    // Send
    let resp = match &options.body {
        Body::Json(payload) => req.json(payload).send().await,
        Body::Form(form) => req.form(form).send().await,
        Body::None => req.send().await,
    }
    .map_err(mlua::Error::external)?;

    convert_response_async(resp, options.allow_error).await
}

async fn convert_response_async(resp: Response, allow_error: bool) -> mlua::Result<HttpResponse> {
    let status = resp.status();
    if !allow_error && !status.is_success() {
        return Err(mlua::Error::external(format!("http error: status {}", status.as_u16())));
    }

    let mut headers = Vec::new();
    for (name, value) in resp.headers() {
        let value_str = value.to_str().map_err(mlua::Error::external)?.to_string();
        headers.push((name.to_string(), value_str));
    }

    let body = resp.text().await.map(Some).map_err(mlua::Error::external)?;

    Ok(HttpResponse {
        status: status.as_u16(),
        headers,
        body,
    })
}
