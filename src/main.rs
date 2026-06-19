use crate::config::{Config, ConfigError};
use crate::constants::{MAIN_DIR, MODS_DIR};
use crate::game::{inject_game, remove_game_injection};
use crate::load::{load_mods, unload_mods};
use crate::mbin::{deserialize_mbin, serialize_mxml};
use crate::pak::compression::Platform;
use crate::pak::HGPAKFile;
use clap::Parser;
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use std::env::home_dir;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;

mod config;
mod constants;
mod game;
mod load;
mod mbin;
mod pak;
mod mxml;

/// 无人深空模组管理器命令行入口
#[derive(Debug, Parser)]
#[command(about = "无人深空模组管理器", version = "0.1.0")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// 支持的子命令定义
#[derive(Debug, clap::Subcommand)]
enum Commands {
    /// MBIN 序列化与反序列化
    Mbin(MbinArgs),
    /// HGPAK 解包与打包
    Pak(PakArgs),
    /// 游戏代码注入
    Inject(InjectGameArgs),
    #[command(hide = true)]
    OnGameStart,
    #[command(hide = true)]
    OnGameStop,
}

/// MBIN 操作参数
#[derive(Debug, clap::Args)]
struct MbinArgs {
    /// 操作类型
    #[arg(value_enum)]
    operation: MBinOperations,

    /// 输入文件/目录路径 (支持多个)
    #[arg(required = true)]
    paths: Vec<String>,

    /// 输出目录
    #[arg(required = true)]
    output_dir: String,
}

/// MBIN 支持的操作类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum MBinOperations {
    Serialize,
    Deserialize,
}

/// HGPAK 打包与解包参数
#[derive(Debug, clap::Args)]
struct PakArgs {
    /// 操作类型
    #[arg(value_enum)]
    operation: PakOperations,

    /// 输入路径（解包时为 PAK 文件，打包时为目录）
    #[arg(required = true)]
    path: PathBuf,

    /// 输出路径（解包时为目录，打包时为 PAK 文件）
    #[arg(required = true)]
    output_dir: PathBuf,

    /// 目标平台
    #[arg(value_enum)]
    platform: Platform,

    /// 是否启用压缩 (仅打包)
    #[arg(short, long, default_value_t = true)]
    compress: bool,
}

/// HGPAK 支持的操作类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum PakOperations {
    Unpack,
    Pack,
}

/// 游戏注入参数
#[derive(Debug, clap::Args)]
struct InjectGameArgs {
    /// 是否移除注入
    #[arg(short, long, default_value_t = false)]
    remove: bool,
}

#[tokio::main]
async fn main() {
    let main_dir = home_dir().expect("无法获取用户目录").join(MAIN_DIR);
    let mods_dir = main_dir.join(MODS_DIR);
    if !main_dir.exists() {
        fs::create_dir_all(&main_dir).expect("无法创建主目录");
    }
    if !mods_dir.exists() {
        fs::create_dir_all(&mods_dir).expect("无法创建模组目录");
    }

    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(ConfigError::ReadError(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("⚠️未找到配置文件，正在创建默认配置...",);
            let default_cfg = Config::default();
            let res = default_cfg.save();
            if let Err(e) = res {
                eprintln!("❌ 无法创建配置文件: {}", e.to_string().red());
                std::process::exit(1);
            }
            eprintln!("✅ 默认配置文件已创建: {}", "config.json".green());
            default_cfg
        }
        Err(e) => {
            eprintln!("❌ 无法加载配置文件: {}", e.to_string().red());
            std::process::exit(1);
        }
    };

    let cli = Cli::parse();

    match cli.command {
        Commands::Mbin(args) => handle_mbin(args, config.mbin_concurrency).await,
        Commands::Pak(args) => match args.operation {
            PakOperations::Unpack => handle_pak_unpack(args, config.pack_batch_chunk_count),
            PakOperations::Pack => handle_pak_repack(args, config.pack_batch_chunk_count),
        },
        Commands::Inject(args) => {
            if args.remove {
                let res = remove_game_injection(&config.game_dir);
                if let Err(e) = res {
                    eprintln!("❌ 移除注入失败: {}", e.to_string().red());
                }
            } else {
                let res = inject_game(&config.game_dir);
                if let Err(e) = res {
                    eprintln!("❌ 游戏注入失败: {}", e.to_string().red());
                }
            }
        }
        Commands::OnGameStart => {
            if !load_mods(&config.game_dir, &main_dir, &config) {
                eprintln!("{}", "❌ 无法加载模组！将启动原版游戏！".red());
            }
        }
        Commands::OnGameStop => unload_mods(&config.game_dir, &main_dir),
    }
}

