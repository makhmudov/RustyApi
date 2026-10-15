//! Экстракторы `Json`, `Path`, `Query`, `Form` с ошибками 422 в формате FastAPI / Pydantic v2.
//!
//! После десериализации значение проверяется через [`Validate`] (правила `#[field(...)]`).
//! Ошибки разбора и проверки возвращаются как `422 {"detail": [{"type", "loc", "msg", "input", "ctx"}]}`;
//! `loc` начинается с `"body"`, `"path"` или `"query"`, путь внутри значения строится через
//! `serde_path_to_error`.

use std::ops::{Deref, DerefMut};

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, PathRejection};
use axum::extract::{FromRequest, FromRequestParts, RawPathParams, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::HTTPException;
use crate::schema::{ErrorDetail, Loc, LocItem, Validate, ValidationError};

macro_rules! newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name<T>(pub T);

        impl<T> $name<T> {
            /// Внутреннее значение.
            pub fn into_inner(self) -> T {
                self.0
            }
        }

        impl<T> Deref for $name<T> {
            type Target = T;
            fn deref(&self) -> &T {
                &self.0
            }
        }

        impl<T> DerefMut for $name<T> {
            fn deref_mut(&mut self) -> &mut T {
                &mut self.0
            }
        }

        impl<T> From<T> for $name<T> {
            fn from(value: T) -> Self {
                Self(value)
            }
        }
    };
}

newtype! {
    /// Тело запроса в JSON (аргумент) или JSON-ответ (возвращаемое значение).
    ///
    /// ```
    /// use rustyapi::prelude::*;
    ///
    /// #[schema]
    /// struct Item {
    ///     #[field(min_length = 1)]
    ///     name: String,
    /// }
    ///
    /// #[post("/items")]
    /// async fn create_item(Json(item): Json<Item>) -> Json<Item> {
    ///     Json(item)
    /// }
    /// ```
    Json
}

newtype! {
    /// Параметры пути: `Path<i64>`, `Path<(String, i64)>` или `#[schema]`-структура.
    Path
}

newtype! {
    /// Параметры строки запроса: `Query<Params>`, где `Params` — `#[schema]`-структура.
    Query
}

newtype! {
    /// Тело формы `application/x-www-form-urlencoded`: `Form<Login>`.
    Form
}

/// Подставляет в `input` исходные значения из запроса (без префикса `loc`).
fn with_raw_input(err: ValidationError, lookup: &Lookup<'_>) -> ValidationError {
    ValidationError::new(
        err.into_errors()
            .into_iter()
            .map(|mut error| {
                if let Some(raw) = error.loc.get(1..).and_then(|loc| lookup.at(loc)) {
                    error.input = raw;
                }
                error
            })
            .collect(),
    )
}

fn validated<T: Validate>(value: T, prefix: &'static str) -> Result<T, ValidationError> {
    let mut errors = Vec::new();
    value.validate_into(&mut vec![LocItem::from(prefix)], &mut errors);
    if errors.is_empty() {
        Ok(value)
    } else {
        Err(ValidationError::new(errors))
    }
}

// ---------------------------------------------------------------------------------------------
// Json
// ---------------------------------------------------------------------------------------------

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let json_content = json_content_type(request.headers());
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(body_rejection)?;
        let value = parse_json::<T>(&bytes, json_content).map_err(IntoResponse::into_response)?;
        validated(value, "body").map(Json).map_err(|err| {
            // Pydantic показывает исходное значение (`0`), а не разобранное (`0.0`).
            let root: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            with_raw_input(err, &Lookup::Json(&root)).into_response()
        })
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// `true` — тело нужно разбирать как JSON (как в FastAPI: нет content-type или `*/json`, `*+json`).
fn json_content_type(headers: &HeaderMap) -> bool {
    let Some(content_type) = headers.get(header::CONTENT_TYPE) else {
        return true;
    };
    let Ok(content_type) = content_type.to_str() else {
        return false;
    };
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match essence.split_once('/') {
        Some((_, subtype)) => subtype == "json" || subtype.ends_with("+json"),
        None => false,
    }
}

