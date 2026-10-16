//! PostgreSQL: пул, экстрактор [`Db`], миграции, лёгкий ORM поверх sqlx.
//!
//! ```no_run
//! use rustyapi::prelude::*;
//! use rustyapi::chrono::{DateTime, Utc};
//!
//! #[model(table = "todos")]
//! struct Todo {
//!     #[primary_key]
//!     id: i64,
//!     title: String,
//!     done: bool,
//!     owner_id: i64,
//!     created_at: DateTime<Utc>,
//! }
//!
//! #[get("/todos/{id}")]
//! async fn read_todo(db: Db, Path(id): Path<i64>) -> Result<Json<Todo>> {
//!     Ok(Json(Todo::get_or_404(&db, id).await?))
//! }
//!
//! #[get("/todos")]
//! async fn list_todos(db: Db) -> Result<Json<Vec<Todo>>> {
//!     let todos = Todo::select().eq("done", false).order_by_desc("id").limit(20).all(&db).await?;
//!     Ok(Json(todos))
//! }
//!
//! #[rustyapi::main]
//! async fn main() -> rustyapi::Result<()> {
//!     App::new()
//!         .database_from_env()
//!         .migrations("migrations")
//!         .route(read_todo)
//!         .route(list_todos)
//!         .run()
//!         .await
//! }
//! ```

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::ops::Deref;

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use sqlx::postgres::{PgArguments, PgPoolOptions, PgRow};
use sqlx::{Arguments, Encode, FromRow, PgConnection, PgPool, Postgres, Transaction, Type};

use crate::HTTPException;

pub use sqlx::PgPool as Pool;

// ---------------------------------------------------------------------------------------------
// Пул и экстрактор
// ---------------------------------------------------------------------------------------------

/// Создаёт пул по `DATABASE_URL` (подключение ленивое: при первом запросе).
pub fn pool_from_env() -> Result<PgPool, DbError> {
    let url = std::env::var("DATABASE_URL").map_err(|_| {
        DbError::Config(
            "не задан DATABASE_URL / DATABASE_URL is not set \
             (например postgres://postgres:postgres@localhost:5432/app)"
                .to_owned(),
        )
    })?;
    pool_from_url(&url)
}

/// Создаёт пул по URL (подключение ленивое).
pub fn pool_from_url(url: &str) -> Result<PgPool, DbError> {
    let max = std::env::var("DATABASE_POOL_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    PgPoolOptions::new()
        .max_connections(max)
        .connect_lazy(url)
        .map_err(DbError::Sqlx)
}

/// Применяет миграции из папки (`*.sql` в формате sqlx: `0001_init.sql` или `.up.sql`/`.down.sql`).
pub async fn migrate(pool: &PgPool, dir: &str) -> Result<(), DbError> {
    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(dir))
        .await
        .map_err(|e| DbError::Config(format!("миграции / migrations {dir:?}: {e}")))?;
    migrator
        .run(pool)
        .await
        .map_err(|e| DbError::Config(format!("миграции / migrations {dir:?}: {e}")))
}

/// Подключение к базе в обработчике (аналог `db: Session = Depends(get_db)`). Разыменовывается в
/// [`PgPool`]: `sqlx::query("...").fetch_all(&*db)`.
#[derive(Debug, Clone)]
pub struct Db(pub PgPool);

impl Deref for Db {
    type Target = PgPool;
    fn deref(&self) -> &PgPool {
        &self.0
    }
}

impl Db {
    /// Пул.
    pub fn pool(&self) -> &PgPool {
        &self.0
    }

