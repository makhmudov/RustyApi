# PostgreSQL and the ORM

`App::database_from_env()` creates a lazy sqlx pool from `DATABASE_URL`; `.migrations("migrations")`
applies sqlx migrations on startup. Handlers take `db: Db` (derefs to `PgPool`).

`#[model(table = "todos")]` = `#[schema]` + a generated `FromRow` + the `Model` trait with `get`,
`get_or_404`, `all`, `count`, `delete`, and builders `select()`, `insert()`, `update(id)`,
`delete_where()`. Column names are checked against `Model::COLUMNS`, operators against a whitelist,
and values are always bound as parameters, so SQL injection through a column name is impossible (see
`rustyapi/tests/db.rs`). `DbError` maps to 404 (not found), 409 (unique / foreign key violation) or 500.
Transactions: `db.transaction(async |tx| { ... }).await`, committed on `Ok`, rolled back on `Err`.