fn body_rejection(rejection: BytesRejection) -> Response {
    let status = rejection.status();
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        HTTPException::from_status(status).into_response()
    } else {
        HTTPException::new(
            StatusCode::BAD_REQUEST,
            "There was an error parsing the body",
        )
        .into_response()
    }
}

/// Разбор JSON-тела: быстрый путь — сразу в `T`; при ошибке тело ещё раз читается в `Value`,
/// чтобы заполнить `input`, как это делает Pydantic.
fn parse_json<T: DeserializeOwned>(bytes: &[u8], json_content: bool) -> Result<T, ValidationError> {
    let body = vec![LocItem::from("body")];
    if !json_content {
        // FastAPI не разбирает тело с чужим content-type, и модель получает «сырые» байты.
        return Err(ValidationError::single(ErrorDetail::new(
            "model_attributes_type",
            body,
            "Input should be a valid dictionary or object to extract fields from",
            Value::String(String::from_utf8_lossy(bytes).into_owned()),
        )));
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        // Пустое тело — как отсутствующее поле; `Json<Option<T>>` получит `None`.
        return serde_json::from_value::<T>(Value::Null).map_err(|_| {
            ValidationError::single(ErrorDetail::new(
                "missing",
                body,
                "Field required",
                Value::Null,
            ))
        });
    }
    let mut de = serde_json::Deserializer::from_slice(bytes);
    match serde_path_to_error::deserialize::<_, T>(&mut de) {
        Ok(value) => match de.end() {
            Ok(()) => Ok(value),
            Err(err) => Err(ValidationError::single(json_invalid(bytes, &err))),
        },
        Err(err) => {
            let path = err.path().clone();
            let err = err.into_inner();
            if err.is_syntax() || err.is_eof() {
                return Err(ValidationError::single(json_invalid(bytes, &err)));
            }
            let root: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
            let message = strip_position(&err);
            Err(ValidationError::single(serde_error(
                body,
                &path,
                &message,
                Lookup::Json(&root),
            )))
        }
    }
}

/// `{"type": "json_invalid", "loc": ["body", <позиция>], "msg": "JSON decode error"}` как в FastAPI.
fn json_invalid(bytes: &[u8], err: &serde_json::Error) -> ErrorDetail {
    let text = String::from_utf8_lossy(bytes);
    let line_start: usize = text
        .split_inclusive('\n')
        .take(err.line().saturating_sub(1))
        .map(|line| line.chars().count())
        .sum();
    let column = if err.is_eof() {
        err.column()
    } else {
        err.column().saturating_sub(1)
    };
    let message = strip_position(err);
    let python_message = match message.as_str() {
        "expected value" | "EOF while parsing a value" => "Expecting value",
        "key must be a string" | "EOF while parsing an object" => {
            "Expecting property name enclosed in double quotes"
        }
        "expected `:`" => "Expecting ':' delimiter",
        "expected `,` or `}`" | "expected `,` or `]`" | "EOF while parsing a list" => {
            "Expecting ',' delimiter"
        }
        "trailing characters" => "Extra data",
        "trailing comma" => "Illegal trailing comma before end of object",
        "EOF while parsing a string" => "Unterminated string starting at",
        other => other,
    }
    .to_owned();
    ErrorDetail::new(
        "json_invalid",
        vec![LocItem::from("body"), LocItem::Index(line_start + column)],
        "JSON decode error",
        json!({}),
    )
    .with_ctx(json!({ "error": python_message }))
}

/// Сообщение serde_json без ` at line X column Y`.
fn strip_position(err: &serde_json::Error) -> String {
    let message = err.to_string();
    let suffix = format!(" at line {} column {}", err.line(), err.column());
    message
        .strip_suffix(&suffix)
        .map_or(message.clone(), str::to_owned)
}

