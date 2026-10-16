//! Схемы в стиле Pydantic: трейты [`Validate`] и [`ApiSchema`], ошибки валидации в формате
//! Pydantic v2 ([`ErrorDetail`], [`ValidationError`]).
//!
//! Обычно всё это генерирует атрибут `#[schema]`:
//!
//! ```
//! use rustyapi::prelude::*;
//!
//! #[schema]
//! struct ItemCreate {
//!     #[field(min_length = 1, max_length = 100)]
//!     name: String,
//!     #[field(gt = 0)]
//!     price: f64,
//!     #[field(email)]
//!     owner_email: String,
//!     tags: Vec<String>,
//!     description: Option<String>,
//! }
//!
//! let item = ItemCreate {
//!     name: String::new(),
//!     price: 0.0,
//!     owner_email: "a@example.com".into(),
//!     tags: vec![],
//!     description: None,
//! };
//! let err = item.validate().unwrap_err();
//! assert_eq!(err.errors()[0].type_, "string_too_short");
//! assert_eq!(err.errors()[1].type_, "greater_than");
//! ```

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::HTTPException;

// ---------------------------------------------------------------------------------------------
// Ошибки валидации
// ---------------------------------------------------------------------------------------------

/// Элемент `loc`: имя поля или индекс в списке.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LocItem {
    /// Имя поля / ключ.
    Key(Cow<'static, str>),
    /// Индекс в списке.
    Index(usize),
}

impl From<&'static str> for LocItem {
    fn from(key: &'static str) -> Self {
        Self::Key(Cow::Borrowed(key))
    }
}

impl From<String> for LocItem {
    fn from(key: String) -> Self {
        Self::Key(Cow::Owned(key))
    }
}

impl From<usize> for LocItem {
    fn from(index: usize) -> Self {
        Self::Index(index)
    }
}

impl fmt::Display for LocItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(key) => f.write_str(key),
            Self::Index(index) => write!(f, "{index}"),
        }
    }
}

/// Путь к значению: `["body", "tags", 0]`.
pub type Loc = Vec<LocItem>;

/// Одна ошибка в формате Pydantic v2: `{"type", "loc", "msg", "input", "ctx"}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ErrorDetail {
    /// Тип ошибки: `missing`, `string_too_short`, `greater_than`, ...
    #[serde(rename = "type")]
    pub type_: Cow<'static, str>,
    /// Где ошибка: `["body", "name"]`.
    pub loc: Loc,
    /// Сообщение: `Field required`.
    pub msg: String,
    /// Значение, которое не прошло проверку.
    pub input: Value,
    /// Параметры проверки: `{"min_length": 1}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx: Option<Map<String, Value>>,
}

impl ErrorDetail {
    /// Новая ошибка без `ctx`.
    pub fn new(
        type_: impl Into<Cow<'static, str>>,
        loc: Loc,
        msg: impl Into<String>,
        input: Value,
    ) -> Self {
        Self {
            type_: type_.into(),
            loc,
            msg: msg.into(),
            input,
            ctx: None,
        }
    }

    /// Добавляет `ctx`. Принимает `json!({...})`; не-объект игнорируется.
    pub fn with_ctx(mut self, ctx: Value) -> Self {
        if let Value::Object(map) = ctx {
            self.ctx = Some(map);
        }
        self
    }

    /// Пользовательская ошибка как `ValueError` в валидаторе Pydantic:
    /// `{"type": "value_error", "msg": "Value error, <message>"}`.
    pub fn value_error(loc: Loc, message: impl fmt::Display, input: Value) -> Self {
        let message = message.to_string();
        Self::new("value_error", loc, format!("Value error, {message}"), input)
            .with_ctx(json!({ "error": message }))
    }

    /// Добавляет префикс к `loc` (например, `"body"`).
    pub fn prefixed(mut self, prefix: &[LocItem]) -> Self {
        self.loc.splice(0..0, prefix.iter().cloned());
        self
    }
}

/// Ошибка валидации: список [`ErrorDetail`]. В ответе — `422 {"detail": [...]}`, как
/// `RequestValidationError` в FastAPI.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    errors: Vec<ErrorDetail>,
}

impl ValidationError {
    /// Ошибка из списка деталей.
    pub fn new(errors: Vec<ErrorDetail>) -> Self {
        Self { errors }
    }

    /// Ошибка из одной детали.
    pub fn single(error: ErrorDetail) -> Self {
        Self {
            errors: vec![error],
        }
    }

