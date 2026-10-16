# Выпуск версии

Публикация на crates.io необратима: версию нельзя перезаписать, только отозвать (`cargo yank`).
Поэтому публикацию запускает владелец репозитория: вручную или workflow **Release** с галочкой.

## 0.1.0

1. Ветка релиза смержена в `main`, CI зелёный.
2. `tez new` по умолчанию ставит `rustyapi` той же версии с crates.io, что и сам `tez`
   (`env!("CARGO_PKG_VERSION")` в `rustyapi-cli/src/scaffold.rs`), README и туториалы ставят CLI
   через `cargo install rustyapi-cli` — для 0.1.0 уже сделано.
3. Проверка без загрузки:
   ```bash
   cargo publish --dry-run -p rustyapi-macros -p rustyapi -p rustyapi-cli
   ```
4. Тег `v0.1.0` и GitHub-релиз: Actions → **Release** → Run workflow на `main` без галочки.
   Текст релиза берётся из раздела `[0.1.0]` CHANGELOG.md.
5. Публикация на crates.io — один из двух путей:
   * из CI: добавить секрет `CARGO_REGISTRY_TOKEN` (токен с https://crates.io/settings/tokens, scope
     `publish-new`, `publish-update`) в Settings → Secrets and variables → Actions и снова запустить
     **Release**, поставив галочку «Опубликовать на crates.io»; уже созданный релиз workflow не трогает;
   * локально: `cargo login`, затем строго по порядку зависимостей (cargo сам ждёт, пока версия
     появится в индексе):
     ```bash
     cargo publish -p rustyapi-macros
     cargo publish -p rustyapi
     cargo publish -p rustyapi-cli
     ```
6. Проверка из чистого окружения:
   ```bash
   cargo install rustyapi-cli && tez new hello && cd hello && cargo test
   ```
7. Анонсы (Telegram, Reddit r/rust, Habr) — после того как `cargo install rustyapi-cli` работает.

## Следующие версии

* Версия меняется в одном месте — `[workspace.package]` и `[workspace.dependencies]` корневого
  `Cargo.toml`.
* До 1.0 несовместимые изменения API поднимают минорную версию (0.1 → 0.2).
* В CHANGELOG.md раздел `[Unreleased]` переименовывается в версию с датой.