/// Откуда брать `input` для ошибки.
enum Lookup<'a> {
    /// Разобранное JSON-тело.
    Json(&'a Value),
    /// Пары `key=value` из строки запроса или формы.
    Pairs(&'a [(String, String)]),
}

impl Lookup<'_> {
    fn at(&self, loc: &[LocItem]) -> Option<Value> {
        match self {
            Self::Json(root) => {
                let mut current = *root;
                for item in loc {
                    current = match item {
                        LocItem::Key(key) => current.get(key.as_ref())?,
                        LocItem::Index(index) => current.get(*index)?,
                    };
                }
                Some(current.clone())
            }
            Self::Pairs(pairs) => match loc {
                [] => Some(Value::Object(
                    pairs
                        .iter()
                        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                        .collect(),
                )),
                [LocItem::Key(key)] => pairs
                    .iter()
                    .rev()
                    .find(|(k, _)| k == key.as_ref())
                    .map(|(_, v)| Value::String(v.clone())),
                _ => None,
            },
        }
    }
}

fn path_loc(path: &serde_path_to_error::Path) -> Loc {
    use serde_path_to_error::Segment;
    path.iter()
        .filter_map(|segment| match segment {
            Segment::Seq { index } => Some(LocItem::Index(*index)),
            Segment::Map { key } => Some(LocItem::from(key.clone())),
            Segment::Enum { variant } => Some(LocItem::from(variant.clone())),
            Segment::Unknown => None,
        })
        .collect()
}

/// Ошибка serde → ошибка Pydantic.
fn serde_error(
    prefix: Loc,
    path: &serde_path_to_error::Path,
    message: &str,
    lookup: Lookup<'_>,
) -> ErrorDetail {
    let inner = path_loc(path);
    let mut loc = prefix;
    loc.extend(inner.iter().cloned());

    if let Some(field) = between(message, "missing field `", "`") {
        // Pydantic: input — объект, в котором не хватает поля (для query/form — null).
        let input = match lookup {
            Lookup::Json(_) => lookup.at(&inner).unwrap_or(Value::Null),
            Lookup::Pairs(_) => Value::Null,
        };
        loc.push(LocItem::from(field.to_owned()));
        return ErrorDetail::new("missing", loc, "Field required", input);
    }
    let input = lookup.at(&inner).unwrap_or(Value::Null);
    classify(loc, message, input)
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(start)?;
    rest.find(end).map(|i| &rest[..i])
}

const INT_TYPES: &[&str] = &[
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
];