    /// Все ошибки.
    pub fn errors(&self) -> &[ErrorDetail] {
        &self.errors
    }

    /// Забирает список ошибок.
    pub fn into_errors(self) -> Vec<ErrorDetail> {
        self.errors
    }

    /// Добавляет префикс ко всем `loc`.
    pub fn prefixed(self, prefix: &[LocItem]) -> Self {
        Self {
            errors: self
                .errors
                .into_iter()
                .map(|error| error.prefixed(prefix))
                .collect(),
        }
    }

    /// Тело ответа: `{"detail": [...]}`.
    pub fn to_json(&self) -> Value {
        json!({ "detail": self.errors })
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.errors.len();
        write!(
            f,
            "{count} validation error{}",
            if count == 1 { "" } else { "s" }
        )?;
        for error in &self.errors {
            let loc: Vec<String> = error.loc.iter().map(ToString::to_string).collect();
            write!(
                f,
                "\n{}\n  {} [type={}]",
                loc.join("."),
                error.msg,
                error.type_
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

impl IntoResponse for ValidationError {
    fn into_response(self) -> Response {
        (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(self.to_json())).into_response()
    }
}

impl From<ValidationError> for HTTPException {
    fn from(error: ValidationError) -> Self {
        HTTPException::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            Value::Array(
                error
                    .errors
                    .into_iter()
                    .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
                    .collect(),
            ),
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Validate
// ---------------------------------------------------------------------------------------------

/// Проверка значения после десериализации (правила `#[field(...)]`). Генерируется `#[schema]`.
///
/// Для своих типов без правил достаточно пустой реализации: `impl Validate for MyType {}`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` не реализует `Validate` / `{Self}` does not implement `Validate`",
    label = "добавьте `#[schema]` к типу / add `#[schema]` to the type",
    note = "для типа без правил достаточно `impl rustyapi::schema::Validate for {Self} {{}}`"
)]
pub trait Validate {
    /// Добавляет ошибки в `errors`; `loc` — путь до текущего значения.
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        let _ = (loc, errors);
    }

    /// Проверяет значение целиком.
    fn validate(&self) -> Result<(), ValidationError> {
        let mut errors = Vec::new();
        self.validate_into(&mut Vec::new(), &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationError::new(errors))
        }
    }
}

macro_rules! noop_validate {
    ($($ty:ty),* $(,)?) => {
        $(impl Validate for $ty {})*
    };
}

noop_validate!(
    bool,
    char,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    f32,
    f64,
    String,
    str,
    (),
    Value,
    chrono::NaiveDate,
    chrono::NaiveDateTime,
    chrono::NaiveTime,
    uuid::Uuid,
);

impl<Tz: chrono::TimeZone> Validate for chrono::DateTime<Tz> {}

impl<T: Validate + ?Sized> Validate for &T {
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        (**self).validate_into(loc, errors);
    }
}

impl<T: Validate + ?Sized> Validate for Box<T> {
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        (**self).validate_into(loc, errors);
    }
}

impl<T: Validate> Validate for Option<T> {
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        if let Some(value) = self {
            value.validate_into(loc, errors);
        }
    }
}

fn validate_items<'a, T: Validate + 'a>(
    items: impl Iterator<Item = &'a T>,
    loc: &mut Loc,
    errors: &mut Vec<ErrorDetail>,
) {
    for (index, item) in items.enumerate() {
        loc.push(LocItem::Index(index));
        item.validate_into(loc, errors);
        loc.pop();
    }
}

impl<T: Validate> Validate for Vec<T> {
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        validate_items(self.iter(), loc, errors);
    }
}

impl<T: Validate> Validate for [T] {
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        validate_items(self.iter(), loc, errors);
    }
}

impl<T: Validate, S> Validate for HashSet<T, S> {
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        validate_items(self.iter(), loc, errors);
    }
}

impl<T: Validate> Validate for BTreeSet<T> {
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        validate_items(self.iter(), loc, errors);
    }
}

impl<K: fmt::Display, V: Validate, S> Validate for HashMap<K, V, S> {
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        for (key, value) in self {
            loc.push(LocItem::from(key.to_string()));
            value.validate_into(loc, errors);
            loc.pop();
        }
    }
}

impl<K: fmt::Display, V: Validate> Validate for BTreeMap<K, V> {
    fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
        for (key, value) in self {
            loc.push(LocItem::from(key.to_string()));
            value.validate_into(loc, errors);
            loc.pop();
        }
    }
}

