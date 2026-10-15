//! `tez migrate new|run|revert|status`: миграции sqlx в каталоге `migrations/`.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

use crate::Result;
use crate::term;

#[derive(Debug, Subcommand)]
pub enum MigrateCommand {
    /// Создать миграцию: `migrations/<время>_<имя>.up.sql` и `.down.sql`.
    New {
        /// Имя миграции, например `create_posts`.
        name: String,
        #[command(flatten)]
        dir: DirArgs,
    },
    /// Применить все новые миграции.
    Run(DirArgs),
    /// Откатить последнюю применённую миграцию.
    Revert(DirArgs),
    /// Показать, какие миграции применены.
    Status(DirArgs),
}

#[derive(Debug, Args)]
pub struct DirArgs {
    /// Каталог с миграциями.
    #[arg(long, default_value = "migrations")]
    source: PathBuf,
    /// URL базы (по умолчанию DATABASE_URL из окружения или .env).
    #[arg(long)]
    database_url: Option<String>,
}

pub fn run(command: MigrateCommand) -> Result<()> {
    let _ = dotenvy::dotenv();
    match command {
        MigrateCommand::New { name, dir } => {
            let created = new_migration(&dir.source, &name, chrono::Utc::now())?;
            for path in created {
                term::status(&format!("{} {}", term::green("создано"), path.display()));
            }
            Ok(())
        }
        MigrateCommand::Run(dir) => block_on(apply(dir)),
        MigrateCommand::Revert(dir) => block_on(revert(dir)),
        MigrateCommand::Status(dir) => block_on(status(dir)),
    }
}

fn block_on<F: Future<Output = Result<()>>>(future: F) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future)
}

/// Имя миграции в snake_case: латиница, цифры и `_`.
fn check_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "имя миграции {name:?}: только строчная латиница, цифры и `_` (например create_posts)"
        )
        .into())
    }
}

/// Создаёт файлы миграции. Если в каталоге уже есть простые (необратимые) миграции sqlx —
/// создаётся такая же простая, иначе пара `.up.sql` / `.down.sql`.
pub fn new_migration(
    source: &Path,
    name: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<PathBuf>> {
    check_name(name)?;
    std::fs::create_dir_all(source)?;
    let names: Vec<String> = std::fs::read_dir(source)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".sql"))
        .collect();
    let simple = names
        .iter()
        .any(|n| !n.ends_with(".up.sql") && !n.ends_with(".down.sql"));
    let suffix = format!("_{name}.");
    if let Some(existing) = names.iter().find(|n| n.contains(&suffix)) {
        return Err(format!("миграция {name} уже есть: {existing}").into());
    }
    // Версия — время создания, но строго больше существующих (две миграции за одну секунду).
    let latest = names
        .iter()
        .filter_map(|n| n.split('_').next()?.parse::<i64>().ok())
        .max()
        .unwrap_or(0);
    let version = now
        .format("%Y%m%d%H%M%S")
        .to_string()
        .parse::<i64>()
        .unwrap_or(0)
        .max(latest + 1);
    let files: Vec<(PathBuf, String)> = if simple {
        vec![(
            source.join(format!("{version}_{name}.sql")),
            format!("-- {name}\n"),
        )]
    } else {
        vec![
            (
                source.join(format!("{version}_{name}.up.sql")),
                format!("-- {name}\n"),
            ),
            (
                source.join(format!("{version}_{name}.down.sql")),
                format!("-- откат {name}\n"),
            ),
        ]
    };
    for (path, content) in &files {
        if path.exists() {
            return Err(format!("{} уже существует", path.display()).into());
        }
        std::fs::write(path, content)?;
    }
    Ok(files.into_iter().map(|(path, _)| path).collect())
}

async fn connect(dir: &DirArgs) -> Result<(PgPool, Migrator)> {
    let url = dir
        .database_url
        .clone()
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .ok_or("DATABASE_URL не задан (ни в окружении, ни в .env, ни через --database-url)")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let migrator = Migrator::new(dir.source.as_path()).await?;
    Ok((pool, migrator))
}

async fn applied_versions(pool: &PgPool) -> Result<Vec<i64>> {
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
        .fetch_one(pool)
        .await?;
    if !exists {
        return Ok(Vec::new());
    }
    let rows = sqlx::query("SELECT version FROM _sqlx_migrations WHERE success ORDER BY version")
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get::<i64, _>(0)).collect())
}

async fn apply(dir: DirArgs) -> Result<()> {
    let (pool, migrator) = connect(&dir).await?;
    let before = applied_versions(&pool).await?;
    migrator.run(&pool).await?;
    let after = applied_versions(&pool).await?;
    let new: Vec<_> = migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .filter(|m| after.contains(&m.version) && !before.contains(&m.version))
        .collect();
    if new.is_empty() {
        term::status("новых миграций нет");
    }
    for m in new {
        term::status(&format!(
            "{} {} {}",
            term::green("применена"),
            m.version,
            m.description
        ));
    }
    Ok(())
}

async fn revert(dir: DirArgs) -> Result<()> {
    let (pool, migrator) = connect(&dir).await?;
    let applied = applied_versions(&pool).await?;
    let Some(&last) = applied.last() else {
        term::status("нет применённых миграций");
        return Ok(());
    };
    let reversible = migrator
        .iter()
        .any(|m| m.version == last && m.migration_type.is_down_migration());
    if !reversible {
        return Err(
            format!("миграция {last} необратима (нет .down.sql): откатите её вручную").into(),
        );
    }
    let target = applied.iter().rev().nth(1).copied().unwrap_or(0);
    migrator.undo(&pool, target).await?;
    term::status(&format!("{} {last}", term::green("откачена")));
    Ok(())
}

async fn status(dir: DirArgs) -> Result<()> {
    let (pool, migrator) = connect(&dir).await?;
    let applied = applied_versions(&pool).await?;
    for m in migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
    {
        let mark = if applied.contains(&m.version) {
            term::green("✓")
        } else {
            term::yellow("○")
        };
        println!("{mark} {} {}", m.version, m.description);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tez-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn creates_reversible_pair() {
        let dir = temp("rev");
        let now = "2026-10-15T12:30:05Z".parse().unwrap();
        let files = new_migration(&dir, "create_posts", now).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "20261015123005_create_posts.up.sql",
                "20261015123005_create_posts.down.sql"
            ]
        );
        assert!(
            new_migration(&dir, "create_posts", now).is_err(),
            "не перезаписывает"
        );
        assert!(new_migration(&dir, "Bad Name", now).is_err());
        // Вторая миграция в ту же секунду получает следующую версию.
        let second = new_migration(&dir, "create_tags", now).unwrap();
        assert!(second[0].ends_with("20261015123006_create_tags.up.sql"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn follows_simple_style() {
        let dir = temp("simple");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("0001_todos.sql"), "").unwrap();
        let now = "2026-10-15T12:30:05Z".parse().unwrap();
        let files = new_migration(&dir, "add_users", now).unwrap();
        assert_eq!(files, [dir.join("20261015123005_add_users.sql")]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