    /// Транзакция: commit, если замыкание вернуло `Ok`, иначе rollback.
    ///
    /// ```no_run
    /// # use rustyapi::prelude::*;
    /// # async fn f(db: Db) -> rustyapi::Result<()> {
    /// let id: i64 = db
    ///     .transaction(async |tx| {
    ///         let (id,): (i64,) = rustyapi::sqlx::query_as("INSERT INTO t DEFAULT VALUES RETURNING id")
    ///             .fetch_one(&mut **tx)
    ///             .await?;
    ///         Ok::<_, rustyapi::Error>(id)
    ///     })
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn transaction<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: AsyncFnOnce(&mut Transaction<'static, Postgres>) -> Result<T, E>,
        E: From<DbError>,
    {
        transaction(&self.0, f).await
    }
}

/// Транзакция на пуле (см. [`Db::transaction`]).
pub async fn transaction<T, E, F>(pool: &PgPool, f: F) -> Result<T, E>
where
    F: AsyncFnOnce(&mut Transaction<'static, Postgres>) -> Result<T, E>,
    E: From<DbError>,
{
    let mut tx = pool.begin().await.map_err(DbError::Sqlx)?;
    match f(&mut tx).await {
        Ok(value) => {
            tx.commit().await.map_err(DbError::Sqlx)?;
            Ok(value)
        }
        Err(err) => {
            if let Err(rollback) = tx.rollback().await {
                tracing::error!("rollback failed: {rollback}");
            }
            Err(err)
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Db {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<Db>() {
            Some(db) => Ok(db.clone()),
            None => {
                tracing::error!(
                    "база не настроена: добавьте App::database_from_env() или App::database(pool) \
                     / database is not configured"
                );
                Err(crate::error::internal_server_error())
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Ошибки
// ---------------------------------------------------------------------------------------------

/// Ошибка базы. В ответе: «не найдено» → 404, нарушение unique / foreign key → 409, остальное → 500 с логом.
#[derive(Debug)]
#[non_exhaustive]
pub enum DbError {
    /// Ошибка sqlx.
    Sqlx(sqlx::Error),
    /// Запись не найдена (`detail` для ответа 404).
    NotFound(String),
    /// Неизвестная колонка в билдере запроса (защита от SQL-инъекций через имя колонки).
    UnknownColumn {
        /// Таблица.
        table: &'static str,
        /// Переданное имя.
        column: String,
    },
    /// Оператор не из белого списка.
    InvalidOperator(String),
    /// Значение не удалось закодировать.
    Encode(String),
    /// Ошибка настройки (нет `DATABASE_URL`, папки миграций).
    Config(String),
}

impl DbError {
    /// Код ошибки PostgreSQL (SQLSTATE), если есть.
    pub fn code(&self) -> Option<String> {
        match self {
            Self::Sqlx(sqlx::Error::Database(e)) => e.code().map(|c| c.into_owned()),
            _ => None,
        }
    }

    /// HTTP-статус ответа.
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound(_) | Self::Sqlx(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
            _ => match self.code().as_deref() {
                // unique_violation, foreign_key_violation, exclusion_violation
                Some("23505" | "23503" | "23P01") => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
        }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlx(e) => write!(f, "{e}"),
            Self::NotFound(detail) => f.write_str(detail),
            Self::UnknownColumn { table, column } => write!(
                f,
                "неизвестная колонка {column:?} в таблице {table:?} / unknown column"
            ),
            Self::InvalidOperator(op) => {
                write!(f, "оператор {op:?} не разрешён / operator not allowed")
            }
            Self::Encode(e) => write!(f, "не удалось закодировать значение / encode error: {e}"),
            Self::Config(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlx(e) => Some(e),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for DbError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sqlx(error)
    }
}

impl From<DbError> for HTTPException {
    fn from(error: DbError) -> Self {
        let status = error.status_code();
        match (&error, status) {
            (DbError::NotFound(detail), _) => HTTPException::new(status, detail.clone()),
            (_, StatusCode::NOT_FOUND | StatusCode::CONFLICT) => HTTPException::from_status(status),
            _ => {
                tracing::error!("Database error: {error}");
                HTTPException::from_status(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
}

impl IntoResponse for DbError {
    fn into_response(self) -> Response {
        HTTPException::from(self).into_response()
    }
}

// ---------------------------------------------------------------------------------------------
// Conn: всё, через что можно выполнить запрос
// ---------------------------------------------------------------------------------------------

/// Через что выполнять запросы ORM: `&db`, `&pool`, `&mut *conn`, `&mut tx`.
pub trait Conn<'c>: Send {
    /// Исполнитель sqlx.
    type Executor: sqlx::Executor<'c, Database = Postgres>;
    /// Превращает в исполнителя sqlx.
    fn executor(self) -> Self::Executor;
}

impl<'c> Conn<'c> for &'c Db {
    type Executor = &'c PgPool;
    fn executor(self) -> &'c PgPool {
        &self.0
    }
}

impl<'c> Conn<'c> for &'c PgPool {
    type Executor = &'c PgPool;
    fn executor(self) -> &'c PgPool {
        self
    }
}

impl<'c> Conn<'c> for &'c mut PgConnection {
    type Executor = &'c mut PgConnection;
    fn executor(self) -> &'c mut PgConnection {
        self
    }
}

impl<'c, 't> Conn<'c> for &'c mut Transaction<'t, Postgres> {
    type Executor = &'c mut PgConnection;
    fn executor(self) -> &'c mut PgConnection {
        self
    }
}

// ---------------------------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------------------------

/// Значение, которое можно передать в запрос.
pub trait SqlValue: for<'q> Encode<'q, Postgres> + Type<Postgres> + Send {}
impl<T: for<'q> Encode<'q, Postgres> + Type<Postgres> + Send> SqlValue for T {}

/// Таблица: генерируется `#[model(table = "...")]`.
pub trait Model: Sized + Send + Unpin + for<'r> FromRow<'r, PgRow> {
    /// Имя типа (для «Todo not found»).
    const NAME: &'static str;
    /// Имя таблицы.
    const TABLE: &'static str;
    /// Колонка первичного ключа.
    const PK: &'static str;
    /// Все колонки.
    const COLUMNS: &'static [&'static str];
    /// Тип первичного ключа.
    type Id: SqlValue + Clone + Sync;

    /// Запись по первичному ключу.
    fn get<'c>(
        conn: impl Conn<'c>,
        id: Self::Id,
    ) -> impl Future<Output = Result<Option<Self>, DbError>> + Send {
        Self::select().eq(Self::PK, id).first(conn)
    }

    /// Запись по первичному ключу или 404 `{"detail": "<Name> not found"}`.
    fn get_or_404<'c>(
        conn: impl Conn<'c>,
        id: Self::Id,
    ) -> impl Future<Output = Result<Self, DbError>> + Send {
        async move {
            Self::get(conn, id)
                .await?
                .ok_or_else(|| DbError::NotFound(format!("{} not found", Self::NAME)))
        }
    }

    /// Все записи (по возрастанию первичного ключа).
    fn all<'c>(conn: impl Conn<'c>) -> impl Future<Output = Result<Vec<Self>, DbError>> + Send {
        Self::select().order_by(Self::PK).all(conn)
    }