/// Классифицирует сообщение serde (или ошибку разбора строки) в тип ошибки Pydantic.
fn classify(loc: Loc, message: &str, input: Value) -> ErrorDetail {
    // unknown variant `x`, expected one of `a`, `b`
    if message.starts_with("unknown variant `") {
        // serde: "expected `a`", "expected `a` or `b`", "expected one of `a`, `b`, `c`".
        let expected: Vec<String> = message
            .split_once("expected ")
            .map(|(_, rest)| {
                rest.split('`')
                    .skip(1)
                    .step_by(2)
                    .map(|v| format!("'{v}'"))
                    .collect()
            })
            .unwrap_or_default();
        let expected = join_or(&expected);
        return ErrorDetail::new("enum", loc, format!("Input should be {expected}"), input)
            .with_ctx(json!({ "expected": expected }));
    }

    let (unexpected, expected) = if let Some(rest) = message.strip_prefix("invalid type: ") {
        rest.split_once(", expected ")
            .map_or((rest, ""), |(u, e)| (u, e))
    } else if let Some(rest) = message.strip_prefix("invalid value: ") {
        rest.split_once(", expected ")
            .map_or((rest, ""), |(u, e)| (u, e))
    } else {
        ("", "")
    };
    let from_string = unexpected.starts_with("string") || matches!(input, Value::String(_));

    let is_int = INT_TYPES.contains(&expected)
        || matches!(
            message,
            "invalid digit found in string"
                | "cannot parse integer from empty string"
                | "number too large to fit in target type"
                | "number too small to fit in target type"
        );
    if is_int {
        if message.starts_with("invalid value: integer") {
            return ErrorDetail::new(
                "int_parsing_size",
                loc,
                "Input should be a valid integer, unable to parse input as an integer",
                input,
            );
        }
        if unexpected.starts_with("floating point") {
            return ErrorDetail::new(
                "int_from_float",
                loc,
                "Input should be a valid integer, got a number with a fractional part",
                input,
            );
        }
        if from_string {
            return ErrorDetail::new(
                "int_parsing",
                loc,
                "Input should be a valid integer, unable to parse string as an integer",
                input,
            );
        }
        return ErrorDetail::new("int_type", loc, "Input should be a valid integer", input);
    }

    if matches!(expected, "f32" | "f64") || message == "invalid float literal" {
        if from_string {
            return ErrorDetail::new(
                "float_parsing",
                loc,
                "Input should be a valid number, unable to parse string as a number",
                input,
            );
        }
        return ErrorDetail::new("float_type", loc, "Input should be a valid number", input);
    }

    if expected == "a boolean" || message == "provided string was not `true` or `false`" {
        if from_string {
            return ErrorDetail::new(
                "bool_parsing",
                loc,
                "Input should be a valid boolean, unable to interpret input",
                input,
            );
        }
        return ErrorDetail::new("bool_type", loc, "Input should be a valid boolean", input);
    }

    let simple = match expected {
        "a string" | "a borrowed string" | "a character" => {
            Some(("string_type", "Input should be a valid string"))
        }
        "a sequence" | "a tuple" => Some(("list_type", "Input should be a valid list")),
        "a map" => Some(("dict_type", "Input should be a valid dictionary")),
        "unit" => Some(("none_required", "Input should be None")),
        _ if expected.starts_with("struct ") => Some((
            "model_attributes_type",
            "Input should be a valid dictionary or object to extract fields from",
        )),
        _ if expected.starts_with("tuple") || expected.starts_with("an array") => {
            Some(("list_type", "Input should be a valid list"))
        }
        _ if expected.contains("variant") || expected.starts_with("enum ") => {
            Some(("enum", "Input should be a valid enumeration member"))
        }
        _ => None,
    };
    if let Some((type_, msg)) = simple {
        return ErrorDetail::new(type_, loc, msg, input);
    }

    if message.starts_with("invalid length ") {
        return ErrorDetail::new("list_type", loc, "Input should be a valid list", input);
    }

    let lowered = message.to_ascii_lowercase();
    if lowered.contains("uuid") {
        return ErrorDetail::new(
            "uuid_parsing",
            loc,
            format!("Input should be a valid UUID, {message}"),
            input,
        )
        .with_ctx(json!({ "error": message }));
    }
    if expected.contains("date") || lowered.contains("premature end of input") {
        return ErrorDetail::new(
            "datetime_from_date_parsing",
            loc,
            format!("Input should be a valid datetime or date, {message}"),
            input,
        )
        .with_ctx(json!({ "error": message }));
    }
    ErrorDetail::value_error(loc, message, input)
}

/// `'a' or 'b'`, `'a', 'b' or 'c'` — как перечисляет варианты Pydantic.
fn join_or(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

// ---------------------------------------------------------------------------------------------
// Path
// ---------------------------------------------------------------------------------------------

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Validate + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => validated(value, "path")
                .map(Path)
                .map_err(IntoResponse::into_response),
            Err(rejection) => {
                let names: Vec<String> = RawPathParams::from_request_parts(parts, state)
                    .await
                    .map(|params| params.iter().map(|(k, _)| k.to_owned()).collect())
                    .unwrap_or_default();
                Err(path_rejection(&rejection, &names))
            }
        }
    }
}

fn path_rejection(rejection: &PathRejection, names: &[String]) -> Response {
    use axum::extract::path::ErrorKind;

    let PathRejection::FailedToDeserializePathParams(failed) = rejection else {
        tracing::error!("{rejection}");
        return crate::error::internal_server_error();
    };
    let at = |name: &str| vec![LocItem::from("path"), LocItem::from(name.to_owned())];
    let error = match failed.kind() {
        ErrorKind::ParseErrorAtKey {
            key,
            value,
            expected_type,
        } => classify_scalar(at(key), value, expected_type),
        ErrorKind::ParseErrorAtIndex {
            index,
            value,
            expected_type,
        } => {
            let name = names.get(*index).map_or("", String::as_str);
            classify_scalar(at(name), value, expected_type)
        }
        ErrorKind::ParseError {
            value,
            expected_type,
        } => {
            let name = names.first().map_or("", String::as_str);
            classify_scalar(at(name), value, expected_type)
        }
        ErrorKind::DeserializeError {
            key,
            value,
            message,
        } => classify(at(key), message, Value::String(value.clone())),
        ErrorKind::InvalidUtf8InPathParam { key } => ErrorDetail::new(
            "string_unicode",
            at(key),
            "Input should be a valid string, unable to parse raw data as a unicode string",
            Value::Null,
        ),
        ErrorKind::Message(message) => {
            let name = names.first().map_or("", String::as_str);
            let value = Value::Null;
            if let Some(field) = between(message, "missing field `", "`") {
                ErrorDetail::new("missing", at(field), "Field required", value)
            } else {
                classify(at(name), message, value)
            }
        }
        other => {
            tracing::error!("{other}");
            return crate::error::internal_server_error();
        }
    };
    ValidationError::single(error).into_response()
}