macro_rules! tuple_validate {
    ($(($($name:ident $idx:tt),+))*) => {
        $(
            impl<$($name: Validate),+> Validate for ($($name,)+) {
                fn validate_into(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
                    $(
                        loc.push(LocItem::Index($idx));
                        self.$idx.validate_into(loc, errors);
                        loc.pop();
                    )+
                }
            }
        )*
    };
}

tuple_validate! {
    (A 0)
    (A 0, B 1)
    (A 0, B 1, C 2)
    (A 0, B 1, C 2, D 3)
    (A 0, B 1, C 2, D 3, E 4)
    (A 0, B 1, C 2, D 3, E 4, F 5)
}

// ---------------------------------------------------------------------------------------------
// ApiSchema
// ---------------------------------------------------------------------------------------------

/// Реестр именованных схем — `components.schemas` в OpenAPI.
#[derive(Debug, Default, Clone)]
pub struct Components {
    schemas: BTreeMap<String, Value>,
}

impl Components {
    /// Пустой реестр.
    pub fn new() -> Self {
        Self::default()
    }

    /// Регистрирует схему под именем `name` (если её ещё нет) и возвращает `{"$ref": ...}`.
    /// `build` вызывается один раз; рекурсивные типы поддерживаются.
    pub fn register(&mut self, name: &str, build: impl FnOnce(&mut Self) -> Value) -> Value {
        if !self.schemas.contains_key(name) {
            // Заглушка до построения: рекурсивная ссылка на себя не зациклится.
            self.schemas.insert(name.to_owned(), Value::Null);
            let schema = build(self);
            self.schemas.insert(name.to_owned(), schema);
        }
        Self::reference(name)
    }

    /// `{"$ref": "#/components/schemas/<name>"}`.
    pub fn reference(name: &str) -> Value {
        json!({ "$ref": format!("#/components/schemas/{name}") })
    }

    /// Схема по имени.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.schemas.get(name)
    }

    /// Разворачивает `$ref` на схему из реестра (остальное возвращает как есть).
    pub fn resolve<'a>(&'a self, schema: &'a Value) -> &'a Value {
        schema
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.strip_prefix("#/components/schemas/"))
            .and_then(|name| self.schemas.get(name))
            .unwrap_or(schema)
    }

    /// Добавляет готовую схему.
    pub fn insert(&mut self, name: impl Into<String>, schema: Value) {
        self.schemas.insert(name.into(), schema);
    }

    /// Все схемы.
    pub fn schemas(&self) -> &BTreeMap<String, Value> {
        &self.schemas
    }

    /// Забирает все схемы.
    pub fn into_schemas(self) -> BTreeMap<String, Value> {
        self.schemas
    }
}

/// JSON Schema типа для OpenAPI. Генерируется `#[schema]`, реализован для примитивов, `String`,
/// `Vec`, `Option`, `HashMap`, chrono, uuid, `Value`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` не реализует `ApiSchema` / `{Self}` does not implement `ApiSchema`",
    label = "добавьте `#[schema]` к типу / add `#[schema]` to the type"
)]
pub trait ApiSchema {
    /// Имя в `components.schemas` (для `#[schema]`-типов). `None` — схема встраивается.
    fn schema_name() -> Option<String> {
        None
    }

    /// Схема: встроенная или `{"$ref": ...}` (тогда определение добавляется в `components`).
    fn schema(components: &mut Components) -> Value;

    /// Обязательно ли поле этого типа (у `Option<T>` — нет).
    fn required() -> bool {
        true
    }
}

macro_rules! simple_schema {
    ($($ty:ty => $schema:tt;)*) => {
        $(
            impl ApiSchema for $ty {
                fn schema(_: &mut Components) -> Value {
                    json!($schema)
                }
            }
        )*
    };
}

simple_schema! {
    bool => {"type": "boolean"};
    i8 => {"type": "integer"};
    i16 => {"type": "integer"};
    i32 => {"type": "integer"};
    i64 => {"type": "integer"};
    i128 => {"type": "integer"};
    isize => {"type": "integer"};
    u8 => {"type": "integer"};
    u16 => {"type": "integer"};
    u32 => {"type": "integer"};
    u64 => {"type": "integer"};
    u128 => {"type": "integer"};
    usize => {"type": "integer"};
    f32 => {"type": "number"};
    f64 => {"type": "number"};
    char => {"type": "string", "minLength": 1, "maxLength": 1};
    String => {"type": "string"};
    str => {"type": "string"};
    () => {"type": "null"};
    Value => {};
    chrono::NaiveDate => {"type": "string", "format": "date"};
    chrono::NaiveDateTime => {"type": "string", "format": "date-time"};
    chrono::NaiveTime => {"type": "string", "format": "time"};
    uuid::Uuid => {"type": "string", "format": "uuid"};
}

