use crate::config::Config;
use crate::constants::{CACHE_DIR, HASH_FILE, MOD_PAK, MODS_DIR, PAK_PATH};
use crate::mbin::{deserialize_mbin, serialize_mxml};
use crate::mxml::merge_mxml;
use crate::pak::HGPAKFile;
use crate::pak::compression::Platform;
use crate::{pb_eprintln, set_pb_style};
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar};
use std::collections::HashMap;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use walkdir::WalkDir;

const BACKUP_DIR: &str = "backup";

/// 加载模组主流程：校验哈希缓存、处理模组文件并触发重新打包逻辑
pub fn load_mods(game_dir: &PathBuf, main_dir: &PathBuf, config: &Config) -> bool {
    let mods_dir = main_dir.join(MODS_DIR);
    let cache_dir = main_dir.join(CACHE_DIR);
    let pak_dir = game_dir.join(PAK_PATH);
    let backup_dir = main_dir.join(BACKUP_DIR);
    let mod_pak = pak_dir.join(MOD_PAK);

    if !pak_dir.exists() {
        eprintln!("{}", "❌ 游戏 PAK 目录不存在，无法加载模组！".red());
        return false;
    }

    if mod_pak.exists() {
        println!("{}", "ℹ️ 游戏文件备份存在，开始恢复文件...".blue());
        unload_mods(&game_dir, &main_dir);
    } else if backup_dir.exists() {
        let entries: Vec<_> = WalkDir::new(&backup_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.file_name() != ".DS_Store")
            .collect();

        if !entries.is_empty() {
            println!("{}", "ℹ️ 游戏文件备份存在，开始恢复文件...".blue());
            unload_mods(&game_dir, &main_dir);
        }
    }

    if !mods_dir.exists() || !mods_dir.is_dir() {
        eprintln!("{}", "⚠️ 未安装任何模组，跳过加载".yellow());
        return true;
    }

    let mut mod_files: Vec<PathBuf> = WalkDir::new(&mods_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.file_name() != ".DS_Store")
        .map(|e| e.path().to_path_buf())
        .collect();

    if mod_files.is_empty() {
        eprintln!("{}", "⚠️ 未安装任何模组，跳过加载".yellow());
        return true;
    }

    mod_files.sort();

    let hash_file = cache_dir.join(HASH_FILE);
    let cached_hash = if hash_file.exists() {
        fs::read_to_string(&hash_file)
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    if !cached_hash.is_empty() {
        println!("{}", "ℹ️ 发现缓存，尝试加载...".blue());

        if cached_hash == calc_hash(&mods_dir, &mod_files) {
            println!("{}", "ℹ️ 缓存可用，正在加载...".blue());

            if !apply_cache_files(&cache_dir, &pak_dir, &backup_dir) {
                return false;
            }

            println!("{}", "✅ 模组加载完成！".green());
            return true;
        } else {
            println!("{}", "ℹ️ 缓存无效，正在清除缓存并开始打包模组...".blue());

            if cache_dir.exists() {
                if let Err(e) = fs::remove_dir_all(&cache_dir) {
                    eprintln!("❌ 清空缓存失败: {}", e.to_string().red());
                }
            }
            if let Err(e) = fs::create_dir_all(&cache_dir) {
                eprintln!("❌ 创建缓存目录失败: {}", e.to_string().red());
            }
        }
    } else {
        println!("{}", "ℹ️ 缓存不存在，开始打包模组...".blue());
    }

    if !build_mods(&main_dir, &mod_files, &cached_hash, &config) {
        return false;
    }

    println!("{}", "ℹ️ 正在加载模组...".blue());
    if !apply_cache_files(&cache_dir, &pak_dir, &backup_dir) {
        return false;
    }
    println!(
        "{}",
        "✅ 模组加载完成！正在启动游戏！可以安全关闭此窗口".green()
    );
    true
}

/// 卸载模组：清理生成的模组 Pak 文件并从备份还原游戏原始文件
pub fn unload_mods(game_dir: &PathBuf, main_dir: &PathBuf) {
    let backup_dir = main_dir.join(BACKUP_DIR);
    let pak_dir = game_dir.join(PAK_PATH);
    let mod_pak = pak_dir.join(MOD_PAK);

    if !backup_dir.exists() || !backup_dir.is_dir() {
        eprintln!("❌ 游戏文件备份不存在！无法恢复文件！");
        return;
    }

    if !pak_dir.exists() || !pak_dir.is_dir() {
        eprintln!("❌ 游戏 PAK 目录不存在，无法恢复文件！");
        return;
    }

    let _ = fs::remove_file(&mod_pak);

    let entries: Vec<_> = WalkDir::new(&backup_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.file_name() != ".DS_Store")
        .collect();

    if entries.is_empty() {
        eprintln!("⚠️ 游戏文件备份不存在！跳过文件恢复");
        return;
    }

    let pb = ProgressBar::new(entries.len() as u64);
    set_pb_style(&pb, "恢复游戏文件");
    pb.set_message("ℹ️ 开始恢复游戏文件...".blue().to_string());

    for entry in entries {
        let src_path = entry.path();
        pb.set_message(format!(
            "🔄 {}",
            src_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .cyan()
        ));

        let rel_path = src_path.strip_prefix(&backup_dir).unwrap();
        let target_path = pak_dir.join(rel_path);

        if !target_path.exists() {
            pb_eprintln!(
                pb,
                "⚠️ 存在额外文件，跳过备份恢复: {}",
                target_path.display().to_string().red()
            );
            pb.inc(1);
            continue;
        }

        if let Err(e) = fs::remove_file(&target_path) {
            pb_eprintln!(
                pb,
                "❌ 删除文件 '{}' 失败: {}",
                target_path.display(),
                e.to_string().red()
            );
        }
        if let Err(e) = fs::rename(src_path, &target_path) {
            pb_eprintln!(
                pb,
                "❌ 移动文件 '{}' -> '{}' 失败: {}",
                src_path.display().to_string().red(),
                target_path.display().to_string().red(),
                e.to_string().red()
            );
        }

        pb.inc(1);
    }

    pb.finish_with_message("✅ 游戏文件恢复完成！".green().to_string());
    println!("{}", "✅ 游戏文件恢复完成！可以安全关闭此窗口".green());
}

/// 将缓存好的模组和修改文件应用替换到游戏目录中，同时建立备份
fn apply_cache_files(cache_dir: &PathBuf, pak_dir: &PathBuf, backup_dir: &PathBuf) -> bool {
    if !pak_dir.exists() {
        eprintln!("{}", "❌ 游戏 PAK 目录不存在，无法加载模组！".red());
        return false;
    }

    let cache_entries: Vec<PathBuf> = WalkDir::new(&cache_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.file_name() != ".DS_Store")
        .filter(|e| {
            let rel = e.path().strip_prefix(&cache_dir).unwrap();
            rel != Path::new(HASH_FILE)
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    let pb = ProgressBar::new(cache_entries.len() as u64);
    set_pb_style(&pb, "备份并替换游戏文件");
    pb.set_message("ℹ️ 开始备份并替换游戏文件...".blue().to_string());

    for src_path in &cache_entries {
        let rel_path = src_path.strip_prefix(&cache_dir).unwrap();
        let target_path = pak_dir.join(rel_path);
        let backup_path = backup_dir.join(rel_path);
        let is_mod_pak = rel_path == Path::new(MOD_PAK);
        pb.set_message(format!("🔄 {}", rel_path.to_string_lossy().cyan()));

        if !is_mod_pak && !target_path.exists() {
            pb_eprintln!(
                pb,
                "⚠️ 存在额外文件，跳过缓存加载: {}",
                target_path.display().to_string().red()
            );
            pb.inc(1);
            continue;
        }

        if !is_mod_pak {
            if let Err(e) = fs::rename(&target_path, &backup_path) {
                pb_eprintln!(
                    pb,
                    "❌ 移动文件 '{}' -> '{}' 失败: {}",
                    target_path.display().to_string().red(),
                    backup_path.display().to_string().red(),
                    e.to_string().red()
                );
                pb.inc(1);
                continue;
            }
        }

        if let Err(e) = fs::copy(&src_path, &target_path) {
            pb_eprintln!(
                pb,
                "❌ 复制文件 '{}' -> '{}' 失败: {}",
                src_path.display().to_string().red(),
                target_path.display().to_string().red(),
                e.to_string().red()
            );
        }

        pb.inc(1);
    }

    pb.finish_with_message("✅ 游戏文件替换完成！".green().to_string());
    true
}

/// 根据模组文件的相对路径和文件内容块计算唯一哈希值
fn calc_hash(mods_dir: &PathBuf, mod_files: &Vec<PathBuf>) -> String {
    let pb = ProgressBar::new(mod_files.len() as u64);
    set_pb_style(&pb, "计算模组哈希");
    pb.set_message("🔍 计算模组哈希中...".blue().to_string());

    let mut hasher = DefaultHasher::new();
    for file in mod_files {
        // 哈希相对路径结构
        let rel = file.strip_prefix(&mods_dir).unwrap().to_string_lossy();
        pb.set_message(format!("🔍 {}", rel.cyan()));
        rel.as_bytes().hash(&mut hasher);

        // 分块读取并哈希文件内容
        let file = fs::File::open(file);
        if let Err(e) = file {
            pb_eprintln!(pb, "❌ 打开模组文件失败: {}", e.to_string().red());
            continue;
        }
        let mut reader = BufReader::new(file.unwrap());
        let mut buffer = [0; 65536];
        loop {
            let bytes_read = reader.read(&mut buffer);
            if let Err(e) = bytes_read {
                pb_eprintln!(pb, "❌ 读取模组文件失败: {}", e.to_string().red());
                continue;
            }
            let bytes_read = bytes_read.unwrap();
            if bytes_read == 0 {
                break;
            }
            hasher.write(&buffer[..bytes_read]);
        }

        pb.inc(1);
    }
    pb.finish_with_message("✅ 模组哈希计算完成！".green().to_string());

    format!("{:016x}", hasher.finish())
}

/// 核心模组构建函数：解包冲突目标、执行 MXML 深度合并并生成最终模组 PAK
fn build_mods(
    main_dir: &PathBuf,
    mod_files: &Vec<PathBuf>,
    cached_hash: &str,
    config: &Config,
) -> bool {
    let pak_dir = &config.game_dir.join(PAK_PATH);
    let cache_dir = main_dir.join(CACHE_DIR);
    let mods_dir = main_dir.join(MODS_DIR);

    if !pak_dir.exists() {
        eprintln!("{}", "❌ 游戏 PAK 目录不存在，无法打包模组！".red());
        return false;
    }

    let temp_dir = TempDir::new();
    if let Err(e) = temp_dir {
        eprintln!("❌ 创建临时目录失败: {}", e.to_string().red());
        return false;
    }
    let temp_dir = temp_dir.unwrap();

    let (modification_map, files_to_pack) = scan_modifications(&mods_dir, mod_files);
    let mut mxml_files = HashMap::new();
    let mut conflicted = false;
    for (rel_path, files) in &modification_map {
        let is_mxml = rel_path.contains(".mxml");
        if files.len() > 1 && !is_mxml {
            if !conflicted {
                conflicted = true;
                eprintln!("{}", "❌ 存在模组冲突！".red());
            }
            eprintln!("  - 文件: {}", rel_path.red());
            eprintln!("    冲突模组: ");
            eprintln!(
                "{}",
                files
                    .iter()
                    .map(|n| format!("      - {}", n.0.yellow()))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        if is_mxml {
            mxml_files.insert(rel_path.clone(), files.clone());
        }
    }
    if conflicted {
        return false;
    }

    let mut normal_files_to_pack: Vec<_> = files_to_pack
        .iter()
        .filter(|(_, rel)| !mxml_files.contains_key(&rel.to_lowercase().replace(".exml", ".mxml")))
        .map(|f| f.clone())
        .collect();

    if cache_dir.exists() {
        if let Err(e) = fs::remove_dir_all(&cache_dir) {
            eprintln!("❌ 清空缓存目录失败: {}", e.to_string().red());
            return false;
        }
    }
    if let Err(e) = fs::create_dir_all(&cache_dir) {
        eprintln!("❌ 创建备份目录失败: {}", e.to_string().red());
        return false;
    }

    if let Err(e) = fs::create_dir_all(&cache_dir) {
        eprintln!("❌ 创建缓存目录失败: {}", e.to_string().red());
        return false;
    }

    println!("{}", "ℹ️ 正在修改游戏文件...".blue());
    if !repack_game_files(
        &pak_dir,
        &cache_dir,
        modification_map.keys().collect::<Vec<_>>(),
        config.pack_batch_chunk_count,
        &mxml_files,
        &temp_dir.path().to_path_buf(),
    ) {
        return false;
    }

    println!("{}", "ℹ️ 正在合并 MXML 文件...".blue());
    let pb = ProgressBar::new(mxml_files.len() as u64);
    set_pb_style(&pb, "合并 MXML 文件");
    pb.set_message("ℹ️ 开始合并 MXML 文件...".blue().to_string());
    let mut conflicts = Vec::new();
    for (rel_path, files) in &mxml_files {
        pb.set_message(format!("🧩 {} - 反序列化原文件", rel_path.cyan()));
        let mbin_path = temp_dir.path().join(&rel_path.replace(".mxml", ".mbin"));
        let mxml_path = temp_dir.path().join(rel_path);

        let parent = mbin_path.parent();
        if parent.is_none() {
            eprintln!(
                "❌ 无法获取文件所在目录: {}",
                mbin_path.display().to_string().red()
            );
            pb.inc(1);
            continue;
        }
        let parent = parent.unwrap();

        let mut base_content = None;
        if mbin_path.exists() {
            if let Err(e) = deserialize_mbin(
                &[&*mbin_path.to_string_lossy().to_string()],
                &*parent.to_string_lossy().to_string(),
            ) {
                eprintln!(
                    "❌ 反序列化 MBIN 文件 '{}' 失败: {}",
                    rel_path.red(),
                    e.to_string().red()
                );
                pb.inc(1);
                continue;
            }
            base_content = match fs::read_to_string(&mxml_path) {
                Ok(content) => Some(content),
                Err(_) => None,
            };
        }
        pb.set_message(format!("🧩 {} - 读取模组文件", rel_path.cyan()));
        let extra_content = files
            .iter()
            .map(|(n, p)| match fs::read_to_string(p) {
                Err(e) => {
                    eprintln!("❌ 读取模组文件失败: {}", e.to_string().red());
                    None
                }
                Ok(content) => Some((content, n.clone())),
            })
            .filter(|c| c.is_some())
            .map(|c| c.unwrap())
            .collect::<Vec<_>>();
        pb.set_message(format!("🧩 {} - 合并内容", rel_path.cyan()));
        let merged = merge_mxml(base_content, extra_content);
        if let Err(e) = merged {
            eprintln!(
                "❌ 合并 MXML 文件 '{}' 失败: {}",
                rel_path.red(),
                e.to_string().red()
            );
            pb.inc(1);
            continue;
        }
        pb.set_message(format!("🧩 {} - 序列化合并内容", rel_path.cyan()));
        let (merged_content, conflict) = merged.unwrap();
        if conflict.is_empty() && conflicts.is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("❌ 创建目录失败: {}", e.to_string().red());
                pb.inc(1);
                continue;
            }
            if let Err(e) = fs::write(&mxml_path, &merged_content) {
                eprintln!("❌ 写入 MXML 文件失败: {}", e.to_string().red());
                pb.inc(1);
                continue;
            }
            if let Err(e) = serialize_mxml(
                &[&*mxml_path.to_string_lossy().to_string()],
                &*parent.to_string_lossy().to_string(),
            ) {
                eprintln!("❌ 序列化 MXML 文件失败: {}", e.to_string().red());
                pb.inc(1);
                continue;
            }
            normal_files_to_pack.push((mbin_path, rel_path.replace(".mxml", ".mbin")));
        } else if !conflict.is_empty() {
            conflicts.push((rel_path, conflict));
        }
        pb.inc(1);
    }
    if !conflicts.is_empty() {
        pb.finish_with_message("❌ 存在冲突！".red().to_string());
        eprintln!("{}", "❌ 存在模组 MXML 文件冲突！".red());
        for (rel_path, conflict) in conflicts {
            eprintln!("  - 文件: {}", rel_path.red());
            for (node, names) in conflict {
                eprintln!("      - 节点: {}", node.yellow());
                eprintln!("        冲突模组: ");
                eprintln!(
                    "{}",
                    names
                        .iter()
                        .map(|n| format!("          - {}", n.bright_yellow()))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
        return false;
    }
    pb.finish_with_message("✅ 合并完成！".green().to_string());

    println!("{}", "ℹ️ 正在打包模组文件...".blue());
    let pb = ProgressBar::new(normal_files_to_pack.len() as u64);
    set_pb_style(&pb, "打包模组文件");
    pb.set_message("ℹ️ 开始打包...".blue().to_string());

    if let Err(e) = HGPAKFile::repack(
        normal_files_to_pack,
        &*cache_dir.join(MOD_PAK),
        true,
        Platform::Mac,
        Some(config.pack_batch_chunk_count),
        Some(&mut |file, count, total| {
            pb.set_position(count as u64);
            pb.set_length(total as u64);
            pb.set_message(format!("📦 {}", file.cyan()));
        }),
    ) {
        pb_eprintln!(pb, "❌ 打包模组 PAK 文件失败: {}", e.to_string().red());
        pb.finish_with_message("❌ 打包失败！".red().to_string());
        return false;
    }
    pb.finish_with_message("✅ 打包完成！".green().to_string());

    let hash_path = cache_dir.join(HASH_FILE);
    let mut hash = cached_hash.to_string();
    if hash.is_empty() {
        hash = calc_hash(&mods_dir, mod_files);
    }
    if let Err(e) = fs::write(&hash_path, &hash) {
        eprintln!("❌ 写入缓存 Hash 失败: {}", e.to_string().red());
        return false;
    }

    println!("{}", "✅ 模组打包完成！".green());
    true
}

/// 解析模组文件的顶层文件夹名称（模组名）和相对文件路径
fn get_name_and_rel_path(mods_dir: &Path, mod_file: &Path) -> Option<(String, String)> {
    if let Ok(rel) = mod_file.strip_prefix(mods_dir) {
        let mut components = rel.components();
        let name = components.next();
        if name.is_none() {
            return None;
        }
        let rel_path: PathBuf = components.collect();
        Some((
            name.unwrap().as_os_str().to_string_lossy().to_string(),
            rel_path.to_string_lossy().to_string(),
        ))
    } else {
        None
    }
}

/// 遍历扫描当前加载的所有模组文件，并映射它们所修改的目标游戏文件路径
fn scan_modifications(
    mods_dir: &PathBuf,
    mod_files: &Vec<PathBuf>,
) -> (
    HashMap<String, Vec<(String, PathBuf)>>,
    Vec<(PathBuf, String)>,
) {
    let mut modification_map = HashMap::new();
    let mut files_to_pack = Vec::new();

    for path in mod_files {
        let res = get_name_and_rel_path(&mods_dir, path);
        if res.is_none() {
            eprintln!("❌ 解析模组文件路径失败: {}", path.display());
            continue;
        }
        let (name, rel_path) = res.unwrap();
        modification_map
            .entry(rel_path.to_lowercase().replace(".exml", ".mxml"))
            .or_insert(Vec::new())
            .push((name, path.clone()));
        files_to_pack.push((path.clone(), rel_path.to_lowercase()));
    }
    (modification_map, files_to_pack)
}

/// 扫描并定位受模组影响的原始游戏 PAK 文件，提取出需要合并或替换的目标基础资源
fn repack_game_files(
    pak_dir: &PathBuf,
    cache_dir: &PathBuf,
    modified_paths: Vec<&String>,
    batch_chunks: usize,
    mxml_files: &HashMap<String, Vec<(String, PathBuf)>>,
    mxml_dir: &PathBuf,
) -> bool {
    let pak_files: Vec<_> = WalkDir::new(pak_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".pak"))
        .map(|e| e.path().to_path_buf())
        .collect();

    let mut edited = Vec::new();

    let modified_paths = &modified_paths
        .iter()
        .map(|s| s.to_lowercase())
        .collect::<Vec<_>>();
    let mxml_files = &mxml_files.iter().map(|(k, _)| k).collect::<Vec<_>>();

    let multi = MultiProgress::new();
    let pb = multi.add(ProgressBar::new(pak_files.len() as u64));
    set_pb_style(&pb, "修改游戏文件");
    pb.set_message("ℹ️ 开始修改游戏文件...".blue().to_string());

    for pak_file in pak_files {
        let filename = pak_file.file_name();
        if filename.is_none() {
            pb_eprintln!(multi, "❌ 无法获取 PAK 文件名: {}", pak_file.display());
            continue;
        }
        let filename = filename.unwrap();

        pb.set_message(format!("🔧 {}", filename.to_string_lossy().cyan()));

        let hgpak = HGPAKFile::open(&pak_file, Platform::Mac, Some(batch_chunks));
        if let Err(e) = hgpak {
            pb_eprintln!(multi, "❌ 打开 PAK 文件失败: {}", e.to_string().red());
            continue;
        }
        let hgpak = hgpak.unwrap();

        let files = hgpak.files.keys().collect::<Vec<_>>();
        let mut modified = false;
        for &file in &files {
            if modified_paths.contains(&file)
                || mxml_files.contains(&&file.replace(".mbin", ".mxml"))
            {
                modified = true;
                break;
            }
        }
        if !modified {
            continue;
        }

        let temp_dir = TempDir::new();
        if let Err(e) = temp_dir {
            pb_eprintln!(multi, "❌ 创建临时目录失败: {}", e.to_string().red());
            continue;
        }
        let temp_dir = temp_dir.unwrap();

        let pb2 = multi.add(ProgressBar::no_length());
        set_pb_style(&pb2, "解包游戏文件");
        pb2.set_message("ℹ️ 开始解包游戏文件...".blue().to_string());

        if let Err(e) = hgpak.unpack(
            &temp_dir.path(),
            Some(&mut |file, count, total| {
                pb2.set_position(count as u64);
                pb2.set_length(total as u64);
                pb2.set_message(format!("📂 {}", file.cyan()));
            }),
        ) {
            pb_eprintln!(multi, "❌ 解包游戏 PAK 文件失败: {}", e.to_string().red());
            pb2.finish_with_message("❌ 游戏文件解包失败！".red().to_string());
            continue;
        }

        pb2.finish_with_message("✅ 解包完成！".green().to_string());

        for rel_path in mxml_files {
            let mbin_path = rel_path.replace(".mxml", ".mbin");
            let temp_dir_path = &temp_dir.path().join(&mbin_path);
            if !temp_dir_path.exists() {
                continue;
            }
            let mxml_dir_path = &mxml_dir.join(&mbin_path);
            let parent = mxml_dir_path.parent();
            if parent.is_none() {
                pb_eprintln!(
                    multi,
                    "❌ 无法获取文件所在目录: {}",
                    mxml_dir_path.display().to_string().red()
                );
                continue;
            }
            let parent = parent.unwrap();
            if let Err(e) = fs::create_dir_all(parent) {
                pb_eprintln!(multi, "❌ 创建目录失败: {}", e.to_string().red());
                continue;
            }
            if let Err(e) = fs::rename(&temp_dir_path, &mxml_dir_path) {
                pb_eprintln!(
                    multi,
                    "❌ 移动文件 '{}' -> '{}' 失败: {}",
                    temp_dir_path.display().to_string().red(),
                    mxml_dir_path.display().to_string().red(),
                    e.to_string().red()
                );
                continue;
            }
        }

        for modified_path in modified_paths {
            let file_path = temp_dir.path().join(modified_path);
            if !file_path.exists() {
                continue;
            }
            if let Err(e) = fs::remove_file(&file_path) {
                pb_eprintln!(
                    multi,
                    "❌ 删除文件 '{}' 失败: {}",
                    file_path.display().to_string().red(),
                    e.to_string().red()
                );
                continue;
            }
        }

        let pb2 = multi.add(ProgressBar::no_length());
        set_pb_style(&pb2, "打包游戏文件");
        pb2.set_message("ℹ️ 开始打包游戏文件...".blue().to_string());

        if let Err(e) = HGPAKFile::repack(
            vec![(temp_dir.path().to_path_buf(), "".to_string())],
            &*cache_dir.join(filename),
            true,
            Platform::Mac,
            Some(batch_chunks),
            Some(&mut |file, count, total| {
                pb2.set_position(count as u64);
                pb2.set_length(total as u64);
                pb2.set_message(format!("📦 {}", file.cyan()));
            }),
        ) {
            pb_eprintln!(multi, "❌ 打包游戏 PAK 文件失败: {}", e.to_string().red());
            pb2.finish_with_message("❌ 游戏文件打包失败！".red().to_string());
            continue;
        }
        pb2.finish_with_message("✅ 游戏文件打包完成！".green().to_string());

        edited.push(filename.to_string_lossy().to_string());
    }
    pb.finish();
    println!("{}", "✅ 游戏文件修改完成！".green());
    if edited.is_empty() {
        println!("{}", "  未修改任何文件".green());
    } else {
        println!(
            "{}",
            format!("  共修改 {} 个文件：", edited.len())
                .to_string()
                .green()
        );
        println!(
            "{}",
            edited
                .iter()
                .map(|s| format!("    - {}", s.green()))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    true
}