/// 设置统一的进度条样式
pub fn set_pb_style(pb: &ProgressBar, name: &str) {
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                &*(format!("[{}]", name.magenta())
                    + " [{elapsed_precise}] [{bar:.cyan/blue}] {pos}/{len} ({eta}) {msg}"),
            )
            .unwrap()
            .progress_chars("=> "),
    );
}

/// 安全打印的宏定义，避免破坏进度条输出
#[macro_export]
macro_rules! pb_eprintln {
    ($pb:expr, $($arg:tt)*) => {
        $pb.suspend(|| {
            eprintln!($($arg)*);
        });
    };
}

/// 异步并发处理 MBIN 文件的序列化/反序列化
async fn handle_mbin(args: MbinArgs, concurrency: usize) {
    let total = args.paths.len();

    let pb = ProgressBar::new(total as u64);
    set_pb_style(&pb, "处理 MBIN 文件");
    pb.set_message("ℹ️ 准备处理...".blue().to_string());

    let sem = Arc::new(Semaphore::new(concurrency));
    let mut tasks = Vec::with_capacity(total);

    for path in args.paths {
        let sem = Arc::clone(&sem);
        let out_dir = args.output_dir.clone();
        let pb = pb.clone();

        let task = tokio::spawn(async move {
            pb.set_message(format!("🔍 {}", path.cyan()));
            let _guard = sem.acquire().await;
            let res = match args.operation {
                MBinOperations::Serialize => serialize_mxml(&[&path], &out_dir),
                MBinOperations::Deserialize => deserialize_mbin(&[&path], &out_dir),
            };
            if let Err(e) = res {
                pb_eprintln!(
                    pb,
                    "❌ 文件 '{}' 处理失败: {}",
                    path.yellow(),
                    e.to_string().red()
                );
            }
            pb.inc(1);
        });
        tasks.push(task);
    }

    for t in tasks {
        let _ = t.await;
    }

    pb.finish_with_message("✅ 处理完成！".green().to_string());
}

/// 执行 HGPAK 重新打包逻辑
fn handle_pak_repack(args: PakArgs, batch_chunks: usize) {
    let pb = ProgressBar::no_length();
    set_pb_style(&pb, "打包");
    pb.set_message("ℹ️ 开始打包...".blue().to_string());

    let res = HGPAKFile::repack(
        vec![(args.path, "".to_string())],
        &*args.output_dir,
        args.compress,
        args.platform,
        Some(batch_chunks),
        Some(&mut |file, count, total| {
            pb.set_position(count as u64);
            pb.set_length(total as u64);
            pb.set_message(format!("📦 {}", file.cyan()));
        }),
    );

    match res {
        Ok(_) => pb.finish_with_message("✅ 打包完成！".green().to_string()),
        Err(e) => {
            pb.finish_with_message("❌ 打包失败！".red().to_string());
            eprintln!("❌ 打包失败: {}", e.to_string().red());
        }
    }
}

/// 执行 HGPAK 解包逻辑
fn handle_pak_unpack(args: PakArgs, batch_chunks: usize) {
    let pb = ProgressBar::no_length();
    set_pb_style(&pb, "解包");
    pb.set_message("ℹ️ 开始解包...".blue().to_string());

    let pak = HGPAKFile::open(&args.path, args.platform, Some(batch_chunks));
    let pak = match pak {
        Ok(p) => p,
        Err(e) => {
            pb_eprintln!(pb, "❌ 读取文件失败: {}", e.to_string().red());
            return;
        }
    };

    let res = pak.unpack(
        &args.output_dir,
        Some(&mut |file, count, total| {
            pb.set_position(count as u64);
            pb.set_length(total as u64);
            pb.set_message(format!("📂 {}", file.cyan()));
        }),
    );

    match res {
        Ok(_) => pb.finish_with_message("✅ 解包完成！".green().to_string()),
        Err(e) => {
            pb.finish_with_message("❌ 解包失败！".red().to_string());
            eprintln!("❌ 解包失败: {}", e.to_string().red());
        }
    }
}