/// Ошибка разбора строкового значения (путь) в тип `expected_type`.
fn classify_scalar(loc: Loc, value: &str, expected_type: &str) -> ErrorDetail {
    let short = expected_type.rsplit("::").next().unwrap_or(expected_type);
    let message = match short {
        t if INT_TYPES.contains(&t) => format!("invalid type: string, expected {t}"),
        "f32" | "f64" => format!("invalid type: string, expected {short}"),
        "bool" => "provided string was not `true` or `false`".to_owned(),
        "char" => "invalid type: string, expected a character".to_owned(),
        _ => format!("invalid value: string, expected {short}"),
    };
    classify(loc, &message, Value::String(value.to_owned()))
}

// ---------------------------------------------------------------------------------------------
// Query / Form
// ---------------------------------------------------------------------------------------------

/// Разбор и проверка `key=value&...` (строка запроса или форма).
fn parse_urlencoded<T: DeserializeOwned + Validate>(
    input: &[u8],
    prefix: &'static str,
) -> Result<T, ValidationError> {
    let pairs = || -> Vec<(String, String)> {
        form_urlencoded::parse(input)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    };
    let de = serde_urlencoded::Deserializer::new(form_urlencoded::parse(input));
    let value = serde_path_to_error::deserialize::<_, T>(de).map_err(|err| {
        let pairs = pairs();
        let path = err.path().clone();
        let message = err.into_inner().to_string();
        ValidationError::single(serde_error(
            vec![LocItem::from(prefix)],
            &path,
            &message,
            Lookup::Pairs(&pairs),
        ))
    })?;
    validated(value, prefix).map_err(|err| with_raw_input(err, &Lookup::Pairs(&pairs())))
}

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned + Validate + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or("");
        parse_urlencoded::<T>(query.as_bytes(), "query")
            .map(Query)
            .map_err(IntoResponse::into_response)
    }
}

impl<T, S> FromRequest<S> for Form<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        if request.method() == axum::http::Method::GET
            || request.method() == axum::http::Method::HEAD
        {
            let query = request.uri().query().unwrap_or("").to_owned();
            return parse_urlencoded::<T>(query.as_bytes(), "body")
                .map(Form)
                .map_err(IntoResponse::into_response);
        }
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(body_rejection)?;
        parse_urlencoded::<T>(&bytes, "body")
            .map(Form)
            .map_err(IntoResponse::into_response)
    }
}

impl<T: Serialize> IntoResponse for Form<T> {
    fn into_response(self) -> Response {
        axum::Form(self.0).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join() {
        assert_eq!(join_or(&["'a'".into()]), "'a'");
        assert_eq!(join_or(&["'a'".into(), "'b'".into()]), "'a' or 'b'");
        assert_eq!(
            join_or(&["'a'".into(), "'b'".into(), "'c'".into()]),
            "'a', 'b' or 'c'"
        );
    }

    #[test]
    fn content_types() {
        let mut headers = HeaderMap::new();
        assert!(json_content_type(&headers));
        headers.insert(
            header::CONTENT_TYPE,
            "application/json; charset=utf-8".parse().unwrap(),
        );
        assert!(json_content_type(&headers));
        headers.insert(
            header::CONTENT_TYPE,
            "application/vnd.api+json".parse().unwrap(),
        );
        assert!(json_content_type(&headers));
        headers.insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());
        assert!(!json_content_type(&headers));
    }
}
