# Выпуск версии

Публикация на crates.io необратима: версию нельзя перезаписать, только отозвать (`cargo yank`).
Поэтому каждый шаг ниже выполняется вручную владельцем репозитория.

## 0.1.0

1. Ветка релиза смержена в `main`, CI зелёный.
2. `tez new` по умолчанию берёт RustyAPI из git. Перед публикацией переключить на crates.io:
   в `rustyapi-cli/src/scaffold.rs` ветка `(None, None)` — `format!("{:?}", env!("CARGO_PKG_VERSION"))`
   вместо git-зависимости; поправить README (`cargo install rustyapi-cli`, `rustyapi = "0.1"`).
   Закоммитить, дождаться зелёного CI.
3. Проверка без загрузки:
   ```bash
   cargo publish --dry-run -p rustyapi-macros -p rustyapi -p rustyapi-cli
   ```
4. Вход: `cargo login` с токеном с https://crates.io/settings/tokens (scope `publish-new`, `publish-update`).
5. Публикация — строго по порядку зависимостей (cargo сам ждёт, пока версия появится в индексе):
   ```bash
   cargo publish -p rustyapi-macros
   cargo publish -p rustyapi
   cargo publish -p rustyapi-cli
   ```
6. Проверка из чистого окружения:
   ```bash
   cargo install rustyapi-cli && tez new hello && cd hello && cargo test
   ```
7. Тег и релиз на GitHub:
   ```bash
   git tag -a v0.1.0 -m "RustyAPI 0.1.0" && git push origin v0.1.0
   ```
   Текст релиза — раздел `[0.1.0]` из CHANGELOG.md.
8. Анонсы (Telegram, Reddit r/rust, Habr) — после того как `cargo install rustyapi-cli` работает.

## Следующие версии

* Версия меняется в одном месте — `[workspace.package]` и `[workspace.dependencies]` корневого
  `Cargo.toml`.
* До 1.0 несовместимые изменения API поднимают минорную версию (0.1 → 0.2).
* В CHANGELOG.md раздел `[Unreleased]` переименовывается в версию с датой.
