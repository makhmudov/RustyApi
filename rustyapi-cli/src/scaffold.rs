//! `tez new` и `tez generate crud`: проекты и код из шаблонов (`templates/`).

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::Result;
use crate::term;

#[derive(Debug, Args)]
pub struct NewArgs {
    /// Имя проекта (каталог и crate), например `myapp`.
    name: String,
    /// Зависимость rustyapi из локального каталога (для разработки самого фреймворка).
    #[arg(long, value_name = "DIR", conflicts_with = "rustyapi_version")]
    rustyapi_path: Option<PathBuf>,
    /// Версия rustyapi с crates.io (по умолчанию — версия самого `tez`).
    #[arg(long, value_name = "VERSION")]
    rustyapi_version: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum GenerateCommand {
    /// Модель + роутер + миграция + тест: `tez generate crud post` → `/posts`.
    Crud {
        /// Имя сущности в единственном числе, snake_case: `post`, `blog_post`.
        name: String,
        /// Множественное число (по умолчанию — по правилам английского: post → posts).
        #[arg(long)]
        plural: Option<String>,
        /// Каталог проекта (по умолчанию текущий).
        #[arg(long, value_name = "DIR")]
        manifest_dir: Option<PathBuf>,
    },
    /// Клиент API из OpenAPI: `tez generate client --ts --out web/src/api.ts`.
    Client {
        /// TypeScript (пока единственный вариант).
        #[arg(long, required = true)]
        ts: bool,
        /// Куда записать.
        #[arg(long, short, default_value = "client.ts")]
        out: PathBuf,
        /// Взять схему из файла (`openapi.json`) вместо сборки приложения.
        #[arg(long, value_name = "FILE")]
        input: Option<PathBuf>,
        #[command(flatten)]
        target: crate::TargetArgs,
    },
}

/// Подстановка `{{key}}` в шаблоне.
fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (key, value) in vars {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    debug_assert!(!out.contains("{{"), "не подставлено: {out}");
    out
}

fn write_new(path: &Path, content: &str) -> Result<()> {
    if path.exists() {
        return Err(format!("{} уже существует", path.display()).into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

/// Форматирует созданные файлы через rustfmt (если он установлен), чтобы `cargo fmt --check`
/// в новом проекте проходил при любом имени сущности.
fn rustfmt(files: &[PathBuf]) {
    let rust: Vec<&PathBuf> = files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    if rust.is_empty() {
        return;
    }
    let _ = std::process::Command::new("rustfmt")
        .args(["--edition", "2024"])
        .args(rust)
        .stderr(std::process::Stdio::null())
        .status();
}

/// Проверка имени: snake_case, начинается с буквы.
fn check_ident(name: &str, what: &str) -> Result<()> {
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    const RESERVED: &[&str] = &[
        "self", "super", "crate", "mod", "type", "fn", "struct", "enum", "impl", "use", "test",
        "main", "app", "models", "routers", "common", "root",
    ];
    if !ok || RESERVED.contains(&name) {
        return Err(format!(
            "{what} {name:?}: нужен snake_case из латиницы (например my_app), не зарезервированное слово"
        )
        .into());
    }
    Ok(())
}

/// Случайный ключ для `SECRET_KEY` в `.env` (два UUID v4: 244 случайных бита).
fn random_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub fn new_project(args: NewArgs) -> Result<()> {
    let name = args.name.trim_end_matches('/');
    let dir = PathBuf::from(name);
    let crate_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("некорректное имя проекта")?
        .replace('-', "_");
    check_ident(&crate_name, "имя проекта")?;
    if dir.exists() {
        return Err(format!("каталог {} уже существует", dir.display()).into());
    }
    let dep = match (&args.rustyapi_path, &args.rustyapi_version) {
        (Some(path), _) => {
            let path = path
                .canonicalize()
                .map_err(|e| format!("{}: {e}", path.display()))?;
            format!("{{ path = {:?} }}", path.display().to_string())
        }
        (None, Some(version)) => format!("{version:?}"),
        // По умолчанию — та же версия с crates.io, что и у самого `tez`.
        (None, None) => format!("{:?}", env!("CARGO_PKG_VERSION")),
    };
    let secret = random_secret();
    let vars = [
        ("crate", crate_name.as_str()),
        ("crate_ident", crate_name.as_str()),
        ("rustyapi_dep", dep.as_str()),
        ("secret", secret.as_str()),
    ];
    let example_vars = [("crate", crate_name.as_str()), ("secret", "change-me")];
    let files: [(&str, String); 13] = [
        (
            "Cargo.toml",
            render(include_str!("../templates/project/Cargo.toml.tmpl"), &vars),
        ),
        (
            "src/main.rs",
            render(include_str!("../templates/project/main.rs.tmpl"), &vars),
        ),
        (
            "src/lib.rs",
            render(include_str!("../templates/project/lib.rs.tmpl"), &vars),
        ),
        (
            "src/routers/mod.rs",
            "//! Роутеры (`tez generate crud <name>` добавляет сюда новые).\n\npub mod root;\n"
                .into(),
        ),
        (
            "src/routers/root.rs",
            render(include_str!("../templates/project/root.rs.tmpl"), &vars),
        ),
        (
            "src/models/mod.rs",
            "//! Модели базы данных (`tez generate crud <name>` добавляет сюда новые).\n".into(),
        ),
        ("migrations/.gitkeep", String::new()),
        (
            "tests/common/mod.rs",
            render(include_str!("../templates/project/common.rs.tmpl"), &vars),
        ),
        (
            "tests/api.rs",
            render(include_str!("../templates/project/api_test.rs.tmpl"), &vars),
        ),
        (
            ".env",
            render(include_str!("../templates/project/env.tmpl"), &vars),
        ),
        (
            ".env.example",
            render(include_str!("../templates/project/env.tmpl"), &example_vars),
        ),
        (
            ".gitignore",
            include_str!("../templates/project/gitignore.tmpl").into(),
        ),
        (
            "docker-compose.yml",
            render(
                include_str!("../templates/project/docker-compose.yml.tmpl"),
                &vars,
            ),
        ),
    ];
    for (path, content) in files.iter().chain(
        [(
            "README.md",
            render(include_str!("../templates/project/README.md.tmpl"), &vars),
        )]
        .iter(),
    ) {
        write_new(&dir.join(path), content)?;
    }
    term::status(&format!(
        "{} проект {}",
        term::green("создан"),
        dir.display()
    ));
    eprintln!(
        "\n  cd {}\n  docker compose up -d\n  tez dev\n  open http://127.0.0.1:8000/docs\n",
        dir.display()
    );
    Ok(())
}

/// Множественное число по правилам английского: post → posts, category → categories, box → boxes.
pub fn pluralize(name: &str) -> String {
    let consonant_y = name.ends_with('y')
        && !name
            .chars()
            .rev()
            .nth(1)
            .is_some_and(|c| "aeiou".contains(c));
    if consonant_y {
        format!("{}ies", &name[..name.len() - 1])
    } else if ["s", "x", "z", "ch", "sh"]
        .iter()
        .any(|s| name.ends_with(s))
    {
        format!("{name}es")
    } else {
        format!("{name}s")
    }
}

/// `blog_post` → `BlogPost`.
pub fn pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|p| !p.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// Добавляет строку в файл, если её там ещё нет.
fn append_line(path: &Path, line: &str) -> Result<()> {
    let mut content = std::fs::read_to_string(path).unwrap_or_default();
    if content.lines().any(|l| l.trim() == line.trim()) {
        return Ok(());
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(line);
    content.push('\n');
    std::fs::write(path, content)?;
    Ok(())
}

pub fn generate(command: GenerateCommand) -> Result<()> {
    match command {
        GenerateCommand::Client {
            ts: _,
            out,
            input,
            target,
        } => {
            let openapi: serde_json::Value = match input {
                Some(path) => serde_json::from_str(&std::fs::read_to_string(&path)?)
                    .map_err(|e| format!("{}: {e}", path.display()))?,
                None => crate::routes::query_app(&target, "openapi")?,
            };
            let code = crate::tsclient::generate(&openapi);
            if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&out, code)?;
            let operations = openapi["paths"].as_object().map_or(0, |paths| {
                paths
                    .values()
                    .filter_map(|p| p.as_object())
                    .map(|p| p.len())
                    .sum()
            });
            term::status(&format!(
                "{} {} ({operations} операций)",
                term::green("создан"),
                out.display()
            ));
            Ok(())
        }
        GenerateCommand::Crud {
            name,
            plural,
            manifest_dir,
        } => {
            let dir = match manifest_dir {
                Some(dir) => dir,
                None => std::env::current_dir()?,
            };
            let created = generate_crud(&dir, &name, plural.as_deref(), chrono::Utc::now())?;
            for path in &created.files {
                let shown = path.strip_prefix(&dir).unwrap_or(path);
                term::status(&format!("{} {}", term::green("создано"), shown.display()));
            }
            if created.registered {
                term::status("роутер подключён в src/lib.rs");
            } else {
                term::status(&term::yellow(&format!(
                    "в src/lib.rs не найдена метка `// tez: routers`: подключите роутер сами: \
                     .include_router(routers::{}::router())",
                    created.plural
                )));
            }
            eprintln!(
                "\n  tez migrate run   # создать таблицу {}\n",
                created.plural
            );
            Ok(())
        }
    }
}

pub struct Generated {
    pub files: Vec<PathBuf>,
    pub plural: String,
    pub registered: bool,
}

pub fn generate_crud(
    dir: &Path,
    name: &str,
    plural: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Generated> {
    check_ident(name, "имя сущности")?;
    let plural = plural.map(str::to_owned).unwrap_or_else(|| pluralize(name));
    check_ident(&plural, "множественное число")?;
    if plural == name {
        return Err("имя и множественное число совпадают: задайте --plural".into());
    }
    if !dir.join("Cargo.toml").exists() || !dir.join("src").is_dir() {
        return Err(format!(
            "{} не похож на проект RustyAPI (нет Cargo.toml или src/); создайте его через `tez new`",
            dir.display()
        )
        .into());
    }
    let pascal = pascal_case(name);
    let vars = [
        ("name", name),
        ("Name", pascal.as_str()),
        ("plural", plural.as_str()),
    ];
    let model = dir.join(format!("src/models/{name}.rs"));
    let router = dir.join(format!("src/routers/{plural}.rs"));
    let test = dir.join(format!("tests/{plural}.rs"));
    for path in [&model, &router, &test] {
        if path.exists() {
            return Err(format!("{} уже существует", path.display()).into());
        }
    }
    let migrations = dir.join("migrations");
    let migration = crate::migrate::new_migration(&migrations, &format!("create_{plural}"), now)?;
    for path in &migration {
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let template = if file.ends_with(".down.sql") {
            include_str!("../templates/crud/down.sql.tmpl")
        } else {
            include_str!("../templates/crud/up.sql.tmpl")
        };
        std::fs::write(path, render(template, &vars))?;
    }
    write_new(
        &model,
        &render(include_str!("../templates/crud/model.rs.tmpl"), &vars),
    )?;
    write_new(
        &router,
        &render(include_str!("../templates/crud/router.rs.tmpl"), &vars),
    )?;
    write_new(
        &test,
        &render(include_str!("../templates/crud/test.rs.tmpl"), &vars),
    )?;
    append_line(&dir.join("src/models/mod.rs"), &format!("pub mod {name};"))?;
    append_line(
        &dir.join("src/routers/mod.rs"),
        &format!("pub mod {plural};"),
    )?;

    let lib = dir.join("src/lib.rs");
    let marker = "// tez: routers";
    let mut registered = false;
    if let Ok(source) = std::fs::read_to_string(&lib) {
        if let Some(pos) = source.find(marker) {
            let line_start = source[..pos].rfind('\n').map_or(0, |i| i + 1);
            let indent = &source[line_start..pos];
            let insert = format!("routers::{plural}::router(),\n{indent}");
            let updated = format!("{}{insert}{}", &source[..pos], &source[pos..]);
            std::fs::write(&lib, updated)?;
            registered = true;
        }
    }
    let mut files = migration;
    files.extend([model, router, test]);
    let mut touched = files.clone();
    touched.extend([
        dir.join("src/models/mod.rs"),
        dir.join("src/routers/mod.rs"),
        lib,
    ]);
    rustfmt(&touched);
    Ok(Generated {
        files,
        plural,
        registered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(pluralize("post"), "posts");
        assert_eq!(pluralize("category"), "categories");
        assert_eq!(pluralize("day"), "days");
        assert_eq!(pluralize("box"), "boxes");
        assert_eq!(pluralize("blog_post"), "blog_posts");
        assert_eq!(pascal_case("blog_post"), "BlogPost");
        assert!(check_ident("my_app", "x").is_ok());
        for bad in ["MyApp", "1app", "my-app", "", "self", "test"] {
            assert!(check_ident(bad, "x").is_err(), "{bad}");
        }
    }

    #[test]
    fn templates_are_complete() {
        let vars = [("name", "post"), ("Name", "Post"), ("plural", "posts")];
        for template in [
            include_str!("../templates/crud/model.rs.tmpl"),
            include_str!("../templates/crud/router.rs.tmpl"),
            include_str!("../templates/crud/test.rs.tmpl"),
            include_str!("../templates/crud/up.sql.tmpl"),
        ] {
            let out = render(template, &vars);
            assert!(!out.contains("{{"), "{out}");
        }
        let router = render(include_str!("../templates/crud/router.rs.tmpl"), &vars);
        assert!(router.contains("use crate::models::post::{Post, PostCreate, PostUpdate};"));
    }
}
