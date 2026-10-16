//! CLI `tez` — инструмент разработчика для RustyAPI (как `fastapi` / `uvicorn --reload`).

mod cargo;
mod dev;
mod migrate;
mod routes;
mod scaffold;
mod term;
mod tsclient;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

/// Инструмент разработчика для фреймворка RustyAPI.
#[derive(Debug, Parser)]
#[command(name = "tez", version, about, propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Создать новый проект: main.rs, routers/, models/, migrations/, .env, docker-compose, тесты.
    New(scaffold::NewArgs),
    /// Собрать, запустить и перезапускать при изменениях (src/, migrations/, .env).
    Dev(DevArgs),
    /// Release-сборка и запуск.
    Run(RunArgs),
    /// Миграции базы данных (DATABASE_URL из окружения или .env).
    #[command(subcommand)]
    Migrate(migrate::MigrateCommand),
    /// Сгенерировать код.
    #[command(subcommand)]
    Generate(scaffold::GenerateCommand),
    /// Таблица всех эндпоинтов приложения.
    Routes(RoutesArgs),
}

/// Какой бинарник собирать (как у `cargo`).
#[derive(Debug, Clone, Default, Args)]
pub struct TargetArgs {
    /// Пакет workspace.
    #[arg(short, long)]
    package: Option<String>,
    /// Бинарник пакета.
    #[arg(long)]
    bin: Option<String>,
    /// Каталог проекта (по умолчанию текущий).
    #[arg(long, value_name = "DIR")]
    manifest_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct DevArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Аргументы для приложения (после `--`).
    #[arg(last = true)]
    app_args: Vec<String>,
}

#[derive(Debug, Args)]
struct RunArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Аргументы для приложения (после `--`).
    #[arg(last = true)]
    app_args: Vec<String>,
}

#[derive(Debug, Args)]
struct RoutesArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Вывести JSON вместо таблицы.
    #[arg(long)]
    json: bool,
}

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::New(args) => scaffold::new_project(args),
        Command::Dev(args) => dev::run(args.target, args.app_args),
        Command::Run(args) => cargo::run_release(&args.target, &args.app_args),
        Command::Migrate(command) => migrate::run(command),
        Command::Generate(command) => scaffold::generate(command),
        Command::Routes(args) => routes::run(&args.target, args.json),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{} {error}", term::red("ошибка:"));
            ExitCode::FAILURE
        }
    }
}