    /// Число записей.
    fn count<'c>(conn: impl Conn<'c>) -> impl Future<Output = Result<i64, DbError>> + Send {
        Self::select().count(conn)
    }

    /// Построитель `SELECT`.
    fn select() -> Select<Self> {
        Select::new()
    }

    /// Построитель `INSERT ... RETURNING *`.
    fn insert() -> Insert<Self> {
        Insert::new()
    }

    /// Построитель `UPDATE ... WHERE pk = id RETURNING *` (PATCH: только переданные поля).
    fn update(id: Self::Id) -> Update<Self> {
        Update::new(id)
    }

    /// Удаляет запись по первичному ключу; 404, если её нет.
    fn delete<'c>(
        conn: impl Conn<'c>,
        id: Self::Id,
    ) -> impl Future<Output = Result<(), DbError>> + Send {
        async move {
            let deleted = Self::delete_where().eq(Self::PK, id).execute(conn).await?;
            if deleted == 0 {
                return Err(DbError::NotFound(format!("{} not found", Self::NAME)));
            }
            Ok(())
        }
    }

    /// Построитель `DELETE ... WHERE ...`.
    fn delete_where() -> Delete<Self> {
        Delete::new()
    }
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn column_list<M: Model>() -> String {
    M::COLUMNS
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Белый список операторов сравнения.
pub const OPERATORS: &[&str] = &[
    "=",
    "<>",
    "!=",
    "<",
    "<=",
    ">",
    ">=",
    "LIKE",
    "ILIKE",
    "NOT LIKE",
    "NOT ILIKE",
    "IS DISTINCT FROM",
    "IS NOT DISTINCT FROM",
];

/// Общая часть билдеров: условия `WHERE` и аргументы.
struct Filter<M> {
    conditions: Vec<String>,
    args: PgArguments,
    error: Option<DbError>,
    _model: PhantomData<fn() -> M>,
}

impl<M: Model> Filter<M> {
    fn new() -> Self {
        Self {
            conditions: Vec::new(),
            args: PgArguments::default(),
            error: None,
            _model: PhantomData,
        }
    }

    fn fail(&mut self, error: DbError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    /// Проверяет колонку по `M::COLUMNS` и возвращает её в кавычках.
    fn column(&mut self, column: &str) -> Option<String> {
        if M::COLUMNS.contains(&column) {
            Some(quote_ident(column))
        } else {
            self.fail(DbError::UnknownColumn {
                table: M::TABLE,
                column: column.to_owned(),
            });
            None
        }
    }

    /// Добавляет аргумент и возвращает `$N`.
    fn bind(&mut self, value: impl SqlValue) -> Option<String> {
        match self.args.add(value) {
            Ok(()) => Some(format!("${}", self.args.len())),
            Err(e) => {
                self.fail(DbError::Encode(e.to_string()));
                None
            }
        }
    }

    fn compare(&mut self, column: &str, op: &str, value: impl SqlValue) {
        let normalized = op.trim().to_ascii_uppercase();
        if !OPERATORS.contains(&normalized.as_str()) {
            self.fail(DbError::InvalidOperator(op.to_owned()));
            return;
        }
        let (Some(column), Some(param)) = (self.column(column), self.bind(value)) else {
            return;
        };
        self.conditions
            .push(format!("{column} {normalized} {param}"));
    }

    fn raw_condition(&mut self, column: &str, suffix: &str) {
        if let Some(column) = self.column(column) {
            self.conditions.push(format!("{column} {suffix}"));
        }
    }

    fn is_in<T>(&mut self, column: &str, values: Vec<T>)
    where
        Vec<T>: SqlValue,
    {
        let (Some(column), Some(param)) = (self.column(column), self.bind(values)) else {
            return;
        };
        self.conditions.push(format!("{column} = ANY({param})"));
    }

    fn where_sql(&self) -> String {
        if self.conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.conditions.join(" AND "))
        }
    }

    fn check(&mut self) -> Result<(), DbError> {
        match self.error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

macro_rules! filter_methods {
    () => {
        /// `column = value`.
        pub fn eq(mut self, column: &str, value: impl SqlValue) -> Self {
            self.filter.compare(column, "=", value);
            self
        }

        /// `column <> value`.
        pub fn ne(mut self, column: &str, value: impl SqlValue) -> Self {
            self.filter.compare(column, "<>", value);
            self
        }

        /// `column < value`.
        pub fn lt(mut self, column: &str, value: impl SqlValue) -> Self {
            self.filter.compare(column, "<", value);
            self
        }

        /// `column <= value`.
        pub fn le(mut self, column: &str, value: impl SqlValue) -> Self {
            self.filter.compare(column, "<=", value);
            self
        }

        /// `column > value`.
        pub fn gt(mut self, column: &str, value: impl SqlValue) -> Self {
            self.filter.compare(column, ">", value);
            self
        }

        /// `column >= value`.
        pub fn ge(mut self, column: &str, value: impl SqlValue) -> Self {
            self.filter.compare(column, ">=", value);
            self
        }

        /// `column LIKE pattern`.
        pub fn like(mut self, column: &str, pattern: impl SqlValue) -> Self {
            self.filter.compare(column, "LIKE", pattern);
            self
        }

        /// `column ILIKE pattern` (без учёта регистра).
        pub fn ilike(mut self, column: &str, pattern: impl SqlValue) -> Self {
            self.filter.compare(column, "ILIKE", pattern);
            self
        }

        /// Произвольный оператор из белого списка [`OPERATORS`].
        pub fn filter(mut self, column: &str, op: &str, value: impl SqlValue) -> Self {
            self.filter.compare(column, op, value);
            self
        }

        /// `column IS NULL`.
        pub fn is_null(mut self, column: &str) -> Self {
            self.filter.raw_condition(column, "IS NULL");
            self
        }

        /// `column IS NOT NULL`.
        pub fn is_not_null(mut self, column: &str) -> Self {
            self.filter.raw_condition(column, "IS NOT NULL");
            self
        }

        /// `column = ANY(values)`.
        pub fn is_in<T>(mut self, column: &str, values: Vec<T>) -> Self
        where
            Vec<T>: SqlValue,
        {
            self.filter.is_in(column, values);
            self
        }

        /// Условие, только если значение есть (удобно для необязательных фильтров).
        pub fn eq_opt(self, column: &str, value: Option<impl SqlValue>) -> Self {
            match value {
                Some(value) => self.eq(column, value),
                None => self,
            }
        }
    };
}

/// `SELECT` с фильтрами, сортировкой и пагинацией.
pub struct Select<M> {
    filter: Filter<M>,
    order: Vec<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

impl<M: Model> Select<M> {
    fn new() -> Self {
        Self {
            filter: Filter::new(),
            order: Vec::new(),
            limit: None,
            offset: None,
        }
    }

    filter_methods!();

    /// `ORDER BY column ASC`.
    pub fn order_by(mut self, column: &str) -> Self {
        if let Some(column) = self.filter.column(column) {
            self.order.push(format!("{column} ASC"));
        }
        self
    }

    /// `ORDER BY column DESC`.
    pub fn order_by_desc(mut self, column: &str) -> Self {
        if let Some(column) = self.filter.column(column) {
            self.order.push(format!("{column} DESC"));
        }
        self
    }

    /// `LIMIT n`.
    pub fn limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// `OFFSET n`.
    pub fn offset(mut self, offset: i64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// SQL запроса (для отладки и тестов).
    pub fn sql(&self) -> String {
        self.build(&column_list::<M>())
    }

    fn build(&self, what: &str) -> String {
        let mut sql = format!(
            "SELECT {what} FROM {}{}",
            quote_ident(M::TABLE),
            self.filter.where_sql()
        );
        if !self.order.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&self.order.join(", "));
        }
        if let Some(limit) = self.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        if let Some(offset) = self.offset {
            sql.push_str(&format!(" OFFSET {offset}"));
        }
        sql
    }

    /// Все подходящие записи.
    pub fn all<'c>(
        self,
        conn: impl Conn<'c>,
    ) -> impl Future<Output = Result<Vec<M>, DbError>> + Send {
        let mut this = self;
        async move {
            this.filter.check()?;
            let sql = this.sql();
            Ok(sqlx::query_as_with::<_, M, _>(&sql, this.filter.args)
                .fetch_all(conn.executor())
                .await?)
        }
    }

    /// Первая подходящая запись.
    pub fn first<'c>(
        self,
        conn: impl Conn<'c>,
    ) -> impl Future<Output = Result<Option<M>, DbError>> + Send {
        let mut this = self;
        async move {
            this.filter.check()?;
            this.limit = Some(1);
            let sql = this.sql();
            Ok(sqlx::query_as_with::<_, M, _>(&sql, this.filter.args)
                .fetch_optional(conn.executor())
                .await?)
        }
    }

    /// Первая подходящая запись или 404.
    pub fn first_or_404<'c>(
        self,
        conn: impl Conn<'c>,
    ) -> impl Future<Output = Result<M, DbError>> + Send {
        let this = self;
        async move {
            this.first(conn)
                .await?
                .ok_or_else(|| DbError::NotFound(format!("{} not found", M::NAME)))
        }
    }

    /// Число подходящих записей (без учёта `LIMIT`/`OFFSET`).
    pub fn count<'c>(
        self,
        conn: impl Conn<'c>,
    ) -> impl Future<Output = Result<i64, DbError>> + Send {
        let mut this = self;
        async move {
            this.filter.check()?;
            let sql = format!(
                "SELECT COUNT(*) FROM {}{}",
                quote_ident(M::TABLE),
                this.filter.where_sql()
            );
            let (count,): (i64,) = sqlx::query_as_with(&sql, this.filter.args)
                .fetch_one(conn.executor())
                .await?;
            Ok(count)
        }
    }
}