impl<Tz: chrono::TimeZone> ApiSchema for chrono::DateTime<Tz> {
    fn schema(_: &mut Components) -> Value {
        json!({"type": "string", "format": "date-time"})
    }
}

impl<T: ApiSchema + ?Sized> ApiSchema for &T {
    fn schema_name() -> Option<String> {
        T::schema_name()
    }
    fn schema(components: &mut Components) -> Value {
        T::schema(components)
    }
    fn required() -> bool {
        T::required()
    }
}

impl<T: ApiSchema + ?Sized> ApiSchema for Box<T> {
    fn schema_name() -> Option<String> {
        T::schema_name()
    }
    fn schema(components: &mut Components) -> Value {
        T::schema(components)
    }
    fn required() -> bool {
        T::required()
    }
}

impl<T: ApiSchema> ApiSchema for Option<T> {
    fn schema(components: &mut Components) -> Value {
        json!({ "anyOf": [T::schema(components), {"type": "null"}] })
    }
    fn required() -> bool {
        false
    }
}

impl<T: ApiSchema> ApiSchema for Vec<T> {
    fn schema(components: &mut Components) -> Value {
        json!({ "type": "array", "items": T::schema(components) })
    }
}

impl<T: ApiSchema> ApiSchema for [T] {
    fn schema(components: &mut Components) -> Value {
        json!({ "type": "array", "items": T::schema(components) })
    }
}

impl<T: ApiSchema, S> ApiSchema for HashSet<T, S> {
    fn schema(components: &mut Components) -> Value {
        json!({ "type": "array", "uniqueItems": true, "items": T::schema(components) })
    }
}

impl<T: ApiSchema> ApiSchema for BTreeSet<T> {
    fn schema(components: &mut Components) -> Value {
        json!({ "type": "array", "uniqueItems": true, "items": T::schema(components) })
    }
}

impl<K, V: ApiSchema, S> ApiSchema for HashMap<K, V, S> {
    fn schema(components: &mut Components) -> Value {
        map_schema(V::schema(components))
    }
}

impl<K, V: ApiSchema> ApiSchema for BTreeMap<K, V> {
    fn schema(components: &mut Components) -> Value {
        map_schema(V::schema(components))
    }
}

/// `dict[str, V]`; для `dict[str, Any]` Pydantic пишет `additionalProperties: true`.
fn map_schema(values: Value) -> Value {
    let values = if values == json!({}) {
        Value::Bool(true)
    } else {
        values
    };
    json!({ "additionalProperties": values, "type": "object" })
}

macro_rules! tuple_schema {
    ($(($($name:ident),+))*) => {
        $(
            impl<$($name: ApiSchema),+> ApiSchema for ($($name,)+) {
                fn schema(components: &mut Components) -> Value {
                    let items = vec![$($name::schema(components)),+];
                    let len = items.len();
                    json!({
                        "type": "array",
                        "prefixItems": items,
                        "minItems": len,
                        "maxItems": len,
                    })
                }
            }
        )*
    };
}

tuple_schema! {
    (A)
    (A, B)
    (A, B, C)
    (A, B, C, D)
    (A, B, C, D, E)
    (A, B, C, D, E, F)
}

