# The `tez` CLI

```sh
cargo install --path rustyapi-cli    # until 0.1.0 is on crates.io
```

| Command | What it does |
|---|---|
| `tez new myapp` | project: `src/{main,lib}.rs`, `routers/`, `models/`, `migrations/`, `tests/`, `.env` with a random `SECRET_KEY`, `docker-compose.yml` (Postgres) |
| `tez dev` | build, run, restart on changes to `*.rs`, `Cargo.toml`, `migrations/*.sql`, `.env*` |
| `tez run` | `cargo build --release` and run |
| `tez migrate new <name>` | `migrations/<time>_<name>.up.sql` + `.down.sql` |
| `tez migrate run` / `revert` / `status` | apply / revert the last one / list |
| `tez generate crud post` | `Post` model, `/posts` router (5 endpoints), migration, test; the router is registered in `src/lib.rs` |
| `tez routes` | endpoint table (`--json` for scripts) |

## tez dev

Like `uvicorn --reload`, but aware of compilation:

- the old server keeps answering while `cargo build` runs;
- after a successful build the old process gets SIGTERM (kill after 5 s), then the new one starts;
- on a compile error cargo prints it as usual and the server stays on the previous version until the
  next change;
- changes are debounced for 300 ms; `target/`, `.git/` and editor temp files are ignored.

If a rebuild takes longer than 5 seconds, `tez` suggests the mold linker once.

Pass app arguments after `--`: `tez dev -- --flag`. In a workspace with several binaries:
`tez dev -p api --bin server`.

## tez routes

Builds the app and runs it with `RUSTYAPI_PRINT=routes`: `App::run()` prints the routes and exits
without starting the server or touching the database. `RUSTYAPI_PRINT=openapi` prints the OpenAPI
document.

## Migrations

`DATABASE_URL` comes from the environment, `.env` or `--database-url`. If the directory already has
simple sqlx migrations (`0001_init.sql`, no `.up`/`.down`), `migrate new` creates a simple one too.
The version is the creation time, always greater than the existing ones.