/// `INSERT INTO table (...) VALUES (...) RETURNING ...`.
pub struct Insert<M> {
    filter: Filter<M>,
    columns: Vec<String>,
    params: Vec<String>,
}

impl<M: Model> Insert<M> {
    fn new() -> Self {
        Self {
            filter: Filter::new(),
            columns: Vec::new(),
            params: Vec::new(),
        }
    }

    /// Значение колонки.
    pub fn set(mut self, column: &str, value: impl SqlValue) -> Self {
        if let (Some(column), Some(param)) = (self.filter.column(column), self.filter.bind(value)) {
            self.columns.push(column);
            self.params.push(param);
        }
        self
    }

    /// Значение колонки, только если оно передано (иначе — `DEFAULT` из таблицы).
    pub fn set_opt(self, column: &str, value: Option<impl SqlValue>) -> Self {
        match value {
            Some(value) => self.set(column, value),
            None => self,
        }
    }

    /// SQL запроса.
    pub fn sql(&self) -> String {
        let table = quote_ident(M::TABLE);
        let returning = column_list::<M>();
        if self.columns.is_empty() {
            format!("INSERT INTO {table} DEFAULT VALUES RETURNING {returning}")
        } else {
            format!(
                "INSERT INTO {table} ({}) VALUES ({}) RETURNING {returning}",
                self.columns.join(", "),
                self.params.join(", ")
            )
        }
    }