/// Заголовок в стиле Pydantic: `owner_email` → `Owner Email`.
pub fn title_case(name: &str) -> String {
    name.split(['_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------------------------
// Проверки правил #[field(...)]
// ---------------------------------------------------------------------------------------------

/// Значения, у которых есть длина: строки (символы) и коллекции (элементы).
#[diagnostic::on_unimplemented(
    message = "`min_length`/`max_length` применимы к строкам и коллекциям, а не к `{Self}` \
               / length rules apply to strings and collections"
)]
pub trait HasLength {
    /// Длина: число символов строки или элементов коллекции.
    fn length(&self) -> usize;
    /// Строка ли это (меняет тип и текст ошибки).
    fn is_string(&self) -> bool {
        false
    }
}

impl HasLength for String {
    fn length(&self) -> usize {
        self.chars().count()
    }
    fn is_string(&self) -> bool {
        true
    }
}

impl HasLength for str {
    fn length(&self) -> usize {
        self.chars().count()
    }
    fn is_string(&self) -> bool {
        true
    }
}

impl<T> HasLength for Vec<T> {
    fn length(&self) -> usize {
        self.len()
    }
}

impl<T, S> HasLength for HashSet<T, S> {
    fn length(&self) -> usize {
        self.len()
    }
}

impl<T> HasLength for BTreeSet<T> {
    fn length(&self) -> usize {
        self.len()
    }
}

impl<K, V, S> HasLength for HashMap<K, V, S> {
    fn length(&self) -> usize {
        self.len()
    }
}

impl<K, V> HasLength for BTreeMap<K, V> {
    fn length(&self) -> usize {
        self.len()
    }
}

/// Граница числового правила (`gt = 0`, `le = 1.5`).
#[derive(Debug, Clone, Copy)]
pub enum Bound {
    /// Целочисленная граница.
    Int(i128),
    /// Дробная граница.
    Float(f64),
}

impl Bound {
    fn as_f64(self) -> f64 {
        match self {
            // Потеря точности за пределами 2^53 допустима: это граница правила, а не данные.
            #[allow(clippy::cast_precision_loss)]
            Self::Int(i) => i as f64,
            Self::Float(f) => f,
        }
    }
}

/// Числа для правил `gt`, `ge`, `lt`, `le`.
#[diagnostic::on_unimplemented(
    message = "`gt`/`ge`/`lt`/`le` применимы только к числам, а не к `{Self}` \
               / numeric rules apply to numbers only"
)]
pub trait Numeric {
    /// Сравнение с границей.
    fn compare(&self, bound: Bound) -> Option<std::cmp::Ordering>;
    /// Граница в типе поля — для `ctx` (у `f64`-поля `gt = 0` даёт `0.0`, как в Pydantic).
    fn bound_json(bound: Bound) -> Value;
}

macro_rules! int_numeric {
    ($($ty:ty),*) => {
        $(
            impl Numeric for $ty {
                fn compare(&self, bound: Bound) -> Option<std::cmp::Ordering> {
                    match bound {
                        Bound::Int(b) => i128::try_from(*self).ok().map_or(
                            Some(std::cmp::Ordering::Greater),
                            |v| Some(v.cmp(&b)),
                        ),
                        #[allow(clippy::cast_precision_loss)]
                        Bound::Float(b) => (*self as f64).partial_cmp(&b),
                    }
                }
                fn bound_json(bound: Bound) -> Value {
                    match bound {
                        Bound::Int(b) => json!(b),
                        Bound::Float(b) => json!(b),
                    }
                }
            }
        )*
    };
}

int_numeric!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

macro_rules! float_numeric {
    ($($ty:ty),*) => {
        $(
            impl Numeric for $ty {
                fn compare(&self, bound: Bound) -> Option<std::cmp::Ordering> {
                    f64::from(*self).partial_cmp(&bound.as_f64())
                }
                fn bound_json(bound: Bound) -> Value {
                    json!(bound.as_f64())
                }
            }
        )*
    };
}

float_numeric!(f32, f64);

/// Строковые значения для `pattern`, `email`, `url`.
#[diagnostic::on_unimplemented(
    message = "`pattern`/`email`/`url` применимы только к строкам, а не к `{Self}` \
               / string rules apply to strings only"
)]
pub trait AsStr {
    /// Строка.
    fn as_str_value(&self) -> &str;
}

impl AsStr for String {
    fn as_str_value(&self) -> &str {
        self
    }
}

impl AsStr for str {
    fn as_str_value(&self) -> &str {
        self
    }
}

/// Не публичный API: используется кодом, который генерирует `#[schema]`.
#[doc(hidden)]
pub mod __private {
    use std::cmp::Ordering;
    use std::marker::PhantomData;

    use serde::Serialize;
    use serde_json::{Map, Value, json};

    pub use super::{ApiSchema, AsStr, Bound, Components, HasLength, Loc, LocItem, Numeric};
    use super::{ErrorDetail, Validate};

    fn input<T: Serialize + ?Sized>(value: &T) -> Value {
        serde_json::to_value(value).unwrap_or(Value::Null)
    }

    fn plural(n: usize, word: &str) -> String {
        if n == 1 {
            format!("{n} {word}")
        } else {
            format!("{n} {word}s")
        }
    }

