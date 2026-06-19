use crate::constants::{
    EXEC_FILE, LAUNCHER_BIN, LAUNCHER_FILE, PAK_PATH, PLIST_FILE, STARTUP_EXEC_PATH,
};
use crate::mbin::MbinError;
use plist::Value;
use std::env::home_dir;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// 游戏目录检测与注入相关错误
#[derive(Error, Debug)]
pub enum GameError {
    #[error("无法检测游戏目录，请手动配置")]
    DirDetectionError(),

    #[error("游戏目录无效: {0}")]
    InvalidGameDir(String),

    #[error("无法修改 Info.plist: {0}")]
    PlistModifyError(String),

    #[error("无法创建启动程序: {0}")]
    ExecCreationError(String),
}

/// 自动检测本地游戏安装目录
pub fn detect_game_dir() -> Result<PathBuf, GameError> {
    let home_dir = home_dir().unwrap();
    let default_game_dirs: [PathBuf; 3] = [
        PathBuf::from("/Applications/No Man's Sky.app"),
        home_dir.join("/Applications/No Man's Sky.app"),
        home_dir.join(
            "Library/Application Support/Steam/steamapps/common/No Man's Sky/No Man's Sky.app",
        ),
    ];
    for dir in default_game_dirs {
        if is_game_dir_valid(&dir).is_ok() {
            return Ok(dir);
        }
    }
    Err(GameError::DirDetectionError())
}

/// 验证游戏目录结构是否完整有效
pub fn is_game_dir_valid(game_dir: &PathBuf) -> Result<(), GameError> {
    if game_dir.join(PAK_PATH).exists() {
        Ok(())
    } else {
        Err(GameError::InvalidGameDir(game_dir.display().to_string()))
    }
}

/// 执行游戏启动注入流程（写入代理启动程序并修改 Info.plist）
pub fn inject_game(game_dir: &Path) -> Result<(), GameError> {
    let launcher_path = game_dir.join(STARTUP_EXEC_PATH).join(LAUNCHER_FILE);
    if launcher_path.parent().is_none() || !launcher_path.parent().unwrap().exists() {
        return Err(GameError::ExecCreationError(format!(
            "启动目录不存在: {}",
            launcher_path.parent().unwrap().display()
        )));
    }

    let mut launcher_file = File::create(&launcher_path)
        .map_err(|e| GameError::ExecCreationError(format!("无法创建启动程序文件: {}", e)))?;
    launcher_file
        .write_all(LAUNCHER_BIN)
        .map_err(|e| GameError::ExecCreationError(format!("写入启动程序失败: {}", e)))?;

    fs::set_permissions(&launcher_path, fs::Permissions::from_mode(0o755))
        .map_err(|e| MbinError::InitError(format!("{e}")))
        .map_err(|e| GameError::ExecCreationError(format!("设置启动程序权限失败: {}", e)))?;

    let plist_path = game_dir.join(PLIST_FILE);
    if !plist_path.exists() {
        return Err(GameError::InvalidGameDir(format!(
            "未找到 Info.plist: {}",
            plist_path.display()
        )));
    }

    let mut plist_value = Value::from_file(&plist_path)
        .map_err(|e| GameError::PlistModifyError(format!("读取 Plist 文件失败: {}", e)))?;
    let dict = plist_value
        .as_dictionary_mut()
        .ok_or_else(|| GameError::PlistModifyError("Plist 根节点不是字典结构".to_string()))?;

    dict.insert(
        "CFBundleExecutable".to_string(),
        Value::String(LAUNCHER_FILE.to_string()),
    );

    let file = File::create(&plist_path)
        .map_err(|e| GameError::PlistModifyError(format!("无法创建 Plist 文件: {}", e)))?;
    plist::to_writer_xml(file, &plist_value)
        .map_err(|e| GameError::PlistModifyError(format!("写入 Plist 文件失败: {}", e)))?;
    correct_plist_file(&plist_path)?;

    println!("✅ 游戏注入成功！");
    Ok(())
}

/// 移除游戏内的代理注入，还原游戏原始启动配置
pub fn remove_game_injection(game_dir: &Path) -> Result<(), GameError> {
    let launcher_path = game_dir.join(STARTUP_EXEC_PATH).join(LAUNCHER_FILE);
    let _ = fs::remove_file(&launcher_path);

    let plist_path = game_dir.join(PLIST_FILE);
    if !plist_path.exists() {
        return Err(GameError::InvalidGameDir(format!(
            "未找到 Info.plist: {}",
            plist_path.display()
        )));
    }

    let mut plist_value = Value::from_file(&plist_path)
        .map_err(|e| GameError::PlistModifyError(format!("读取 Plist 文件失败: {}", e)))?;
    let dict = plist_value
        .as_dictionary_mut()
        .ok_or_else(|| GameError::PlistModifyError("Plist 文件根节点不是字典结构".to_string()))?;

    dict.insert(
        "CFBundleExecutable".to_string(),
        Value::String(EXEC_FILE.to_string()),
    );

    let file = File::create(&plist_path)
        .map_err(|e| GameError::PlistModifyError(format!("无法创建 Plist 文件: {}", e)))?;
    plist::to_writer_xml(file, &plist_value)
        .map_err(|e| GameError::PlistModifyError(format!("写入 Plist 文件失败: {}", e)))?;
    correct_plist_file(&plist_path)?;

    println!("✅ 注入移除成功！");
    Ok(())
}

/// 修正 plist 序列化的问题
fn correct_plist_file(path: &Path) -> Result<(), GameError> {
    let content = fs::read_to_string(path)
        .map_err(|e| GameError::PlistModifyError(format!("读取 plist 文件失败: {}", e)))?;
    let new_content = content.replace("&apos;", "'") + "\n";
    fs::write(path, new_content)
        .map_err(|e| GameError::PlistModifyError(format!("写入 plist 文件失败: {}", e)))?;
    Ok(())
}