    /// Выполняет и возвращает созданную запись.
    pub fn execute<'c>(
        self,
        conn: impl Conn<'c>,
    ) -> impl Future<Output = Result<M, DbError>> + Send {
        let mut this = self;
        async move {
            this.filter.check()?;
            let sql = this.sql();
            Ok(sqlx::query_as_with::<_, M, _>(&sql, this.filter.args)
                .fetch_one(conn.executor())
                .await?)
        }
    }
}

/// `UPDATE table SET ... WHERE pk = id RETURNING ...`.
pub struct Update<M: Model> {
    filter: Filter<M>,
    id: M::Id,
    sets: Vec<String>,
}

impl<M: Model> Update<M> {
    fn new(id: M::Id) -> Self {
        Self {
            filter: Filter::new(),
            id,
            sets: Vec::new(),
        }
    }

    /// Новое значение колонки.
    pub fn set(mut self, column: &str, value: impl SqlValue) -> Self {
        if let (Some(column), Some(param)) = (self.filter.column(column), self.filter.bind(value)) {
            self.sets.push(format!("{column} = {param}"));
        }
        self
    }

    /// Новое значение, только если оно передано — для PATCH (`Option`-поля тела запроса).
    pub fn set_opt(self, column: &str, value: Option<impl SqlValue>) -> Self {
        match value {
            Some(value) => self.set(column, value),
            None => self,
        }
    }