    pub fn min_length<T: HasLength + Serialize + ?Sized>(
        value: &T,
        min: usize,
        loc: &Loc,
        errors: &mut Vec<ErrorDetail>,
    ) {
        let len = value.length();
        if len >= min {
            return;
        }
        let error = if value.is_string() {
            ErrorDetail::new(
                "string_too_short",
                loc.clone(),
                format!("String should have at least {}", plural(min, "character")),
                input(value),
            )
            .with_ctx(json!({ "min_length": min }))
        } else {
            ErrorDetail::new(
                "too_short",
                loc.clone(),
                format!(
                    "List should have at least {} after validation, not {len}",
                    plural(min, "item")
                ),
                input(value),
            )
            .with_ctx(json!({ "field_type": "List", "min_length": min, "actual_length": len }))
        };
        errors.push(error);
    }

    pub fn max_length<T: HasLength + Serialize + ?Sized>(
        value: &T,
        max: usize,
        loc: &Loc,
        errors: &mut Vec<ErrorDetail>,
    ) {
        let len = value.length();
        if len <= max {
            return;
        }
        let error = if value.is_string() {
            ErrorDetail::new(
                "string_too_long",
                loc.clone(),
                format!("String should have at most {}", plural(max, "character")),
                input(value),
            )
            .with_ctx(json!({ "max_length": max }))
        } else {
            ErrorDetail::new(
                "too_long",
                loc.clone(),
                format!(
                    "List should have at most {} after validation, not {len}",
                    plural(max, "item")
                ),
                input(value),
            )
            .with_ctx(json!({ "field_type": "List", "max_length": max, "actual_length": len }))
        };
        errors.push(error);
    }

    /// Правило сравнения: `gt`, `ge`, `lt`, `le`. `shown` — граница как в исходном коде.
    pub fn compare<T: Numeric + Serialize>(
        value: &T,
        rule: &'static str,
        bound: Bound,
        shown: &'static str,
        loc: &Loc,
        errors: &mut Vec<ErrorDetail>,
    ) {
        let ordering = value.compare(bound);
        let (ok, type_, text) = match rule {
            "gt" => (
                ordering == Some(Ordering::Greater),
                "greater_than",
                "greater than",
            ),
            "ge" => (
                matches!(ordering, Some(Ordering::Greater | Ordering::Equal)),
                "greater_than_equal",
                "greater than or equal to",
            ),
            "lt" => (ordering == Some(Ordering::Less), "less_than", "less than"),
            _ => (
                matches!(ordering, Some(Ordering::Less | Ordering::Equal)),
                "less_than_equal",
                "less than or equal to",
            ),
        };
        if ok {
            return;
        }
        let mut ctx = Map::new();
        ctx.insert(rule.to_owned(), T::bound_json(bound));
        errors.push(
            ErrorDetail::new(
                type_,
                loc.clone(),
                format!("Input should be {text} {shown}"),
                input(value),
            )
            .with_ctx(Value::Object(ctx)),
        );
    }

    pub fn pattern<T: AsStr + Serialize + ?Sized>(
        value: &T,
        regex: &regex::Regex,
        loc: &Loc,
        errors: &mut Vec<ErrorDetail>,
    ) {
        if regex.is_match(value.as_str_value()) {
            return;
        }
        let pattern = regex.as_str();
        errors.push(
            ErrorDetail::new(
                "string_pattern_mismatch",
                loc.clone(),
                format!("String should match pattern '{pattern}'"),
                input(value),
            )
            .with_ctx(json!({ "pattern": pattern })),
        );
    }

    pub fn email<T: AsStr + Serialize + ?Sized>(
        value: &T,
        loc: &Loc,
        errors: &mut Vec<ErrorDetail>,
    ) {
        if let Err(reason) = super::check_email(value.as_str_value()) {
            errors.push(
                ErrorDetail::new(
                    "value_error",
                    loc.clone(),
                    format!("value is not a valid email address: {reason}"),
                    input(value),
                )
                .with_ctx(json!({ "reason": reason })),
            );
        }
    }

    pub fn url<T: AsStr + Serialize + ?Sized>(value: &T, loc: &Loc, errors: &mut Vec<ErrorDetail>) {
        match super::check_url(value.as_str_value()) {
            Ok(()) => {}
            Err(super::UrlError::Scheme) => errors.push(
                ErrorDetail::new(
                    "url_scheme",
                    loc.clone(),
                    "URL scheme should be 'http' or 'https'",
                    input(value),
                )
                .with_ctx(json!({ "expected_schemes": "'http' or 'https'" })),
            ),
            Err(super::UrlError::Parsing(error)) => errors.push(
                ErrorDetail::new(
                    "url_parsing",
                    loc.clone(),
                    format!("Input should be a valid URL, {error}"),
                    input(value),
                )
                .with_ctx(json!({ "error": error })),
            ),
        }
    }

    // --- autoref-специализация: проверка вложенных значений, если тип реализует Validate ---

    pub struct Probe<'a, T: ?Sized>(pub &'a T);

    pub trait ViaValidate {
        fn __rustyapi_validate(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>);
    }

    impl<T: Validate + ?Sized> ViaValidate for &Probe<'_, T> {
        fn __rustyapi_validate(&self, loc: &mut Loc, errors: &mut Vec<ErrorDetail>) {
            self.0.validate_into(loc, errors);
        }
    }

    pub trait NoValidate {
        fn __rustyapi_validate(&self, _loc: &mut Loc, _errors: &mut Vec<ErrorDetail>) {}
    }

    impl<T: ?Sized> NoValidate for Probe<'_, T> {}

    // --- autoref-специализация: схема типа, если он реализует ApiSchema, иначе `{}` ---

    pub struct SchemaProbe<T: ?Sized>(PhantomData<T>);

    impl<T: ?Sized> SchemaProbe<T> {
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            Self(PhantomData)
        }
    }

    pub trait ViaApiSchema {
        fn __rustyapi_schema(&self, components: &mut Components) -> Value;
        fn __rustyapi_required(&self) -> bool;
    }

    impl<T: ApiSchema + ?Sized> ViaApiSchema for &SchemaProbe<T> {
        fn __rustyapi_schema(&self, components: &mut Components) -> Value {
            T::schema(components)
        }
        fn __rustyapi_required(&self) -> bool {
            T::required()
        }
    }

    pub trait NoApiSchema {
        fn __rustyapi_schema(&self, _components: &mut Components) -> Value {
            json!({})
        }
        fn __rustyapi_required(&self) -> bool {
            true
        }
    }

    impl<T: ?Sized> NoApiSchema for SchemaProbe<T> {}

    /// Правила поля для схемы OpenAPI.
    #[derive(Default)]
    pub struct Constraints {
        pub min_length: Option<usize>,
        pub max_length: Option<usize>,
        pub gt: Option<Value>,
        pub ge: Option<Value>,
        pub lt: Option<Value>,
        pub le: Option<Value>,
        pub pattern: Option<&'static str>,
        pub email: bool,
        pub url: bool,
    }

    /// Применяет правила к схеме поля. У `Option<T>` (`anyOf: [T, null]`) — к ветке `T`, как в Pydantic.
    pub fn apply_constraints(schema: &mut Value, c: &Constraints, components: &Components) {
        if let Some(Value::Array(branches)) = schema.get_mut("anyOf") {
            if let Some(first) = branches.first_mut() {
                apply_constraints(first, c, components);
            }
            return;
        }
        let kind = components
            .resolve(schema)
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if schema.get("$ref").is_some() {
            // К ссылке нельзя дописать ключи рядом в 3.0; в 3.1 можно, но оставим чистый $ref.
            return;
        }
        let Value::Object(map) = schema else { return };
        let (min_key, max_key) = if kind == "string" {
            ("minLength", "maxLength")
        } else if kind == "object" {
            ("minProperties", "maxProperties")
        } else {
            ("minItems", "maxItems")
        };
        if let Some(min) = c.min_length {
            map.insert(min_key.into(), json!(min));
        }
        if let Some(max) = c.max_length {
            map.insert(max_key.into(), json!(max));
        }
        for (key, value) in [
            ("exclusiveMinimum", &c.gt),
            ("minimum", &c.ge),
            ("exclusiveMaximum", &c.lt),
            ("maximum", &c.le),
        ] {
            if let Some(value) = value {
                map.insert(key.into(), value.clone());
            }
        }
        if let Some(pattern) = c.pattern {
            map.insert("pattern".into(), json!(pattern));
        }
        if c.email {
            map.insert("format".into(), json!("email"));
        }
        if c.url {
            map.insert("format".into(), json!("uri"));
            map.entry("minLength").or_insert(json!(1));
        }
    }

    /// Граница в JSON для схемы: в типе поля, если это число (autoref-специализация).
    pub trait ViaNumeric {
        fn __rustyapi_bound(&self, bound: Bound) -> Value;
    }

    impl<T: Numeric + ?Sized> ViaNumeric for &SchemaProbe<T> {
        fn __rustyapi_bound(&self, bound: Bound) -> Value {
            T::bound_json(bound)
        }
    }

    pub trait NoNumeric {
        fn __rustyapi_bound(&self, bound: Bound) -> Value {
            match bound {
                Bound::Int(b) => json!(b),
                Bound::Float(b) => json!(b),
            }
        }
    }

    impl<T: ?Sized> NoNumeric for SchemaProbe<T> {}

    pub fn to_json<T: Serialize>(value: &T) -> Value {
        serde_json::to_value(value).unwrap_or(Value::Null)
    }

    /// `regex::Regex` для `pattern` (паттерн уже проверен макросом при компиляции).
    pub fn regex(pattern: &str) -> regex::Regex {
        regex::Regex::new(pattern).unwrap_or_else(|err| {
            panic!("некорректный pattern / invalid pattern {pattern:?}: {err}")
        })
    }

    pub use std::sync::LazyLock;
}