    /// Выполняет и возвращает обновлённую запись; 404, если её нет.
    pub fn execute<'c>(
        self,
        conn: impl Conn<'c>,
    ) -> impl Future<Output = Result<M, DbError>> + Send {
        let mut this = self;
        async move {
            this.filter.check()?;
            if this.sets.is_empty() {
                return M::get_or_404(conn, this.id).await;
            }
            let pk = quote_ident(M::PK);
            let Some(param) = this.filter.bind(this.id.clone()) else {
                this.filter.check()?;
                unreachable!()
            };
            let sql = format!(
                "UPDATE {} SET {} WHERE {pk} = {param} RETURNING {}",
                quote_ident(M::TABLE),
                this.sets.join(", "),
                column_list::<M>()
            );
            sqlx::query_as_with::<_, M, _>(&sql, this.filter.args)
                .fetch_optional(conn.executor())
                .await?
                .ok_or_else(|| DbError::NotFound(format!("{} not found", M::NAME)))
        }
    }
}

/// `DELETE FROM table WHERE ...`.
pub struct Delete<M> {
    filter: Filter<M>,
}

impl<M: Model> Delete<M> {
    fn new() -> Self {
        Self {
            filter: Filter::new(),
        }
    }

    filter_methods!();

    /// Выполняет и возвращает число удалённых строк. Без условий не выполняется (защита от
    /// случайного `DELETE` всей таблицы): используйте `.filter(...)` или SQL напрямую.
    pub fn execute<'c>(
        self,
        conn: impl Conn<'c>,
    ) -> impl Future<Output = Result<u64, DbError>> + Send {
        let mut this = self;
        async move {
            this.filter.check()?;
            if this.filter.conditions.is_empty() {
                return Err(DbError::Config(
                    "DELETE без условий запрещён / DELETE without conditions is not allowed".into(),
                ));
            }
            let sql = format!(
                "DELETE FROM {}{}",
                quote_ident(M::TABLE),
                this.filter.where_sql()
            );
            let result = sqlx::query_with(&sql, this.filter.args)
                .execute(conn.executor())
                .await?;
            Ok(result.rows_affected())
        }
    }
}

/// Не публичный API: используется кодом, который генерирует `#[model]`.
#[doc(hidden)]
pub mod __private {
    pub use sqlx::postgres::PgRow;
    pub use sqlx::{Error, FromRow, Row};
}