/// Проверка email в духе `email-validator` (тексты ошибок совпадают с Pydantic `EmailStr`).
fn check_email(value: &str) -> Result<(), &'static str> {
    let Some((local, domain)) = value.rsplit_once('@') else {
        return Err("An email address must have an @-sign.");
    };
    if local.is_empty() {
        return Err("There must be something before the @-sign.");
    }
    if domain.is_empty() {
        return Err("There must be something after the @-sign.");
    }
    if local.contains('@') {
        return Err("The email address is not valid. It must have exactly one @-sign.");
    }
    if value.chars().any(char::is_whitespace) {
        return Err("The email address contains invalid characters.");
    }
    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return Err("The email address contains invalid periods.");
    }
    if !domain.contains('.') {
        return Err("The part after the @-sign is not valid. It should have a period.");
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return Err("An email address cannot have a period immediately after the @-sign.");
    }
    let domain_ok = domain
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '.');
    if !domain_ok {
        return Err("The part after the @-sign contains invalid characters.");
    }
    if domain
        .split('.')
        .any(|label| label.starts_with('-') || label.ends_with('-'))
    {
        return Err("An email address cannot have a hyphen immediately after or before a period.");
    }
    Ok(())
}

enum UrlError {
    Scheme,
    Parsing(&'static str),
}

/// Проверка URL как `HttpUrl` в Pydantic: схема http/https и непустой хост.
fn check_url(value: &str) -> Result<(), UrlError> {
    if value.is_empty() {
        return Err(UrlError::Parsing("input is empty"));
    }
    let Some((scheme, rest)) = value.split_once(':') else {
        return Err(UrlError::Parsing("relative URL without a base"));
    };
    let scheme_ok = scheme
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok {
        return Err(UrlError::Parsing("relative URL without a base"));
    }
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(UrlError::Scheme);
    }
    let rest = rest.trim_start_matches(['/', '\\']);
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    if host.is_empty() {
        return Err(UrlError::Parsing("empty host"));
    }
    if host
        .chars()
        .any(|c| c.is_whitespace() || c == '<' || c == '>')
    {
        return Err(UrlError::Parsing("invalid international domain name"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails() {
        assert!(check_email("a@example.com").is_ok());
        assert_eq!(
            check_email("nope"),
            Err("An email address must have an @-sign.")
        );
        assert_eq!(
            check_email("a@b"),
            Err("The part after the @-sign is not valid. It should have a period.")
        );
        assert_eq!(
            check_email("@b.com"),
            Err("There must be something before the @-sign.")
        );
    }

    #[test]
    fn urls() {
        assert!(check_url("https://example.com/a?b").is_ok());
        assert!(matches!(
            check_url("notaurl"),
            Err(UrlError::Parsing("relative URL without a base"))
        ));
        assert!(matches!(check_url("ftp://x.com"), Err(UrlError::Scheme)));
        assert!(matches!(
            check_url("http://"),
            Err(UrlError::Parsing("empty host"))
        ));
    }

    #[test]
    fn titles() {
        assert_eq!(title_case("owner_email"), "Owner Email");
        assert_eq!(title_case("id"), "Id");
    }

    #[test]
    fn display() {
        let err = ValidationError::single(ErrorDetail::new(
            "missing",
            vec!["body".into(), "name".into()],
            "Field required",
            Value::Null,
        ));
        assert_eq!(
            err.to_string(),
            "1 validation error\nbody.name\n  Field required [type=missing]"
        );
    }
}
